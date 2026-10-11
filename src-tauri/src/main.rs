#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod clipboard;
mod converter;
mod dictionaries;
mod file_ops;
mod note_separator;
mod recent;
mod recovery;
mod text_import;

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

// x2t runs with a different working directory, so a relative launch argument
// must be anchored to ours. A plain join, not canonicalize: that would resolve
// symlinks and, on Windows, add the \\?\ prefix to the path kept in recents.
fn absolute_arg_path(arg: &str, cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    if arg.starts_with('-') {
        return None;
    }
    let path = cwd.join(arg);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn main() {
    // WebKitGTK's DMABUF renderer produces jagged/rough canvas rendering on some
    // drivers (Nvidia proprietary especially, issue #27). The Flatpak already sets
    // this for everyone; mirror it here so the .deb matches. Set before any WebKit
    // initialization, and only if the user has not set their own value (so
    // WEBKIT_DISABLE_DMABUF_RENDERER=0 re-enables the DMABUF path).
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let context = tauri::generate_context!();
    // The user-facing version comes from tauri.conf.json; Cargo's crate version stays independent.
    let app_version = context.package_info().version.clone();

    #[cfg(target_os = "macos")]
    let temp_dir = std::path::PathBuf::from("/tmp/euro-office-lite");
    #[cfg(not(target_os = "macos"))]
    let temp_dir = std::env::temp_dir().join("euro-office-lite");
    std::fs::create_dir_all(&temp_dir).ok();

    {
        use std::io::Write;
        let log_path = temp_dir.join("js-debug.log");
        if let Ok(mut f) = std::fs::File::create(&log_path) {
            let _ = writeln!(
                f,
                "[STARTUP] Euro Office Lite {} platform={} arch={}",
                app_version,
                std::env::consts::OS,
                std::env::consts::ARCH
            );
        }
    }

    #[cfg(target_os = "linux")]
    {
        let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let distro = os_release
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map(|value| value.trim_matches('"'))
            .unwrap_or("unknown");
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_string());
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_else(|_| "unknown".to_string());
        let dmabuf = std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER")
            .unwrap_or_else(|_| "unset".to_string());
        log_startup(
            &temp_dir,
            &format!(
                "Linux distro={} session={} desktop={} WEBKIT_DISABLE_DMABUF_RENDERER={}",
                distro, session, desktop, dmabuf
            ),
        );
    }

    let file_to_open: Option<String> = {
        let args: Vec<String> = std::env::args().collect();
        // An empty base leaves a relative path untouched, which is what we can do
        // when the working directory is unreadable.
        let cwd = std::env::current_dir().unwrap_or_default();
        let path = args
            .get(1)
            .and_then(|arg| absolute_arg_path(arg, &cwd))
            .and_then(|path| path.into_os_string().into_string().ok());
        if let Some(ref path) = path {
            let file_name = std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
            log_startup(&temp_dir, &format!("Opening associated file: {}", file_name));
        }
        path
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
            pending_recent: Mutex::new(None),
            recovery: Mutex::new(None),
            pending_open: Mutex::new(file_to_open),
            text_options: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            file_ops::open_file,
            file_ops::save_file,
            file_ops::save_file_as,
            file_ops::write_editor_bin,
            file_ops::print_document,
            file_ops::create_new,
            file_ops::get_current_path,
            file_ops::take_pending_open_file,
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
            note_separator::remove_note_separator,
            clipboard::read_clipboard_image,
            clipboard::read_clipboard_text,
            recent::recent_files_state,
            recent::set_recent_files_enabled,
            recent::clear_recent_files,
            recovery::recovery_begin,
            recovery::save_changes,
            recovery::recovery_mark_saved,
            recovery::recovery_end,
            recovery::recovery_candidates,
            recovery::recovery_load,
            recovery::recovery_discard,
            dictionaries::list_user_dictionaries,
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
                    // same URL serves different content across documents
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
                let state = ctx.app_handle().state::<AppState>();
                let media_dir = state.temp_dir.join("media");
                let _ = std::fs::create_dir_all(&media_dir);
                // After a compare or a merge, sdkjs does not register the url map
                // convert_for_insert hands it and asks for the bare name the inserted
                // binary carries ("image1.png"), which is not a path this can read.
                // Resolving it against insert_tmp/media/ is what turns that request
                // into the image the user actually inserted; staging is keyed on
                // content, so the file already staged there answers with the very same
                // name it got at convert time instead of piling up a second copy.
                // Only the file name, so a relative request cannot walk out of that
                // directory with a ..
                let insert_src = state
                    .temp_dir
                    .join("insert_tmp")
                    .join("media")
                    .join(std::path::Path::new(src_path).file_name().unwrap_or_default());
                let src = if std::path::Path::new(src_path).is_absolute() {
                    std::path::Path::new(src_path)
                } else {
                    insert_src.as_path()
                };
                // The reply is the name sdkjs will write into the document, which is not
                // necessarily the source's: see stage_into_media for the collision rules.
                if let Some(name) = file_ops::stage_into_media(&media_dir, src) {
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
                if let Ok(resp) = ureq::get(url).call() {
                    if let Ok(bytes) = resp.into_body().read_to_vec() {
                        // Staging picks the final name, which may not be dest_name if
                        // media/ already holds a different image under it. downloads/
                        // has to follow that name: the bridge answers this request by
                        // taking the basename of the path below and writing it into the
                        // document, so the two have to agree on one name (bridge.js:1111).
                        if let Some(name) =
                            file_ops::stage_bytes_into_media(&media_dir, &dest_name, &bytes)
                        {
                            let dest_download = downloads_dir.join(&name);
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
                }
                return tauri::http::Response::builder()
                    .status(500)
                    .header("Cache-Control", "no-store")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(b"Download failed".to_vec())
                    .unwrap();
            }

            // Spellcheck dictionaries. The worker resolves every language
            // against a single base, so this is where the user's own folder and
            // the bundled ones meet: the user's disk answers first and whatever
            // is not there falls back to the asset baked into the binary, which
            // keeps es_ES and en_US coming out of the bundle without a second
            // code path in the bridge. split_request is the whitelist that makes
            // reading from a user-writable directory safe.
            if decoded_path.starts_with("dictionaries/") {
                let served = dictionaries::split_request(&decoded_path).and_then(|(folder, file)| {
                    dictionaries::read_user_file(ctx.app_handle(), &folder, &file)
                        .map(|bytes| (bytes, "application/octet-stream".to_string()))
                        .or_else(|| {
                            ctx.app_handle()
                                .asset_resolver()
                                .get(format!("/{}/{}/{}", dictionaries::DIR_NAME, folder, file))
                                .filter(|asset| {
                                    dictionaries::is_dictionary_asset(&asset.bytes)
                                })
                                .map(|asset| (asset.bytes, asset.mime_type))
                        })
                });
                // no-store: the point of the folder is that replacing a file and
                // restarting picks the new one up, which a cached response would
                // quietly defeat.
                return match served {
                    Some((bytes, mime)) => tauri::http::Response::builder()
                        .status(200)
                        .header("Content-Type", mime)
                        .header("Cache-Control", "no-store")
                        .header("Access-Control-Allow-Origin", "*")
                        .body(bytes)
                        .unwrap(),
                    None => tauri::http::Response::builder()
                        .status(404)
                        .header("Cache-Control", "no-store")
                        .header("Access-Control-Allow-Origin", "*")
                        .body(b"Not Found".to_vec())
                        .unwrap(),
                };
            }

            let result = if decoded_path.starts_with("abs/") {
                let raw_abs_path = &decoded_path[4..];
                // x2t normalizes Windows extended paths (\\?\C:\...) as /?/C:/...
                // in AllFonts.js. That spelling is not a valid path for std::fs::read.
                let abs_path = if raw_abs_path.starts_with("/?/")
                    && raw_abs_path.as_bytes().get(4) == Some(&b':')
                {
                    &raw_abs_path[3..]
                } else {
                    raw_abs_path
                };
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

            if !binaries_dir.is_dir() {
                log_startup(&temp_dir, "ERROR: bundled binaries directory is missing");
            }

            run_font_generation(&temp_dir, &binaries_dir);

            let handle = app.handle().clone();
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(debug_assertions)]
                window.open_devtools();

                // On Wayland a trackpad pinch arrives as a GDK_TOUCHPAD_PINCH event
                // that WebKitGTK consumes at the widget level and turns into a scale
                // of the whole interface. Two things were ruled out by testing: it
                // does not go through the WebKitWebView zoom-level property, and it
                // never reaches the DOM, so the JS wheel guard in editor-patches.js
                // has nothing to cancel. None of the 64 WebKitSettings properties
                // switch the gesture off. Claim the event before WebKit's own
                // handler runs, which is the earliest public hook available.
                #[cfg(target_os = "linux")]
                {
                    use gtk::prelude::*;
                    let pinch_log = temp_dir.clone();
                    let reported = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let eval_window = window.clone();
                    if let Err(e) = window.with_webview(move |webview| {
                        let wv = webview.inner();
                        log_startup(&pinch_log, "pinch guard installed at GTK level");

                        // gdk 0.18 mis-generates the phase accessor as is_phase() -> bool,
                        // so gesture boundaries are inferred from the event timestamps
                        // instead. scale() is relative to the start of the gesture, which
                        // makes the ratio between consecutive events the useful quantity.
                        let last_time = std::cell::Cell::new(0u32);
                        let last_scale = std::cell::Cell::new(1.0f64);
                        let pending = std::cell::Cell::new(1.0f64);

                        wv.connect_event(move |_, event| {
                            if event.event_type() != gtk::gdk::EventType::TouchpadPinch {
                                return gtk::glib::Propagation::Proceed;
                            }
                            if !reported.swap(true, std::sync::atomic::Ordering::Relaxed) {
                                log_startup(&pinch_log, "touchpad pinch seen at GTK level, event claimed");
                            }

                            if let Some(pinch) = event.downcast_ref::<gtk::gdk::EventTouchpadPinch>() {
                                let time = pinch.time();
                                let scale = pinch.scale();
                                if time.saturating_sub(last_time.get()) > 300 {
                                    // Long gap means a new gesture, so start a fresh
                                    // baseline rather than comparing against the old one.
                                    pending.set(1.0);
                                } else if last_scale.get() > 0.0 {
                                    pending.set(pending.get() * (scale / last_scale.get()));
                                }
                                last_scale.set(scale);
                                last_time.set(time);

                                // Apply in 10% steps so the document zoom moves the way it
                                // does for Ctrl+= rather than sliding continuously, which
                                // would re-render the canvas on every event.
                                let ratio = pending.get();
                                if !(0.9..=1.1).contains(&ratio) {
                                    let step = if ratio > 1.0 { 10 } else { -10 };
                                    pending.set(1.0);
                                    // Position travels with the step so the page can
                                    // decide whether the gesture is over the document
                                    // rather than the surrounding chrome.
                                    let (x, y) = pinch.position();
                                    let _ = eval_window.eval(&format!(
                                        "window.__eoPinchZoom && window.__eoPinchZoom({step},{x:.1},{y:.1})"
                                    ));
                                }
                            }
                            gtk::glib::Propagation::Stop
                        });
                    }) {
                        log_startup(&temp_dir, &format!("ERROR: pinch guard not installed: {e}"));
                    }
                }

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

            Ok(())
        })
        .build(context)
        .expect("error running Euro-Office Lite")
        // The one point every ordinary exit passes through: closing an
        // unmodified document never reaches force_close (Tauri just closes the
        // window), and neither path may leave a recovery folder behind. A
        // killed process never runs this, which is the case the folder is for.
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                recovery::end_session(&app_handle.state::<AppState>(), true);
            }
        });
}

// x2t does not use fontconfig: it scans a fixed list of directories
// (/usr/share/fonts, /usr/share/X11/fonts, /usr/local/share/fonts and a couple more),
// so fonts installed per user or exposed by the Flatpak host mounts are invisible to
// the editor even though fc-list finds them (issue #29). Passing them explicitly is
// safe: x2t deduplicates directories it already scans.
#[cfg(target_os = "linux")]
fn extra_font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = vec![
        // Flatpak host font mounts
        std::path::PathBuf::from("/run/host/fonts"),
        std::path::PathBuf::from("/run/host/local-fonts"),
        std::path::PathBuf::from("/run/host/user-fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    dirs.retain(|dir| dir.is_dir());
    dirs
}

#[cfg(not(target_os = "linux"))]
fn extra_font_dirs() -> Vec<std::path::PathBuf> {
    Vec::new()
}

#[cfg(target_os = "linux")]
const FONT_EXTENSIONS: [&str; 5] = ["ttf", "otf", "ttc", "pfb", "pfa"];

#[cfg(target_os = "linux")]
fn has_font_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext = ext.to_ascii_lowercase();
            FONT_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

// x2t's font scanner skips symlink entries when it lists a directory, so fonts
// installed as links (on NixOS every font is one) never reach the editor.
// Resolving the links and handing over their real directories is safe: x2t
// deduplicates directories it already scans.
#[cfg(target_os = "linux")]
fn symlink_font_target_dirs(roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    const MAX_DEPTH: usize = 8;

    let mut found: Vec<std::path::PathBuf> = Vec::new();
    // Walking with an explicit stack and symlink_metadata: following directory
    // links while walking can loop forever.
    let mut stack: Vec<(std::path::PathBuf, usize)> =
        roots.iter().map(|root| (root.clone(), 0usize)).collect();

    while let Some((dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let link_meta = match std::fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if link_meta.file_type().is_symlink() {
                let target = match std::fs::canonicalize(&path) {
                    Ok(target) => target,
                    Err(_) => continue,
                };
                let target_meta = match std::fs::metadata(&target) {
                    Ok(meta) => meta,
                    Err(_) => continue,
                };
                if target_meta.is_dir() {
                    found.push(target);
                } else if has_font_extension(&target) {
                    if let Some(parent) = target.parent() {
                        found.push(parent.to_path_buf());
                    }
                }
            } else if link_meta.is_dir() && depth + 1 < MAX_DEPTH {
                stack.push((path, depth + 1));
            }
        }
    }

    found.sort();
    found.dedup();
    // A target under a root is already covered: x2t scans the roots recursively,
    // and handing it the same font twice shows a duplicate in the font list.
    let roots: Vec<std::path::PathBuf> = roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
    found.retain(|dir| !roots.iter().any(|root| dir.starts_with(root)));
    found
}

#[cfg(not(target_os = "linux"))]
fn symlink_font_target_dirs(_roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    Vec::new()
}

// The directories x2t scans on its own. It skips their symlinks too, so they
// are walked here even though they are not passed as extra arguments.
#[cfg(target_os = "linux")]
fn symlink_scan_roots() -> Vec<std::path::PathBuf> {
    let mut roots = extra_font_dirs();
    for standard in [
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/usr/share/X11/fonts",
    ] {
        let dir = std::path::PathBuf::from(standard);
        if dir.is_dir() {
            roots.push(dir);
        }
    }
    roots
}

#[cfg(not(target_os = "linux"))]
fn symlink_scan_roots() -> Vec<std::path::PathBuf> {
    Vec::new()
}

fn run_font_generation(temp_dir: &std::path::Path, binaries_dir: &std::path::Path) {
    let marker = temp_dir.join(".fonts_generated");

    let allfonts_server = temp_dir.join("fontdata").join("AllFonts.js");
    if marker.exists() && allfonts_server.exists() {
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

    let mut cmd = std::process::Command::new(&x2t_exe);
    cmd.current_dir(binaries_dir)
        .arg("-create-allfonts")
        .arg(&fontdata_str)
        .arg(&fonts_str);
    for dir in extra_font_dirs() {
        log_startup(temp_dir, &format!("Extra font directory: {}", dir.display()));
        cmd.arg(dir);
    }
    let symlink_dirs = symlink_font_target_dirs(&symlink_scan_roots());
    for dir in &symlink_dirs {
        log_startup(temp_dir, &format!("Symlink font target directory: {}", dir.display()));
        cmd.arg(dir);
    }
    if !symlink_dirs.is_empty() {
        log_startup(
            temp_dir,
            &format!("Symlink font target directories resolved: {}", symlink_dirs.len()),
        );
    }
    #[cfg(target_os = "linux")]
    cmd.env("LD_LIBRARY_PATH", binaries_dir);
    match cmd.output() {
        Ok(result) => {
            let code = result.status.code().unwrap_or(-1);
            if result.status.success() {
                let _ = std::fs::write(&marker, "generated");
                let allfonts_size = std::fs::metadata(fontdata_dir.join("AllFonts.js"))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                let selection_size = std::fs::metadata(fontdata_dir.join("font_selection.bin"))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                log_startup(
                    temp_dir,
                    &format!(
                        "Font generation complete: AllFonts.js={} bytes, font_selection.bin={} bytes",
                        allfonts_size, selection_size
                    ),
                );
            } else {
                log_startup(
                    temp_dir,
                    &format!(
                        "ERROR: Font generation failed (exit code {}): {}",
                        code,
                        String::from_utf8_lossy(&result.stderr).trim()
                    ),
                );
            }
        }
        Err(e) => {
            log_startup(temp_dir, &format!("ERROR: Font generation failed: {}", e));
        }
    }
}

// The walker only has a body on Linux, so the assertions are gated the same way
// even though the symlink APIs they use exist on every unix.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("eo-fonts-{}-{}", name, unique));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup(name: &str) -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new(name);
        let root = temp.join("root");
        let store = temp.join("store");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&store).expect("create store");
        (temp, root, store)
    }

    fn resolve(root: &PathBuf) -> Vec<PathBuf> {
        symlink_font_target_dirs(&[root.clone()])
    }

    #[test]
    fn symlink_to_font_file_yields_target_parent() {
        let (_temp, root, store) = setup("file");
        std::fs::write(store.join("Sans.ttf"), b"font").expect("write font");
        symlink(store.join("Sans.ttf"), root.join("Sans.ttf")).expect("link");

        let expected = std::fs::canonicalize(&store).expect("canonicalize store");
        assert_eq!(resolve(&root), vec![expected]);
    }

    #[test]
    fn symlink_to_directory_yields_that_directory() {
        let (_temp, root, store) = setup("dir");
        std::fs::write(store.join("Sans.otf"), b"font").expect("write font");
        symlink(&store, root.join("linked")).expect("link");

        let expected = std::fs::canonicalize(&store).expect("canonicalize store");
        assert_eq!(resolve(&root), vec![expected]);
    }

    #[test]
    fn real_font_file_yields_nothing() {
        let (_temp, root, _store) = setup("real");
        std::fs::write(root.join("Sans.ttf"), b"font").expect("write font");

        assert!(resolve(&root).is_empty());
    }

    #[test]
    fn broken_symlink_yields_nothing() {
        let (_temp, root, store) = setup("broken");
        symlink(store.join("Missing.ttf"), root.join("Missing.ttf")).expect("link");

        assert!(resolve(&root).is_empty());
    }

    #[test]
    fn symlink_to_non_font_file_yields_nothing() {
        let (_temp, root, store) = setup("nonfont");
        std::fs::write(store.join("notes.txt"), b"text").expect("write file");
        symlink(store.join("notes.txt"), root.join("notes.txt")).expect("link");

        assert!(resolve(&root).is_empty());
    }

    #[test]
    fn two_symlinks_to_same_target_yield_one_directory() {
        let (_temp, root, store) = setup("dedup");
        std::fs::write(store.join("Sans.ttf"), b"font").expect("write font");
        std::fs::write(store.join("Serif.TTF"), b"font").expect("write font");
        symlink(store.join("Sans.ttf"), root.join("Sans.ttf")).expect("link");
        symlink(store.join("Serif.TTF"), root.join("Serif.TTF")).expect("link");

        let expected = std::fs::canonicalize(&store).expect("canonicalize store");
        assert_eq!(resolve(&root), vec![expected]);
    }

    #[test]
    fn target_inside_the_root_yields_nothing() {
        let (_temp, root, _store) = setup("inside");
        let other = root.join("b");
        std::fs::create_dir_all(&other).expect("create subdir");
        std::fs::create_dir_all(root.join("a")).expect("create subdir");
        std::fs::write(other.join("Sans.ttf"), b"font").expect("write font");
        symlink(other.join("Sans.ttf"), root.join("a/Sans.ttf")).expect("link");

        assert!(resolve(&root).is_empty());
    }

    #[test]
    fn nested_directories_are_walked() {
        let (_temp, root, store) = setup("nested");
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).expect("create nested");
        std::fs::write(store.join("Sans.ttf"), b"font").expect("write font");
        symlink(store.join("Sans.ttf"), nested.join("Sans.ttf")).expect("link");

        let expected = std::fs::canonicalize(&store).expect("canonicalize store");
        assert_eq!(resolve(&root), vec![expected]);
    }
}

#[cfg(test)]
mod arg_path_tests {
    use super::absolute_arg_path;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("eo-arg-{}-{}", name, unique));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn existing_absolute_path_is_kept() {
        let temp = TempDir::new("abs");
        let file = temp.0.join("doc.docx");
        std::fs::write(&file, b"x").expect("write file");
        let elsewhere = std::env::temp_dir();

        assert_eq!(
            absolute_arg_path(file.to_str().unwrap(), &elsewhere),
            Some(file.clone())
        );
    }

    #[test]
    fn existing_relative_path_is_joined_to_cwd() {
        let temp = TempDir::new("rel");
        std::fs::write(temp.0.join("doc.docx"), b"x").expect("write file");

        assert_eq!(
            absolute_arg_path("doc.docx", &temp.0),
            Some(temp.0.join("doc.docx"))
        );
    }

    #[test]
    fn missing_path_yields_none() {
        let temp = TempDir::new("missing");

        assert_eq!(absolute_arg_path("missing.docx", &temp.0), None);
    }

    #[test]
    fn dash_argument_is_ignored() {
        let temp = TempDir::new("dash");
        std::fs::write(temp.0.join("-flag"), b"x").expect("write file");

        assert_eq!(absolute_arg_path("-flag", &temp.0), None);
    }
}
