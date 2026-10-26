use crate::file_ops::AppState;
use tauri::{State, WebviewWindow};

#[tauri::command]
pub fn set_window_title(window: WebviewWindow, name: String) -> Result<(), String> {
    window
        .set_title(&format!("Euro-Office Lite — {}", name))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_document_modified(
    window: WebviewWindow,
    state: State<'_, AppState>,
    modified: bool,
) -> Result<(), String> {
    *state.modified.lock().unwrap() = modified;
    let current = window.title().map_err(|e| e.to_string())?;
    let base = current.trim_start_matches("● ");
    let title = if modified {
        format!("● {}", base)
    } else {
        base.to_string()
    };
    window.set_title(&title).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn force_close(app_handle: tauri::AppHandle) -> Result<(), String> {
    app_handle.exit(0);
    Ok(())
}

#[tauri::command]
pub fn js_log(msg: String) {
    let msg = crate::protocol_limits::truncate_log(&msg, crate::protocol_limits::JS_LOG_MAX_BYTES);
    println!("[JS] {}", msg);
    crate::temp_layout::append_log(msg);
}
