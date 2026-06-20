#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod converter;
mod file_ops;

use file_ops::AppState;
use std::sync::Mutex;

fn main() {
    let temp_dir = std::env::temp_dir().join("euro-office-lite");
    std::fs::create_dir_all(&temp_dir).ok();

    {
        use std::io::Write;
        let log_path = temp_dir.join("js-debug.log");
        if let Ok(mut f) = std::fs::File::create(&log_path) {
            let _ = writeln!(f, "[RUST] App started at {:?}", std::time::SystemTime::now());
            let _ = writeln!(f, "[RUST] Log path: {:?}", log_path);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            current_file: Mutex::new(None),
            temp_dir,
            modified: Mutex::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            file_ops::open_file,
            file_ops::save_file,
            file_ops::save_file_as,
            file_ops::save_changes,
            file_ops::write_editor_bin,
            file_ops::create_new,
            file_ops::get_current_path,
            bridge::exec_command,
            bridge::set_window_title,
            bridge::set_document_modified,
            bridge::load_font,
            bridge::js_log,
        ])
        .register_uri_scheme_protocol("ascdesktop", |_ctx, request| {
            let uri = request.uri().to_string();
            let path = uri
                .strip_prefix("ascdesktop://")
                .or_else(|| uri.strip_prefix("ascdesktop:///"))
                .unwrap_or("");

            let src_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src");
            let file_path = src_dir.join(path);

            match std::fs::read(&file_path) {
                Ok(data) => {
                    let mime = if path.ends_with(".ttf") || path.ends_with(".otf") {
                        "font/ttf"
                    } else if path.ends_with(".js") {
                        "application/javascript"
                    } else if path.ends_with(".png") {
                        "image/png"
                    } else {
                        "application/octet-stream"
                    };
                    tauri::http::Response::builder()
                        .status(200)
                        .header("Content-Type", mime)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(data)
                        .unwrap()
                }
                Err(_) => tauri::http::Response::builder()
                    .status(404)
                    .body(b"Not Found".to_vec())
                    .unwrap(),
            }
        })
        .run(tauri::generate_context!())
        .expect("error running Euro-Office Lite");
}
