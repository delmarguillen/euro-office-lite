use crate::file_ops::AppState;
use tauri::{State, WebviewWindow};

// The window's title with no document, as tauri.conf.json sets it.
pub const WINDOW_TITLE: &str = "Euro-Office Lite";

// Sets the window's title, and shows it in the title bar tao adds on Wayland.
pub fn set_title(window: &WebviewWindow, title: &str) -> tauri::Result<()> {
    window.set_title(title)?;
    #[cfg(target_os = "linux")]
    show_title_in_header(window, title);
    Ok(())
}

// GTK is only used from the main thread, where the window lives.
#[cfg(target_os = "linux")]
fn show_title_in_header(window: &WebviewWindow, title: &str) {
    let on_main = window.clone();
    let title = title.to_string();
    let _ = window.run_on_main_thread(move || {
        let Ok(gtk_window) = on_main.gtk_window() else {
            return;
        };
        set_header_title(&gtk_window, &title);
    });
}

// tao's title bar on Wayland is a GtkEventBox that holds the GtkHeaderBar
// (tao's wayland/header.rs). GTK copies a new window title to the title bar
// only when the title bar itself is a GtkHeaderBar
// (gtk_window_set_title_internal in gtk/gtkwindow.c), so tao's keeps the title
// the window was built with unless it is set here.
#[cfg(target_os = "linux")]
fn set_header_title(window: &impl gtk::prelude::IsA<gtk::Window>, title: &str) {
    use gtk::prelude::*;
    let Some(header) = header_in_event_box(window) else {
        return;
    };
    header.set_title(Some(title));
}

// None without a title bar, or with one that is not tao's event box: GTK's own
// header bar, which GTK keeps up to date, or another widget.
#[cfg(target_os = "linux")]
fn header_in_event_box(window: &impl gtk::prelude::IsA<gtk::Window>) -> Option<gtk::HeaderBar> {
    use gtk::prelude::*;
    let title_bar = window.titlebar()?;
    let event_box = title_bar.downcast::<gtk::EventBox>().ok()?;
    let child = event_box.child()?;
    child.downcast::<gtk::HeaderBar>().ok()
}

#[tauri::command]
pub fn set_window_title(window: WebviewWindow, name: String) -> Result<(), String> {
    set_title(&window, &format!("{} — {}", WINDOW_TITLE, name)).map_err(|e| e.to_string())
}

// `page`: the number of a Markdown page reloaded with a recovery copy, none
// for any other page (app_quit::report_modified).
#[tauri::command]
pub fn set_document_modified(
    window: WebviewWindow,
    state: State<'_, AppState>,
    modified: bool,
    page: Option<u64>,
) -> Result<(), String> {
    if !crate::app_quit::report_modified(&state, modified, page) {
        return Ok(());
    }
    let current = window.title().map_err(|e| e.to_string())?;
    let base = current.trim_start_matches("● ");
    let title = if modified {
        format!("● {}", base)
    } else {
        base.to_string()
    };
    set_title(&window, &title).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_save_pending(state: State<'_, AppState>, pending: bool) {
    if pending {
        crate::app_quit::begin_save(&state);
        return;
    }
    crate::app_quit::end_save(&state);
}

#[tauri::command]
pub fn force_close(app_handle: tauri::AppHandle) -> Result<(), String> {
    crate::app_quit::discard_after_restore(&app_handle);
    Ok(())
}

#[tauri::command]
pub fn js_log(msg: String) {
    let msg = crate::protocol_limits::truncate_log(&msg, crate::protocol_limits::JS_LOG_MAX_BYTES);
    println!("[JS] {}", msg);
    crate::temp_layout::append_log(msg);
}

// Needs a display, X11 or Wayland: GTK copies the title the same way on both.
// One test, because GTK must stay on the thread that started it.
// Run: xvfb-run cargo test -- --ignored header_title
#[cfg(all(test, target_os = "linux"))]
mod header_title_tests {
    use super::{header_in_event_box, set_header_title, WINDOW_TITLE};
    use gtk::prelude::*;

    const DOCUMENT_TITLE: &str = "● Euro-Office Lite — report.docx";

    fn header_bar() -> gtk::HeaderBar {
        gtk::HeaderBar::builder().title(WINDOW_TITLE).build()
    }

    // The title bar as tao builds it on Wayland.
    fn window_with_boxed_header() -> (gtk::Window, gtk::HeaderBar) {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        let header = header_bar();
        let event_box = gtk::EventBox::new();
        event_box.add(&header);
        window.set_titlebar(Some(&event_box));
        (window, header)
    }

    fn shown(header: &gtk::HeaderBar) -> Option<String> {
        header.title().map(|title| title.to_string())
    }

    #[test]
    #[ignore]
    fn header_title_follows_the_window_title() {
        gtk::init().expect("a display");

        let (window, header) = window_with_boxed_header();
        window.set_title(DOCUMENT_TITLE);
        assert_eq!(
            shown(&header).as_deref(),
            Some(WINDOW_TITLE),
            "GTK alone leaves the boxed header bar as it was"
        );
        set_header_title(&window, DOCUMENT_TITLE);
        assert_eq!(shown(&header).as_deref(), Some(DOCUMENT_TITLE));

        // GTK's own case: it copies the title, so there is nothing to find.
        let plain = gtk::Window::new(gtk::WindowType::Toplevel);
        let plain_header = header_bar();
        plain.set_titlebar(Some(&plain_header));
        plain.set_title(DOCUMENT_TITLE);
        assert_eq!(shown(&plain_header).as_deref(), Some(DOCUMENT_TITLE));
        assert!(header_in_event_box(&plain).is_none());

        // --system-title-bar removes the title bar: nothing to set, no panic.
        let bare = gtk::Window::new(gtk::WindowType::Toplevel);
        assert!(header_in_event_box(&bare).is_none());
        set_header_title(&bare, DOCUMENT_TITLE);
    }
}
