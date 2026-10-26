// Quitting the app (Cmd+Q, the app menu's Quit) goes through the same
// unsaved-changes confirmation as closing the window. On macOS the stock Quit
// item calls NSApp terminate:, which ends the process without any event the
// app can prevent, so the menu is rebuilt with a Quit item of our own.

use tauri::menu::MenuEvent;
use tauri::{AppHandle, Emitter, Manager};

use crate::file_ops::AppState;

pub const QUIT_MENU_ID: &str = "app-quit";

#[derive(Debug, PartialEq, Eq)]
pub enum QuitAction {
    Confirm,
    Exit,
}

pub fn quit_action(modified: bool) -> QuitAction {
    if modified {
        return QuitAction::Confirm;
    }
    QuitAction::Exit
}

pub fn is_quit_item(id: &str) -> bool {
    id == QUIT_MENU_ID
}

// Confirming ends in force_close, which calls exit(); exit() never comes back
// here, so there is no loop.
pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    if !is_quit_item(event.id().as_ref()) {
        return;
    }
    let modified = *app.state::<AppState>().modified.lock().unwrap();
    match quit_action(modified) {
        QuitAction::Confirm => {
            let _ = app.emit("confirm-close", ());
        }
        QuitAction::Exit => app.exit(0),
    }
}

// Tauri's default macOS menu with its last app-menu item (the stock Quit)
// swapped for ours.
#[cfg(target_os = "macos")]
pub fn macos_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem};

    let menu = Menu::default(app)?;
    let Some(first) = menu.items()?.into_iter().next() else {
        return Ok(menu);
    };
    let Some(app_menu) = first.as_submenu() else {
        return Ok(menu);
    };
    let last = app_menu.items()?.len().saturating_sub(1);
    app_menu.remove_at(last)?;
    let label = format!("Quit {}", app.package_info().name);
    let quit = MenuItem::with_id(app, QUIT_MENU_ID, label, true, Some("CmdOrCtrl+Q"))?;
    app_menu.append(&quit)?;
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_document_asks_before_quitting() {
        assert_eq!(quit_action(true), QuitAction::Confirm);
    }

    #[test]
    fn unmodified_document_quits_at_once() {
        assert_eq!(quit_action(false), QuitAction::Exit);
    }

    #[test]
    fn only_our_quit_item_is_handled() {
        assert!(is_quit_item(QUIT_MENU_ID));
        assert!(!is_quit_item("quit"));
        assert!(!is_quit_item(""));
    }
}
