#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod clipboard;
mod converter;
mod file_ops;

use file_ops::AppState;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

fn percent_decode_str(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(
                &input[i + 1..i + 3], 16
            ) {
                out.push(val);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

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
    #[cfg(target_os = "macos")]
    let temp_dir = std::path::PathBuf::from("/tmp/euro-office-lite");
    #[cfg(not(target_os = "macos"))]
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
            file_ops::convert_for_insert,
            file_ops::write_download_temp,
            file_ops::get_system_fonts,
            bridge::exec_command,
            bridge::set_window_title,
            bridge::set_document_modified,
            bridge::load_font,
            bridge::list_media_dir,
            bridge::js_log,
            bridge::force_close,
            clipboard::read_clipboard_image,
            clipboard::read_clipboard_text,
        ])
        .register_uri_scheme_protocol("ascdesktop", |ctx, request| {
            let uri = request.uri().to_string();
            let path = uri
                .strip_prefix("ascdesktop://")
                .or_else(|| uri.strip_prefix("ascdesktop:///"))
                .or_else(|| uri.strip_prefix("http://ascdesktop.localhost/"))
                .or_else(|| uri.strip_prefix("https://ascdesktop.localhost/"))
                .unwrap_or("");
            let path = path.strip_prefix("localhost/").unwrap_or(path);

            #[cfg(debug_assertions)]
            eprintln!("[ascdesktop] uri={}", uri);

            let decoded_path = percent_decode_str(path);

            if decoded_path.starts_with("docmedia/") {
                let raw_path = &decoded_path[9..];
                let rel_path = if let Some(pos) = raw_path.find("ascdesktop://docmedia/") {
                    &raw_path[pos + 21..]
                } else {
                    raw_path
                };
                let rel_path = rel_path.trim_start_matches('/');
                let rel_path = rel_path.replace("media/media/", "media/");
                let state = ctx.app_handle().state::<AppState>();
                let full_path = state.temp_dir.join(&rel_path);
                if let Ok(data) = std::fs::read(&full_path) {
                    let fp: &str = decoded_path.as_ref();
                    let ct = if fp.ends_with(".png") { "image/png" }
                        else if fp.ends_with(".jpg") || fp.ends_with(".jpeg") { "image/jpeg" }
                        else if fp.ends_with(".gif") { "image/gif" }
                        else if fp.ends_with(".svg") { "image/svg+xml" }
                        else if fp.ends_with(".bmp") { "image/bmp" }
                        else if fp.ends_with(".webp") { "image/webp" }
                        else { "application/octet-stream" };
                    // no-store: x2t names media image1.jpg, image2.jpg... per document, so the
                    // same URL serves different content across documents (journal 026 follow-up)
                    return tauri::http::Response::builder()
                        .status(200)
                        .header("Content-Type", ct)
                        .header("Cache-Control", "no-store")
                        .header("Access-Control-Allow-Origin", "*")
                        .body(data)
                        .unwrap();
                }
                return tauri::http::Response::builder()
                    .status(404)
                    .header("Cache-Control", "no-store")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(b"Not Found".to_vec())
                    .unwrap();
            }

            if decoded_path.starts_with("copy-to-media/") {
                let src_path = &decoded_path[14..];
                let src = std::path::Path::new(src_path);
                let state = ctx.app_handle().state::<AppState>();
                let media_dir = state.temp_dir.join("media");
                let _ = std::fs::create_dir_all(&media_dir);
                if let Some(file_name) = src.file_name() {
                    let dest = media_dir.join(file_name);
                    if dest.exists() || std::fs::copy(src, &dest).is_ok() {
                        let name = file_name.to_string_lossy().to_string();
                        // no-store: a cached response would skip the actual copy after media/ is
                        // cleared on document switch, leaving a dangling reference
                        return tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", "text/plain")
                            .header("Cache-Control", "no-store")
                            .header("Access-Control-Allow-Origin", "*")
                            .body(name.into_bytes())
                            .unwrap();
                    }
                }
                return tauri::http::Response::builder()
                    .status(500)
                    .header("Cache-Control", "no-store")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(b"Copy failed".to_vec())
                    .unwrap();
            }

            if decoded_path.starts_with("download-to-media/") {
                let url = &decoded_path[18..];
                let state = ctx.app_handle().state::<AppState>();
                let downloads_dir = state.temp_dir.join("downloads");
                let media_dir = state.temp_dir.join("media");
                let _ = std::fs::create_dir_all(&downloads_dir);
                let _ = std::fs::create_dir_all(&media_dir);
                let file_name = url.rsplit('/').next().unwrap_or("download.jpg")
                    .split('?').next().unwrap_or("download.jpg");
                let safe_name: String = file_name.chars()
                    .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
                    .collect();
                let dest_name = if safe_name.is_empty() { "download.jpg".to_string() } else { safe_name };
                let dest_download = downloads_dir.join(&dest_name);
                let dest = media_dir.join(&dest_name);
                if let Ok(resp) = ureq::get(url).call() {
                    if let Ok(bytes) = resp.into_body().read_to_vec() {
                        let _ = std::fs::write(&dest, &bytes);
                        if std::fs::write(&dest_download, &bytes).is_ok() {
                            let full_path = dest_download.to_string_lossy().to_string();
                            return tauri::http::Response::builder()
                                .status(200)
                                .header("Content-Type", "text/plain")
                                .header("Cache-Control", "no-store")
                                .header("Access-Control-Allow-Origin", "*")
                                .body(full_path.into_bytes())
                                .unwrap();
                        }
                    }
                }
                return tauri::http::Response::builder()
                    .status(500)
                    .header("Cache-Control", "no-store")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(b"Download failed".to_vec())
                    .unwrap();
            }

            let result = if decoded_path.starts_with("abs/") {
                let abs_path = &decoded_path[4..];
                std::fs::read(abs_path).ok()
            } else if decoded_path.ends_with("sdkjs/common/AllFonts.js") {
                let state = ctx.app_handle().state::<AppState>();
                let generated = state.temp_dir.join("fontdata").join("AllFonts.js");
                std::fs::read(&generated).ok().or_else(|| {
                    let resource_dir = ctx.app_handle().path().resource_dir().unwrap_or_default();
                    let candidates = [
                        resource_dir.join(path),
                        resource_dir.join("../src").join(path),
                        resource_dir.join("binaries").join(path),
                    ];
                    candidates.iter().find_map(|p| std::fs::read(p).ok())
                })
            } else {
                let resource_dir = ctx.app_handle().path().resource_dir().unwrap_or_default();
                let candidates = [
                    resource_dir.join(path),
                    resource_dir.join("../src").join(path),
                    resource_dir.join("binaries").join(path),
                ];
                candidates.iter().find_map(|p| std::fs::read(p).ok())
            };

            let effective_path: &str = decoded_path.as_ref();
            let mime = if effective_path.ends_with(".ttf") || effective_path.ends_with(".otf") {
                "font/ttf"
            } else if effective_path.ends_with(".js") {
                "application/javascript"
            } else if effective_path.ends_with(".png") {
                "image/png"
            } else if effective_path.ends_with(".jpg") || effective_path.ends_with(".jpeg") {
                "image/jpeg"
            } else if effective_path.ends_with(".gif") {
                "image/gif"
            } else if effective_path.ends_with(".svg") {
                "image/svg+xml"
            } else if effective_path.ends_with(".bmp") {
                "image/bmp"
            } else if effective_path.ends_with(".webp") {
                "image/webp"
            } else if effective_path.ends_with(".tif") || effective_path.ends_with(".tiff") {
                "image/tiff"
            } else if effective_path.ends_with(".ico") {
                "image/x-icon"
            } else {
                "application/octet-stream"
            };

            match result {
                Some(data) => tauri::http::Response::builder()
                    .status(200)
                    .header("Content-Type", mime)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(data)
                    .unwrap(),
                None => tauri::http::Response::builder()
                    .status(404)
                    .header("Access-Control-Allow-Origin", "*")
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

            let handle = app.handle().clone();
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(feature = "devtools")]
                window.open_devtools();

                let h = handle.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let state = h.state::<AppState>();
                        let modified = *state.modified.lock().unwrap();
                        if modified {
                            api.prevent_close();
                            let _ = h.emit("confirm-close", ());
                        }
                    }
                });
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

    let allfonts_server = temp_dir.join("fontdata").join("AllFonts.js");
    if marker.exists() && allfonts_server.exists() {
        log_startup(temp_dir, "Font generation marker found, skipping regeneration");
        return;
    }
    if marker.exists() && !allfonts_server.exists() {
        log_startup(temp_dir, "Marker exists but fontdata/AllFonts.js missing, regenerating");
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
    let fontdata_dir = temp_dir.join("fontdata");
    let _ = std::fs::create_dir_all(&fontdata_dir);
    let fontdata_str = fontdata_dir.to_string_lossy().to_string();
    let fonts_str = fonts_dir.to_string_lossy().to_string();

    log_startup(temp_dir, &format!("Running: {:?} -create-allfonts {} {}", x2t_exe, fontdata_str, fonts_str));

    let mut cmd = std::process::Command::new(&x2t_exe);
    cmd.current_dir(binaries_dir)
        .arg("-create-allfonts")
        .arg(&fontdata_str)
        .arg(&fonts_str);
    #[cfg(target_os = "linux")]
    cmd.env("LD_LIBRARY_PATH", binaries_dir);
    match cmd.output() {
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
                if let Ok(entries) = std::fs::read_dir(&fontdata_dir) {
                    for entry in entries.flatten() {
                        let size = std::fs::metadata(entry.path()).map(|m| m.len()).unwrap_or(0);
                        log_startup(temp_dir, &format!("  fontdata/{}: {} bytes",
                            entry.file_name().to_string_lossy(), size));
                    }
                }
                let generated_allfonts = fontdata_dir.join("AllFonts.js");
                if let Ok(content) = std::fs::read_to_string(&generated_allfonts) {
                    log_startup(temp_dir, &format!("Generated AllFonts.js: {} lines, {} bytes",
                        content.lines().count(), content.len()));
                    for line in content.lines().take(3) {
                        let truncated = &line[..line.len().min(150)];
                        log_startup(temp_dir, &format!("  {}", truncated));
                    }
                }
            }
        }
        Err(e) => {
            log_startup(temp_dir, &format!("ERROR: Font generation failed: {}", e));
        }
    }
}
