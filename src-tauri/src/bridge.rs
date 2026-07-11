use crate::file_ops::AppState;
use tauri::{State, WebviewWindow};

#[tauri::command]
pub fn exec_command(cmd: String, param: Option<String>) -> Result<String, String> {
    Ok(format!("cmd:{} param:{:?}", cmd, param))
}

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
pub fn load_font(_name: String) -> Result<Option<String>, String> {
    Ok(None)
}

#[tauri::command]
pub fn list_media_dir(state: tauri::State<'_, AppState>) -> String {
    let media_dir = state.temp_dir.join("media");
    match std::fs::read_dir(&media_dir) {
        Ok(entries) => entries.flatten()
            .map(|e| {
                let sz = std::fs::metadata(e.path()).map(|m| m.len()).unwrap_or(0);
                format!("{}({})", e.file_name().to_string_lossy(), sz)
            })
            .collect::<Vec<_>>().join(", "),
        Err(_) => "media/ not found".to_string(),
    }
}

#[tauri::command]
pub fn js_log(msg: String, state: tauri::State<'_, AppState>) {
    println!("[JS] {}", msg);
    use std::io::Write;
    let log_path = state.temp_dir.join("js-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(f, "{}", msg);
    }
}
