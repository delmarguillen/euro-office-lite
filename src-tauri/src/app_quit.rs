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

// Confirming ends in force_close, which exits without asking again; exit()
// never comes back here, so there is no loop.
pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    if !is_quit_item(event.id().as_ref()) {
        return;
    }
    let modified = *app.state::<AppState>().modified.lock().unwrap();
    match quit_action(modified) {
        QuitAction::Confirm => {
            let _ = app.emit("confirm-close", ());
        }
        QuitAction::Exit => quit_after_restore(app),
    }
}

// An exit the user confirmed: there were no changes, or they chose to discard
// them. Clearing the flag is what tells on_exit to delete the recovery folder.
pub fn mark_exit_confirmed(state: &AppState) {
    *state.modified.lock().unwrap() = false;
}

// The only exit(0) of the app: every confirmed exit goes through here.
pub fn exit_confirmed(app: &AppHandle) {
    mark_exit_confirmed(&app.state::<AppState>());
    app.exit(0);
}

// An exit nobody confirmed still has unsaved changes: on macOS, Quit from the
// Dock, logging out and shutting down end the process without asking (tao does
// not implement applicationShouldTerminate:). Their recovery folder stays, so
// the next start offers it.
pub fn discard_recovery_on_exit(modified: bool) -> bool {
    !modified
}

// The document's images are in the instance folder (media/), and a kept
// recovery folder is recovered with them, so the instance folder stays with it.
// The OS drops its lock when the process ends; the next start's sweep keeps it
// while a recovery folder cites it (temp_layout::sweep_dead_instances).
pub fn remove_instance_on_exit(recovery_discarded: bool) -> bool {
    recovery_discarded
}

// RunEvent::Exit: the one point every exit that runs the event loop's end
// passes through. A killed process never gets here; its folder stays too.
pub fn on_exit(state: &AppState) {
    let modified = *state.modified.lock().unwrap();
    let discard = discard_recovery_on_exit(modified);
    crate::recovery::end_session(state, discard);
    if !remove_instance_on_exit(discard) {
        return;
    }
    state.instance.end();
}

// The slideshow window is restored first so the window-state plugin saves its
// normal size on RunEvent::Exit. A quit checks for changes again once
// restored; a discard does not.
fn quit_after_restore(app: &AppHandle) {
    let Some(window) = app.get_webview_window(crate::window_state::MAIN_WINDOW) else {
        exit_confirmed(app);
        return;
    };
    let state = app.state::<crate::presentation::PresentationState>();
    crate::presentation::quit_when_restored(&window, &state);
}

pub fn discard_after_restore(app: &AppHandle) {
    let Some(window) = app.get_webview_window(crate::window_state::MAIN_WINDOW) else {
        exit_confirmed(app);
        return;
    };
    let state = app.state::<crate::presentation::PresentationState>();
    crate::presentation::discard_when_restored(&window, &state);
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
    use crate::file_ops::test_state;
    use crate::recovery::{changes_path, scan_candidates, write_changes_file, RecoverySession};
    use crate::temp_layout::{is_locked, take_lock};
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eo-app-quit-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    // A live session the way recovery_begin and save_changes leave it: the
    // opened document, one change past the last save and the lock held.
    fn live_session(state: &AppState, recover_root: &Path) -> PathBuf {
        let dir = recover_root.join("1000-1");
        std::fs::create_dir_all(dir.join("changes")).unwrap();
        std::fs::write(dir.join("Editor.bin"), b"DOCY;v10").unwrap();
        let entries = write_changes_file(&changes_path(&dir), &["AAAA".to_string()]).unwrap();
        let lock = take_lock(&dir.join("rec.lock")).unwrap();
        *state.recovery.lock().unwrap() = Some(RecoverySession {
            dir: dir.clone(),
            _lock: lock,
            base: 0,
            entries,
            saved_count: 0,
            capped: false,
        });
        dir
    }

    const IMAGE_NAME: &str = "image1.png";

    // An image inserted into the open document, where copy-to-media puts it.
    fn insert_image(state: &AppState) -> PathBuf {
        let media = state.temp_dir.join("media");
        std::fs::create_dir_all(&media).unwrap();
        let image = media.join(IMAGE_NAME);
        std::fs::write(&image, b"\x89PNG").unwrap();
        image
    }

    #[test]
    fn an_exit_with_changes_keeps_the_recovery_folder() {
        assert!(!discard_recovery_on_exit(true));
    }

    #[test]
    fn an_exit_without_changes_deletes_the_recovery_folder() {
        assert!(discard_recovery_on_exit(false));
    }

    #[test]
    fn a_discarded_recovery_removes_the_instance_folder() {
        assert!(remove_instance_on_exit(true));
    }

    #[test]
    fn a_kept_recovery_keeps_the_instance_folder() {
        assert!(!remove_instance_on_exit(false));
    }

    #[test]
    fn an_exit_with_changes_keeps_the_instance_folder_and_its_images() {
        let root = scratch("instance-kept");
        let state = test_state(&root);
        live_session(&state, &root.join("recover"));
        let image = insert_image(&state);
        *state.modified.lock().unwrap() = true;

        on_exit(&state);

        assert!(image.exists());
    }

    #[test]
    fn an_exit_without_changes_removes_the_instance_folder() {
        let root = scratch("instance-removed");
        let state = test_state(&root);
        live_session(&state, &root.join("recover"));
        insert_image(&state);

        on_exit(&state);

        assert!(!state.instance.dir.exists());
    }

    #[test]
    fn a_confirmed_exit_clears_the_modified_flag() {
        let state = test_state(&scratch("confirmed-flag"));
        *state.modified.lock().unwrap() = true;
        mark_exit_confirmed(&state);
        assert!(!*state.modified.lock().unwrap());
    }

    #[test]
    fn an_unconfirmed_exit_leaves_a_folder_the_next_start_offers() {
        let root = scratch("unconfirmed");
        let recover_root = root.join("recover");
        let state = test_state(&root);
        let dir = live_session(&state, &recover_root);
        *state.modified.lock().unwrap() = true;

        on_exit(&state);

        assert!(dir.exists());
        let candidates = scan_candidates(&recover_root, now_ms(), |path| is_locked(path));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].changes, 1);
    }

    #[test]
    fn a_confirmed_exit_deletes_the_recovery_folder() {
        let root = scratch("confirmed");
        let state = test_state(&root);
        let dir = live_session(&state, &root.join("recover"));
        *state.modified.lock().unwrap() = true;

        mark_exit_confirmed(&state);
        on_exit(&state);

        assert!(!dir.exists());
    }

    // force_close, the menu's Quit and anything added later must clear the
    // flag before exiting, so none of them may call exit() on its own.
    #[test]
    fn exit_confirmed_holds_the_only_exit_call() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files_with_exit = Vec::new();
        for entry in std::fs::read_dir(&src).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Every module keeps its tests at the end: this one's needle is
            // not a call.
            let code = text.split("#[cfg(test)]").next().unwrap();
            let calls = code.matches(".exit(").count();
            if calls > 0 {
                let name = entry.file_name().to_string_lossy().to_string();
                files_with_exit.push((name, calls));
            }
        }
        assert_eq!(files_with_exit, vec![("app_quit.rs".to_string(), 1)]);
    }

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
