#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_quit;
mod bridge;
mod clipboard;
mod converter;
mod dictionaries;
mod file_ops;
mod launch_args;
mod note_separator;
mod protocol_limits;
mod recent;
mod recovery;
mod save_target;
mod temp_layout;
mod text_import;
mod x2t_task;

use file_ops::AppState;
use launch_args::TitleBar;
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

fn log_startup(msg: &str) {
    temp_layout::append_log(&format!("[STARTUP] {}", msg));
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

// Two tries: the id carries the millisecond, so the second one is a new folder.
fn start_instance(root: &std::path::Path) -> temp_layout::Instance {
    let pid = std::process::id();
    for _ in 0..2 {
        if let Ok(instance) = temp_layout::begin_instance(root, now_ms(), pid) {
            return instance;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    temp_layout::unlocked_instance(root, now_ms(), pid)
}

// tao adds its GTK header bar only when GDK's backend is Wayland, so that is
// the only place the title bar options mean anything. GDK picks the backend in
// gtk_init: running it here, before tauri does, gives the answer tao will get
// (its own gtk::init then returns at once) while the window does not exist yet.
#[cfg(target_os = "linux")]
fn wayland_title_bar(requested: TitleBar) -> TitleBar {
    use gtk::prelude::*;
    if requested == TitleBar::Default {
        return requested;
    }
    if let Err(e) = gtk::init() {
        log_startup(&format!(
            "WARNING: title bar option ignored, GTK did not start: {e}"
        ));
        return TitleBar::Default;
    }
    let wayland = gtk::gdk::Display::default()
        .map(|display| display.backend().is_wayland())
        .unwrap_or(false);
    if !wayland {
        log_startup("title bar option ignored: it only applies on Wayland");
        return TitleBar::Default;
    }
    requested
}

// main was built hidden (see main), so GTK has not realized it yet and
// negotiates the decorations with the mode already in place when it is shown.
// Straight on the GTK window, which is what tao's set_decorations and show end
// up calling, so the order is fixed and the window is shown before setup goes on.
#[cfg(target_os = "linux")]
fn apply_title_bar(app: &tauri::App, mode: TitleBar) {
    use gtk::prelude::*;
    if mode == TitleBar::Default {
        return;
    }
    let Some(window) = app.get_webview_window("main") else {
        log_startup("ERROR: title bar: no main window");
        return;
    };
    let gtk_window = match window.gtk_window() {
        Ok(gtk_window) => gtk_window,
        Err(e) => {
            log_startup(&format!("ERROR: title bar: no GTK window: {e}"));
            // Never leave the hidden window hidden.
            let _ = window.show();
            return;
        }
    };
    let mut note = "";
    match mode {
        TitleBar::System if gtk_window.titlebar().is_none() => {
            note = " (no GTK header bar to remove)";
        }
        TitleBar::System => gtk_window.set_titlebar(None::<&gtk::Widget>),
        TitleBar::Undecorated => gtk_window.set_decorated(false),
        TitleBar::Default => {}
    }
    let realized_before_show = gtk_window.is_realized();
    gtk_window.show_all();
    log_startup(&format!(
        "title bar mode={:?}{} realized_before_show={}",
        mode, note, realized_before_show
    ));
}

// The first of the places a bundled resource can live that holds it.
fn read_resource(app: &tauri::AppHandle, relative: &std::path::Path) -> Option<Vec<u8>> {
    let resource_dir = app.path().resource_dir().unwrap_or_default();
    let candidates = [
        resource_dir.join(relative),
        resource_dir.join("../src").join(relative),
        resource_dir.join("binaries").join(relative),
    ];
    candidates.iter().find_map(|p| std::fs::read(p).ok())
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

    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut context = tauri::generate_context!();
    // The user-facing version comes from tauri.conf.json; Cargo's crate version stays independent.
    let app_version = context.package_info().version.clone();

    // The root is shared by every instance; the document's files go in this
    // process's own folder under it (temp_layout.rs).
    let root_choice = match temp_layout::temp_root(&context.config().identifier) {
        Ok(choice) => choice,
        Err(reason) => {
            // Without a temp folder of its own no document can be opened or
            // saved, and there is no log folder to write this to either.
            eprintln!("Euro-Office Lite cannot start: no usable temp folder ({})", reason);
            std::process::exit(1);
        }
    };
    let temp_root = root_choice.path;
    std::fs::create_dir_all(&temp_root).ok();
    let instance = start_instance(&temp_root);
    let temp_dir = instance.dir.clone();
    let instance_id = instance.id.clone();
    // Document 1 is whatever the instance shows first (temp_layout.rs).
    let _ = temp_layout::write_doc_seq(&temp_dir, 1);

    temp_layout::start_log(
        &temp_root,
        &instance_id,
        &format!(
            "[STARTUP] Euro Office Lite {} platform={} arch={}",
            app_version,
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    if let Some(note) = &root_choice.note {
        log_startup(&format!(
            "WARNING: temp folder {} is not the preferred one: {}",
            temp_root.display(),
            note
        ));
    }
    if !instance.holds_lock() {
        log_startup(&format!(
            "WARNING: could not lock the instance folder {}",
            temp_dir.display()
        ));
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
        log_startup(&format!(
            "Linux distro={} session={} desktop={} WEBKIT_DISABLE_DMABUF_RENDERER={}",
            distro, session, desktop, dmabuf
        ));
    }

    let launch = launch_args::parse_launch_args(std::env::args().skip(1));
    let (title_bar, notes) = launch_args::resolve_title_bar(
        &launch,
        std::env::var(launch_args::TITLE_BAR_ENV).ok().as_deref(),
    );
    for note in &notes {
        log_startup(note);
    }
    // GTK picks client- or server-side decorations when it realizes the window,
    // and tao realizes a visible window while building it, before setup runs:
    // build it hidden and let setup apply the mode and show it.
    #[cfg(target_os = "linux")]
    let title_bar = wayland_title_bar(title_bar);
    #[cfg(target_os = "linux")]
    if title_bar != TitleBar::Default {
        for window in context.config_mut().app.windows.iter_mut() {
            if window.label == "main" {
                window.visible = false;
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    if title_bar != TitleBar::Default {
        log_startup("title bar option ignored: it only applies on Linux Wayland");
    }

    let file_to_open: Option<String> = {
        // An empty base leaves a relative path untouched, which is what we can do
        // when the working directory is unreadable.
        let cwd = std::env::current_dir().unwrap_or_default();
        let path = launch
            .document
            .as_deref()
            .and_then(|arg| absolute_arg_path(arg, &cwd))
            .and_then(|path| path.into_os_string().into_string().ok());
        if let Some(ref path) = path {
            let file_name = std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
            log_startup(&format!("Opening associated file: {}", file_name));
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

    #[cfg(target_os = "macos")]
    {
        builder = builder.menu(app_quit::macos_menu);
    }

    builder = builder.on_menu_event(app_quit::on_menu_event);

    builder.manage(AppState {
            current_file: Mutex::new(None),
            temp_dir,
            temp_root: temp_root.clone(),
            instance,
            doc_seq: std::sync::atomic::AtomicU64::new(1),
            modified: Mutex::new(false),
            pending_recent: Mutex::new(None),
            recovery: Mutex::new(None),
            pending_open: Mutex::new(file_to_open),
            text_options: Mutex::new(None),
            last_print_pdf: Mutex::new(None),
            picked_images: Mutex::new(Vec::new()),
            pasted_images: Mutex::new(Vec::new()),
            save_suggestion: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            file_ops::open_file,
            file_ops::open_pdf,
            file_ops::save_file,
            file_ops::save_file_as,
            file_ops::save_pdf,
            file_ops::write_editor_bin,
            file_ops::print_document,
            file_ops::create_new,
            file_ops::get_current_path,
            file_ops::pick_image_files,
            file_ops::take_pending_open_file,
            file_ops::open_pdf_viewer,
            file_ops::print_to_printer,
            file_ops::convert_for_insert,
            file_ops::get_system_fonts,
            bridge::set_window_title,
            bridge::set_document_modified,
            bridge::js_log,
            bridge::force_close,
            note_separator::remove_note_separator,
            clipboard::read_clipboard_image,
            clipboard::read_clipboard_text,
            clipboard::read_clipboard_html,
            clipboard::grant_pasted_image,
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
            recovery::get_save_suggestion,
            dictionaries::list_user_dictionaries,
        ])
        .register_uri_scheme_protocol("ascdesktop", |ctx, request| {
            let allow_origin = protocol_limits::allow_origin();
            let uri = request.uri().to_string();
            let path = temp_layout::protocol_path(&uri);

            #[cfg(debug_assertions)]
            eprintln!("[ascdesktop] uri={}", uri);

            let decoded_path = percent_decode_str(path);

            if decoded_path.starts_with("docmedia/") {
                let state = ctx.app_handle().state::<AppState>();
                let served = temp_layout::serve_media_path(
                    &file_ops::open_document(&state),
                    &decoded_path,
                )
                .and_then(|full_path| std::fs::read(full_path).ok());
                if let Some(data) = served {
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
                        .header(allow_origin.0, allow_origin.1)
                        .body(data)
                        .unwrap();
                }
                return tauri::http::Response::builder()
                    .status(404)
                    .header("Cache-Control", "no-store")
                    .header(allow_origin.0, allow_origin.1)
                    .body(b"Not Found".to_vec())
                    .unwrap();
            }

            // The address the bridge gives the open document's images.
            if temp_layout::is_doc_key_path(&decoded_path) {
                let state = ctx.app_handle().state::<AppState>();
                let doc = file_ops::open_document(&state);
                return tauri::http::Response::builder()
                    .status(200)
                    .header("Content-Type", "text/plain")
                    .header("Cache-Control", "no-store")
                    .header(allow_origin.0, allow_origin.1)
                    .body(temp_layout::doc_key(doc.id, doc.seq).into_bytes())
                    .unwrap();
            }

            // An image pasted from another document: copied into this one's
            // media/, answered with the name to reference it by.
            if let Some(url) = decoded_path.strip_prefix("import-media/") {
                let state = ctx.app_handle().state::<AppState>();
                let imported = temp_layout::import_media(
                    &file_ops::open_document(&state),
                    url,
                    file_ops::stage_bytes_into_media,
                );
                let shown: String = url.chars().take(200).collect();
                return match imported {
                    Some(name) => {
                        file_ops::log_event(
                            &state,
                            &format!("[PASTE] imported {} -> {}", shown, name),
                        );
                        tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", "text/plain")
                            .header("Cache-Control", "no-store")
                            .header(allow_origin.0, allow_origin.1)
                            .body(name.into_bytes())
                            .unwrap()
                    }
                    None => {
                        file_ops::log_event(&state, &format!("[PASTE] source gone {}", shown));
                        tauri::http::Response::builder()
                            .status(404)
                            .header("Cache-Control", "no-store")
                            .header(allow_origin.0, allow_origin.1)
                            .body(b"Not Found".to_vec())
                            .unwrap()
                    }
                };
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
                let requested = std::path::Path::new(src_path);
                let src = if requested.is_absolute() {
                    let picked = state.picked_images.lock().unwrap();
                    let pasted = state.pasted_images.lock().unwrap();
                    if !protocol_limits::may_copy_granted(requested, &picked, &pasted) {
                        let shown: String = src_path.chars().take(200).collect();
                        file_ops::log_event(
                            &state,
                            &format!("[COPY] refused {}: not picked nor on clipboard", shown),
                        );
                        return tauri::http::Response::builder()
                            .status(403)
                            .header("Cache-Control", "no-store")
                            .header(allow_origin.0, allow_origin.1)
                            .body(b"Forbidden".to_vec())
                            .unwrap();
                    }
                    requested
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
                        .header(allow_origin.0, allow_origin.1)
                        .body(name.into_bytes())
                        .unwrap();
                }
                return tauri::http::Response::builder()
                    .status(500)
                    .header("Cache-Control", "no-store")
                    .header(allow_origin.0, allow_origin.1)
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
                match protocol_limits::download_image(url) {
                    Err(reason) => {
                        let shown: String = url.chars().take(200).collect();
                        file_ops::log_event(
                            &state,
                            &format!("[DOWNLOAD] refused {}: {}", shown, reason),
                        );
                    }
                    Ok(bytes) => {
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
                                    .header(allow_origin.0, allow_origin.1)
                                    .body(full_path.into_bytes())
                                    .unwrap();
                            }
                        }
                    }
                }
                return tauri::http::Response::builder()
                    .status(500)
                    .header("Cache-Control", "no-store")
                    .header(allow_origin.0, allow_origin.1)
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
                        .header(allow_origin.0, allow_origin.1)
                        .body(bytes)
                        .unwrap(),
                    None => tauri::http::Response::builder()
                        .status(404)
                        .header("Cache-Control", "no-store")
                        .header(allow_origin.0, allow_origin.1)
                        .body(b"Not Found".to_vec())
                        .unwrap(),
                };
            }

            let Some(relative) = protocol_limits::resource_relative(path) else {
                return tauri::http::Response::builder()
                    .status(403)
                    .header(allow_origin.0, allow_origin.1)
                    .body(b"Forbidden".to_vec())
                    .unwrap();
            };
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
                let state = ctx.app_handle().state::<AppState>();
                let all_fonts = temp_layout::fontdata_dir(&state.temp_root).join("AllFonts.js");
                if !protocol_limits::is_generated_font(&all_fonts, abs_path) {
                    return tauri::http::Response::builder()
                        .status(403)
                        .header(allow_origin.0, allow_origin.1)
                        .body(b"Forbidden".to_vec())
                        .unwrap();
                }
                std::fs::read(abs_path).ok()
            } else if decoded_path.ends_with("sdkjs/common/AllFonts.js") {
                let state = ctx.app_handle().state::<AppState>();
                let generated = temp_layout::fontdata_dir(&state.temp_root).join("AllFonts.js");
                std::fs::read(&generated).ok().or_else(|| {
                    read_resource(ctx.app_handle(), &relative)
                })
            } else {
                read_resource(ctx.app_handle(), &relative)
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
                    .header(allow_origin.0, allow_origin.1)
                    .body(data)
                    .unwrap(),
                None => tauri::http::Response::builder()
                    .status(404)
                    .header(allow_origin.0, allow_origin.1)
                    .body(b"Not Found".to_vec())
                    .unwrap(),
            }
        })
        .setup(move |app| {
            // First, so a hidden window is shown as early as the default one.
            #[cfg(target_os = "linux")]
            apply_title_bar(app, title_bar);

            let resource_dir = app.path().resource_dir().unwrap_or_default();
            let binaries_dir = resource_dir.join("binaries");

            if !binaries_dir.is_dir() {
                log_startup("ERROR: bundled binaries directory is missing");
            }

            temp_layout::set_app_version(app.package_info().version.to_string());
            run_font_generation(&temp_root, &instance_id, &binaries_dir);

            // Off the startup path: nothing on screen waits for it.
            let sweep_root = temp_root.clone();
            let sweep_handle = app.handle().clone();
            std::thread::spawn(move || sweep_leftovers(&sweep_handle, &sweep_root));

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
                    let reported = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let eval_window = window.clone();
                    if let Err(e) = window.with_webview(move |webview| {
                        let wv = webview.inner();
                        log_startup("pinch guard installed at GTK level");

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
                                log_startup("touchpad pinch seen at GTK level, event claimed");
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
                        log_startup(&format!("ERROR: pinch guard not installed: {e}"));
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
                let state = app_handle.state::<AppState>();
                recovery::end_session(&state, true);
                state.instance.end();
            }
        });
}

// Folders and logs of instances that are gone. Which folders a crashed
// document still needs is only known from the recovery folders, so without
// them nothing is removed.
fn sweep_leftovers(app: &tauri::AppHandle, root: &std::path::Path) {
    let Ok(recover_root) = recovery::recover_root(app) else {
        return;
    };
    let referenced = recovery::referenced_instances(&recover_root);
    let removed =
        temp_layout::sweep_dead_instances(root, now_ms(), &referenced, temp_layout::is_locked);
    let pruned = temp_layout::prune_logs(root, temp_layout::LOGS_KEPT, |id| {
        temp_layout::instance_alive(root, id)
    });
    if !removed.is_empty() || !pruned.is_empty() {
        log_startup(&format!(
            "Removed {} folders and {} logs of closed instances",
            removed.len(),
            pruned.len()
        ));
    }
}

// The font cache is shared by every instance and slow to build (about 30 s on
// a Raspberry Pi), so an instance that starts while another is building it
// waits for that run, up to this long, instead of building it again.
const FONT_CACHE_WAIT: std::time::Duration = std::time::Duration::from_secs(120);
const FONT_CACHE_POLL: std::time::Duration = std::time::Duration::from_millis(250);

fn run_font_generation(
    temp_root: &std::path::Path,
    instance_id: &str,
    binaries_dir: &std::path::Path,
) {
    let marker = temp_root.join(temp_layout::FONTS_MARKER);
    if marker.exists() && !temp_layout::fonts_ready(temp_root) {
        log_startup("Marker exists but the font cache is missing or stale, regenerating");
    }
    let outcome = temp_layout::ensure_font_cache(
        temp_root,
        instance_id,
        |fontdata_dir: &std::path::Path| generate_fonts(fontdata_dir, binaries_dir),
        FONT_CACHE_WAIT,
        FONT_CACHE_POLL,
    );
    match outcome {
        temp_layout::FontCache::GeneratedByOther => {
            log_startup("Font cache built by another instance")
        }
        temp_layout::FontCache::GaveUp => log_startup(
            "WARNING: another instance is still building the font cache, starting without it",
        ),
        _ => {}
    }
}

// The folder of the running executable: the last place x2t is looked for on
// Linux and macOS (Windows only looks in binaries\).
fn app_exe_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
}

// Runs x2t's font scan into `fontdata_dir` and reports whether it succeeded.
fn generate_fonts(fontdata_dir: &std::path::Path, binaries_dir: &std::path::Path) -> bool {
    x2t_task::generate_fonts(fontdata_dir, binaries_dir, app_exe_dir().as_deref(), &log_startup)
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
