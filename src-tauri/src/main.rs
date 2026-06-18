#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod converter;
mod file_ops;

use file_ops::AppState;
use std::sync::Mutex;

fn main() {
    let temp_dir = std::env::temp_dir().join("euro-office-lite");
    std::fs::create_dir_all(&temp_dir).ok();

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
            file_ops::create_new,
            file_ops::get_current_path,
            bridge::exec_command,
            bridge::set_window_title,
            bridge::set_document_modified,
            bridge::load_font,
            converter::convert_file,
        ])
        .run(tauri::generate_context!())
        .expect("error running Euro-Office Lite");
}
