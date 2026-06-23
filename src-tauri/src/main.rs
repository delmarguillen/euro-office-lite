#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod converter;
mod file_ops;

use file_ops::AppState;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

fn log_startup(temp_dir: &std::path::Path, msg: &str) {
    use std::io::Write;
    let log_path = temp_dir.join("js-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "[STARTUP] {}", msg);
    }
}

fn main() {
    let temp_dir = std::env::temp_dir().join("euro-office-lite");
    std::fs::create_dir_all(&temp_dir).ok();

    {
        use std::io::Write;
        let log_path = temp_dir.join("js-debug.log");
        if let Ok(mut f) = std::fs::File::create(&log_path) {
            let _ = writeln!(f, "[STARTUP] App started at {:?}", std::time::SystemTime::now());
            let _ = writeln!(f, "[STARTUP] Log path: {:?}", log_path);
        }
    }

    let file_to_open: Option<String> = {
        let args: Vec<String> = std::env::args().collect();
        log_startup(&temp_dir, &format!("Launch args: {:?}", args));
        if args.len() > 1 {
            let path = &args[1];
            if !path.starts_with('-') && std::path::Path::new(path).exists() {
                log_startup(&temp_dir, &format!("File association argument: {}", path));
                Some(path.clone())
            } else {
                None
            }
        } else {
            None
        }
    };

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init());

    #[cfg(target_os = "windows")]
    {
        builder = builder.plugin(tauri_plugin_printer_v2::init());
    }

    builder.manage(AppState {
            current_file: Mutex::new(None),
            temp_dir: temp_dir.clone(),
            modified: Mutex::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            file_ops::open_file,
            file_ops::save_file,
            file_ops::save_file_as,
            file_ops::save_changes,
            file_ops::write_editor_bin,
            file_ops::print_document,
            file_ops::create_new,
            file_ops::get_current_path,
            file_ops::open_pdf_viewer,
            bridge::exec_command,
            bridge::set_window_title,
            bridge::set_document_modified,
            bridge::load_font,
            bridge::js_log,
        ])
        .register_uri_scheme_protocol("ascdesktop", |ctx, request| {
            let uri = request.uri().to_string();
            let path = uri
                .strip_prefix("ascdesktop://")
                .or_else(|| uri.strip_prefix("ascdesktop:///"))
                .unwrap_or("");

            let resource_dir = ctx.app_handle().path().resource_dir().unwrap_or_default();

            let candidates = [
                resource_dir.join(path),
                resource_dir.join("../src").join(path),
                resource_dir.join("binaries").join(path),
            ];

            let result = candidates.iter().find_map(|p| std::fs::read(p).ok());

            match result {
                Some(data) => {
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
                None => tauri::http::Response::builder()
                    .status(404)
                    .body(b"Not Found".to_vec())
                    .unwrap(),
            }
        })
        .setup(move |app| {
            let resource_dir = app.path().resource_dir().unwrap_or_default();
            let binaries_dir = resource_dir.join("binaries");

            log_startup(&temp_dir, &format!("Resource dir: {:?}", resource_dir));
            log_startup(&temp_dir, &format!("Binaries dir exists: {}", binaries_dir.exists()));

            run_font_generation(&temp_dir, &binaries_dir);

            #[cfg(feature = "devtools")]
            if let Some(w) = app.get_webview_window("main") {
                w.open_devtools();
            }

            if let Some(ref file_path) = file_to_open {
                let handle = app.handle().clone();
                let fp = file_path.clone();
                tauri::async_runtime::spawn(async move {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let _ = handle.emit("open-file", fp);
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running Euro-Office Lite");
}

fn run_font_generation(temp_dir: &std::path::Path, binaries_dir: &std::path::Path) {
    let marker = temp_dir.join(".fonts_generated");

    if marker.exists() {
        log_startup(temp_dir, "Font generation marker found, skipping regeneration");
        return;
    }

    log_startup(temp_dir, "First-run font generation starting...");

    let mut search_dirs = vec![
        binaries_dir.to_path_buf(),
        binaries_dir.parent().unwrap_or(binaries_dir).to_path_buf(),
    ];
    if let Some(resources) = binaries_dir.parent() {
        if let Some(contents) = resources.parent() {
            let macos_dir = contents.join("MacOS");
            if macos_dir.exists() {
                search_dirs.push(macos_dir);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            search_dirs.push(exe_dir.to_path_buf());
        }
    }
    let x2t_exe = search_dirs.iter().find_map(|dir| {
        std::fs::read_dir(dir).ok().and_then(|rd| {
            rd.filter_map(|e| e.ok()).find(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                converter::is_x2t_binary(&n)
            })
        })
    }).map(|e| e.path());

    let x2t_exe = match x2t_exe {
        Some(p) => p,
        None => {
            log_startup(temp_dir, "ERROR: x2t executable not found, cannot generate fonts");
            return;
        }
    };

    let fonts_dir = binaries_dir.join("fonts");
    let binaries_str = binaries_dir.to_string_lossy().to_string();
    let fonts_str = fonts_dir.to_string_lossy().to_string();

    log_startup(temp_dir, &format!("Running: {:?} -create-allfonts {} {}", x2t_exe, binaries_str, fonts_str));

    match std::process::Command::new(&x2t_exe)
        .current_dir(binaries_dir)
        .arg("-create-allfonts")
        .arg(&binaries_str)
        .arg(&fonts_str)
        .output()
    {
        Ok(result) => {
            let code = result.status.code().unwrap_or(-1);
            log_startup(temp_dir, &format!("Font generation exit code: {}", code));
            if !result.stdout.is_empty() {
                log_startup(temp_dir, &format!("stdout: {}", String::from_utf8_lossy(&result.stdout)));
            }
            if !result.stderr.is_empty() {
                log_startup(temp_dir, &format!("stderr: {}", String::from_utf8_lossy(&result.stderr)));
            }
            if result.status.success() {
                let _ = std::fs::write(&marker, "generated");
                log_startup(temp_dir, "Font generation complete, marker written");
            }
        }
        Err(e) => {
            log_startup(temp_dir, &format!("ERROR: Font generation failed: {}", e));
        }
    }
}
