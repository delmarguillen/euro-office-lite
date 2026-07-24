use crate::file_ops::AppState;
use tauri::State;

fn clipboard_log(state: &State<'_, AppState>, msg: &str) {
    eprintln!("{}", msg);
    let log_path = state.temp_dir.join("js-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        use std::io::Write;
        let _ = writeln!(f, "{}", msg);
    }
}

#[tauri::command]
pub fn read_clipboard_text() -> Result<Option<String>, String> {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[clipboard] Failed to open clipboard: {}", e);
            return Ok(None);
        }
    };

    match clipboard.get_text() {
        Ok(text) => Ok(Some(text)),
        Err(_) => Ok(None),
    }
}

#[tauri::command]
pub fn read_clipboard_image(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            clipboard_log(&state, &format!("[clipboard] Failed to open clipboard: {}", e));
            return Ok(None);
        }
    };

    // Try file_list first: on macOS, get_image() returns the file's icon bitmap,
    // not the actual image. file_list gives us the real file path.
    match read_clipboard_file_image(&mut clipboard, &state) {
        Ok(Some(f)) => return Ok(Some(f)),
        Ok(None) => clipboard_log(&state, "[clipboard] file_list returned None, trying get_image"),
        Err(e) => clipboard_log(&state, &format!("[clipboard] file_list error: {}, trying get_image", e)),
    }

    let img = match clipboard.get_image() {
        Ok(img) => {
            clipboard_log(&state, &format!("[clipboard] get_image OK: {}x{}", img.width, img.height));
            img
        }
        Err(e) => {
            clipboard_log(&state, &format!("[clipboard] get_image failed: {}", e));
            return Ok(None);
        }
    };

    let media_dir = state.temp_dir.join("media");
    if let Err(e) = std::fs::create_dir_all(&media_dir) {
        clipboard_log(&state, &format!("[clipboard] Failed to create media dir: {}", e));
        return Ok(None);
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let filename = format!("clipboard_{}.png", timestamp);
    let path = media_dir.join(&filename);

    let image_buf: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        match image::ImageBuffer::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) {
            Some(buf) => buf,
            None => {
                clipboard_log(&state, "[clipboard] Failed to create image buffer");
                return Ok(None);
            }
        };

    if let Err(e) = image_buf.save_with_format(&path, image::ImageFormat::Png) {
        clipboard_log(&state, &format!("[clipboard] Failed to save PNG: {}", e));
        return Ok(None);
    }

    clipboard_log(&state, &format!("[clipboard] Saved clipboard image to {:?}", path));
    Ok(Some(filename))
}

fn read_clipboard_file_image(
    clipboard: &mut arboard::Clipboard,
    state: &State<'_, AppState>,
) -> Result<Option<String>, String> {
    let files = match clipboard.get().file_list() {
        Ok(f) => {
            let paths: Vec<String> = f.iter().map(|p| format!("{:?}", p)).collect();
            clipboard_log(&state, &format!("[clipboard] file_list OK: {} files: [{}]", f.len(), paths.join(", ")));
            f
        }
        Err(e) => {
            clipboard_log(&state, &format!("[clipboard] file_list failed: {}", e));
            return Ok(None);
        }
    };

    // text/uri-list uses CRLF line endings; arboard's X11 backend keeps the
    // trailing \r in each parsed path, which breaks the extension match and
    // every later fs call. Upstream bug 1Password/arboard#216; the fix (PR
    // #217) is merged but unreleased as of arboard 3.6.1, so trim here.
    // Harmless once upstream ships: trimming an already-clean path is a no-op.
    let files: Vec<std::path::PathBuf> = files
        .into_iter()
        .map(|p| match p.to_str() {
            Some(s) => std::path::PathBuf::from(s.trim_end_matches(&['\r', '\n'][..])),
            None => p,
        })
        .collect();

    const IMAGE_EXTS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "ico"];
    let src = files.into_iter().find(|p| {
        let ext_match = p.extension()
            .and_then(|e| e.to_str())
            .map(|e| IMAGE_EXTS.contains(&e.to_lowercase().as_str()))
            .unwrap_or(false);
        clipboard_log(&state, &format!("[clipboard] checking {:?} -> ext_match={}", p, ext_match));
        ext_match
    });
    let src = match src {
        Some(p) => {
            let exists = p.exists();
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            clipboard_log(&state, &format!("[clipboard] selected source: {:?} (exists={}, size={})", p, exists, size));
            p
        }
        None => {
            clipboard_log(&state, "[clipboard] no image file found in file_list");
            return Ok(None);
        }
    };

    let media_dir = state.temp_dir.join("media");
    clipboard_log(&state, &format!("[clipboard] media_dir: {:?}", media_dir));
    if let Err(e) = std::fs::create_dir_all(&media_dir) {
        clipboard_log(&state, &format!("[clipboard] Failed to create media dir: {}", e));
        return Ok(None);
    }

    let ext = src.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_else(|| "png".into());
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let dest_name = format!("clipboard_file_{}.{}", timestamp, ext);
    let dest = media_dir.join(&dest_name);

    clipboard_log(&state, &format!("[clipboard] copying {:?} -> {:?}", src, dest));
    match std::fs::copy(&src, &dest) {
        Ok(bytes) => {
            clipboard_log(&state, &format!("[clipboard] copy OK: {} bytes written", bytes));
        }
        Err(e) => {
            clipboard_log(&state, &format!("[clipboard] copy FAILED: {}", e));
            return Ok(None);
        }
    }

    let dest_exists = dest.exists();
    let dest_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    clipboard_log(&state, &format!("[clipboard] result: dest_name={}, dest_exists={}, dest_size={}", dest_name, dest_exists, dest_size));
    Ok(Some(dest_name))
}
