use crate::file_ops::AppState;
use std::path::{Path, PathBuf};
use tauri::State;

fn clipboard_log(msg: &str) {
    eprintln!("{}", msg);
    crate::temp_layout::append_log(msg);
}

// Both clipboard reads are `async` commands whose blocking work runs on
// `spawn_blocking`. A plain sync `#[tauri::command]` is executed inline on the
// IPC handler, which on Linux is the GTK main loop: while arboard waits for the
// X11 selection (up to 4s per read, and it cannot be answered at all when our
// own webview owns it - journal 030) the whole UI stops repainting. That is the
// ~15-25s freeze of Issue #37.
#[tauri::command]
pub async fn read_clipboard_text() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(read_clipboard_text_blocking)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[clipboard] read_clipboard_text task failed: {}", e);
            Ok(None)
        })
}

fn read_clipboard_text_blocking() -> Result<Option<String>, String> {
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

// The HTML flavor, for copies made inside the editor that carry no text or
// image: sdkjs keeps its binary in a class of the copied HTML. Same blocking
// pattern as read_clipboard_text.
#[tauri::command]
pub async fn read_clipboard_html() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(read_clipboard_html_blocking)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[clipboard] read_clipboard_html task failed: {}", e);
            Ok(None)
        })
}

fn read_clipboard_html_blocking() -> Result<Option<String>, String> {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            clipboard_log(&format!("[clipboard] Failed to open clipboard: {}", e));
            return Ok(None);
        }
    };

    // The clipboard is user data: log only its size and whether it is an
    // editor copy, never its content.
    match clipboard.get().html() {
        Ok(html) if !html.is_empty() => {
            clipboard_log(&format!(
                "[clipboard] html: {} bytes, sdkjs_binary={}",
                html.len(),
                has_sdkjs_binary_tag(&html)
            ));
            Ok(Some(html))
        }
        Ok(_) => {
            clipboard_log("[clipboard] html: empty");
            Ok(None)
        }
        Err(e) => {
            clipboard_log(&format!("[clipboard] html: none ({})", e));
            Ok(None)
        }
    }
}

// Tags sdkjs writes before its binary in the copied HTML's class, per editor.
fn has_sdkjs_binary_tag(html: &str) -> bool {
    ["docData;", "xslData;", "pptData;", "pdfData;"]
        .iter()
        .any(|tag| html.contains(tag))
}

#[tauri::command]
pub async fn read_clipboard_image(state: State<'_, AppState>) -> Result<Option<String>, String> {
    // Clone the path out of the state: the blocking closure outlives the borrow.
    let temp_dir = state.temp_dir.clone();
    tauri::async_runtime::spawn_blocking(move || read_clipboard_image_blocking(&temp_dir))
        .await
        .unwrap_or_else(|e| {
            eprintln!("[clipboard] read_clipboard_image task failed: {}", e);
            Ok(None)
        })
}

fn read_clipboard_image_blocking(temp_dir: &Path) -> Result<Option<String>, String> {
    // Session type of this read, to tell Wayland from X11 in the log.
    clipboard_log(&format!(
        "[clipboard] read start (wayland_display={})",
        if std::env::var_os("WAYLAND_DISPLAY").is_some() { "yes" } else { "no" }
    ));
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            clipboard_log(&format!("[clipboard] Failed to open clipboard: {}", e));
            return Ok(None);
        }
    };

    // Try file_list first: on macOS, get_image() returns the file's icon bitmap,
    // not the actual image. file_list gives us the real file path.
    match read_clipboard_file_image(&mut clipboard, temp_dir) {
        Ok(Some(f)) => return Ok(Some(f)),
        Ok(None) => clipboard_log("[clipboard] file_list returned None, trying get_image"),
        Err(e) => clipboard_log(&format!("[clipboard] file_list error: {}, trying get_image", e)),
    }

    let img = match clipboard.get_image() {
        Ok(img) => {
            clipboard_log(&format!("[clipboard] get_image OK: {}x{}", img.width, img.height));
            img
        }
        Err(e) => {
            clipboard_log(&format!("[clipboard] get_image failed: {}", e));
            return Ok(None);
        }
    };

    let media_dir = temp_dir.join("media");
    if let Err(e) = std::fs::create_dir_all(&media_dir) {
        clipboard_log(&format!("[clipboard] Failed to create media dir: {}", e));
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
                clipboard_log("[clipboard] Failed to create image buffer");
                return Ok(None);
            }
        };

    if let Err(e) = image_buf.save_with_format(&path, image::ImageFormat::Png) {
        clipboard_log(&format!("[clipboard] Failed to save PNG: {}", e));
        return Ok(None);
    }

    clipboard_log(&format!("[clipboard] Saved clipboard image to {:?}", path));
    Ok(Some(filename))
}

fn read_clipboard_file_image(
    clipboard: &mut arboard::Clipboard,
    temp_dir: &Path,
) -> Result<Option<String>, String> {
    let files = match clipboard.get().file_list() {
        Ok(f) => {
            let paths: Vec<String> = f.iter().map(|p| format!("{:?}", p)).collect();
            clipboard_log(&format!("[clipboard] file_list OK: {} files: [{}]", f.len(), paths.join(", ")));
            f
        }
        Err(e) => {
            clipboard_log(&format!("[clipboard] file_list failed: {}", e));
            return Ok(None);
        }
    };

    // text/uri-list uses CRLF line endings; arboard's X11 backend keeps the
    // trailing \r in each parsed path, which breaks the extension match and
    // every later fs call. Upstream bug 1Password/arboard#216; the fix (PR
    // #217) is merged but unreleased as of arboard 3.6.1, so trim here.
    // Harmless once upstream ships: trimming an already-clean path is a no-op.
    let files: Vec<PathBuf> = files
        .into_iter()
        .map(|p| match p.to_str() {
            Some(s) => PathBuf::from(s.trim_end_matches(&['\r', '\n'][..])),
            None => p,
        })
        .collect();

    const IMAGE_EXTS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "ico"];
    let src = files.into_iter().find(|p| {
        let ext_match = p.extension()
            .and_then(|e| e.to_str())
            .map(|e| IMAGE_EXTS.contains(&e.to_lowercase().as_str()))
            .unwrap_or(false);
        clipboard_log(&format!("[clipboard] checking {:?} -> ext_match={}", p, ext_match));
        ext_match
    });
    let src = match src {
        Some(p) => {
            let exists = p.exists();
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            clipboard_log(&format!("[clipboard] selected source: {:?} (exists={}, size={})", p, exists, size));
            p
        }
        None => {
            clipboard_log("[clipboard] no image file found in file_list");
            return Ok(None);
        }
    };

    let media_dir = temp_dir.join("media");
    clipboard_log(&format!("[clipboard] media_dir: {:?}", media_dir));
    if let Err(e) = std::fs::create_dir_all(&media_dir) {
        clipboard_log(&format!("[clipboard] Failed to create media dir: {}", e));
        return Ok(None);
    }

    let ext = src.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_else(|| "png".into());
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let dest_name = format!("clipboard_file_{}.{}", timestamp, ext);
    let dest = media_dir.join(&dest_name);

    clipboard_log(&format!("[clipboard] copying {:?} -> {:?}", src, dest));
    match std::fs::copy(&src, &dest) {
        Ok(bytes) => {
            clipboard_log(&format!("[clipboard] copy OK: {} bytes written", bytes));
        }
        Err(e) => {
            clipboard_log(&format!("[clipboard] copy FAILED: {}", e));
            return Ok(None);
        }
    }

    let dest_exists = dest.exists();
    let dest_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    clipboard_log(&format!("[clipboard] result: dest_name={}, dest_exists={}, dest_size={}", dest_name, dest_exists, dest_size));
    Ok(Some(dest_name))
}

#[cfg(test)]
mod tests {
    use super::has_sdkjs_binary_tag;

    #[test]
    fn editor_copies_are_tagged() {
        assert!(has_sdkjs_binary_tag(r#"<img src="data:image/png;base64,AA==" class="pptData;20600;RE9D">"#));
        assert!(has_sdkjs_binary_tag(r#"<span class="xslData;7067;AAA">x</span>"#));
        assert!(has_sdkjs_binary_tag(r#"<p class="docData;1803;AAA">x</p>"#));
        assert!(has_sdkjs_binary_tag(r#"<div class="pdfData;10;AAA"></div>"#));
    }

    #[test]
    fn other_html_is_not() {
        assert!(!has_sdkjs_binary_tag(""));
        assert!(!has_sdkjs_binary_tag(r#"<img src="http://127.0.0.1:8765/img.png">"#));
        assert!(!has_sdkjs_binary_tag("<p>pptData without the separator</p>"));
    }
}
