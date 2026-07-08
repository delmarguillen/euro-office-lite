use crate::file_ops::AppState;
use tauri::State;

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
            eprintln!("[clipboard] Failed to open clipboard: {}", e);
            return Ok(None);
        }
    };

    let img = match clipboard.get_image() {
        Ok(img) => img,
        Err(_) => return read_clipboard_file_image(&mut clipboard, &state),
    };

    let media_dir = state.temp_dir.join("media");
    if let Err(e) = std::fs::create_dir_all(&media_dir) {
        eprintln!("[clipboard] Failed to create media dir: {}", e);
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
                eprintln!("[clipboard] Failed to create image buffer");
                return Ok(None);
            }
        };

    if let Err(e) = image_buf.save_with_format(&path, image::ImageFormat::Png) {
        eprintln!("[clipboard] Failed to save PNG: {}", e);
        return Ok(None);
    }

    eprintln!("[clipboard] Saved clipboard image to {:?}", path);
    Ok(Some(filename))
}

fn read_clipboard_file_image(
    clipboard: &mut arboard::Clipboard,
    state: &State<'_, AppState>,
) -> Result<Option<String>, String> {
    let files = match clipboard.get().file_list() {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };

    const IMAGE_EXTS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "ico"];
    let src = files.into_iter().find(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| IMAGE_EXTS.contains(&e.to_lowercase().as_str()))
            .unwrap_or(false)
    });
    let src = match src {
        Some(p) => p,
        None => return Ok(None),
    };

    let media_dir = state.temp_dir.join("media");
    if let Err(e) = std::fs::create_dir_all(&media_dir) {
        eprintln!("[clipboard] Failed to create media dir: {}", e);
        return Ok(None);
    }

    let ext = src.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_else(|| "png".into());
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let dest_name = format!("clipboard_file_{}.{}", timestamp, ext);
    let dest = media_dir.join(&dest_name);

    if let Err(e) = std::fs::copy(&src, &dest) {
        eprintln!("[clipboard] Failed to copy clipboard file image: {}", e);
        return Ok(None);
    }

    eprintln!("[clipboard] Copied clipboard file-reference image to {:?}", dest);
    Ok(Some(dest_name))
}
