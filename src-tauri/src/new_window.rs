// Opens a document in a second window: a new process of this app with the
// document as its launch argument, as a double click on the file does. Each
// process has its own instance folder (temp_layout.rs), so they do not share
// any working files.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tauri::State;

use crate::file_ops::{log_event, AppState};
use crate::launch_args::NEW_WINDOW;
use crate::path_grants::{GrantKind, PathGrants};
use crate::temp_layout::DocumentClaim;

// What the bridge gets back when nothing went wrong.
pub const OPENED: &str = "opened";
pub const ALREADY_OPEN: &str = "already-open";
// The file is open in another window: what open_in_new_window returns, and the
// error open_file and open_pdf refuse with, so two windows never save it.
pub const OPEN_ELSEWHERE: &str = "open-elsewhere";

#[derive(Debug, PartialEq, Eq)]
pub struct NewWindowLaunch {
    pub document: PathBuf,
    // Everything after the program name: the title bar options, NEW_WINDOW,
    // then the path.
    pub args: Vec<OsString>,
}

// `title_bar_args` are the title bar options this process was started with,
// so the new window looks the same. The path goes last and is absolute, so the
// new process never reads it as an option or against another directory.
pub fn new_window_launch(
    grants: &PathGrants,
    token: &str,
    title_bar_args: &[&str],
) -> Result<NewWindowLaunch, String> {
    let document = grants.resolve(token, GrantKind::Open)?;
    if !document.is_absolute() {
        return Err("The file to open in a new window has no absolute path".to_string());
    }
    let mut args: Vec<OsString> = title_bar_args.iter().map(OsString::from).collect();
    args.push(OsString::from(NEW_WINDOW));
    args.push(document.clone().into_os_string());
    Ok(NewWindowLaunch { document, args })
}

// Both paths are resolved by the file system when they can be, so a symlink,
// a ".." or (on Windows) another letter case still names the same file, and
// so do two document portal paths of one host file (temp_layout.rs).
pub fn is_open_document(chosen: &Path, current: Option<&Path>) -> bool {
    let Some(current) = current else {
        return false;
    };
    let identities = (
        crate::temp_layout::document_identity(chosen),
        crate::temp_layout::document_identity(current),
    );
    match identities {
        (Some(chosen), Some(current)) => chosen == current,
        _ => chosen == current,
    }
}

pub fn is_open_elsewhere(state: &AppState, document: &Path) -> bool {
    crate::temp_layout::open_in_another_instance(&state.temp_root, &state.instance.id, document)
}

// Err(OPEN_ELSEWHERE) when a live instance other than `own_id` has `document`
// open, or is opening or saving to it. Otherwise the other windows see it as
// open until the claim is dropped, which the caller does once open-document
// names the file or the open or save failed. A file that does not exist yet
// is open nowhere.
pub fn claim_document(root: &Path, own_id: &str, document: &Path) -> Result<DocumentClaim, String> {
    let Some(claim) = crate::temp_layout::claim_document(root, own_id, document) else {
        return Err(OPEN_ELSEWHERE.to_string());
    };
    Ok(claim)
}

// For the commands that open a file in this window.
pub fn claim_for_open(state: &AppState, document: &Path) -> Result<DocumentClaim, String> {
    let claimed = claim_document(&state.temp_root, &state.instance.id, document);
    if claimed.is_err() {
        log_event(state, "[OPEN] the file is open in another window: not opened");
    }
    claimed
}

// For the commands that write a file chosen in a dialog. The other window keeps
// its own copy and would overwrite this write on its next save.
pub fn claim_for_save(state: &AppState, dest: &Path) -> Result<DocumentClaim, String> {
    let claimed = claim_document(&state.temp_root, &state.instance.id, dest);
    if claimed.is_err() {
        log_event(state, "[SAVE] the destination is open in another window: not saved");
    }
    claimed
}

// Starts this executable without a shell and does not wait for it. A thread
// waits instead, so the finished process does not stay behind as a zombie.
fn spawn_this_app(args: &[OsString]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[tauri::command]
pub fn open_in_new_window(state: State<'_, AppState>, token: String) -> Result<String, String> {
    let launch = new_window_launch(&state.grants, &token, &state.title_bar_args)?;
    let current = state.current_file.lock().unwrap().clone();
    if is_open_document(&launch.document, current.as_deref()) {
        log_event(&state, "[NEW WINDOW] the chosen file is the open document: nothing to do");
        return Ok(ALREADY_OPEN.to_string());
    }
    if is_open_elsewhere(&state, &launch.document) {
        log_event(&state, "[NEW WINDOW] the chosen file is open in another window: nothing to do");
        return Ok(OPEN_ELSEWHERE.to_string());
    }
    spawn_this_app(&launch.args).map_err(|e| {
        log_event(&state, &format!("[NEW WINDOW] launch failed: {e}"));
        e
    })?;
    log_event(&state, "[NEW WINDOW] launched");
    Ok(OPENED.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch_args::{NO_TITLE_BAR, SYSTEM_TITLE_BAR};

    fn absolute_document() -> PathBuf {
        std::env::temp_dir().join("report.docx")
    }

    #[test]
    fn an_open_token_gives_the_options_new_window_then_the_absolute_path() {
        let grants = PathGrants::default();
        let file = grants.grant(absolute_document(), GrantKind::Open);
        let launch =
            new_window_launch(&grants, &file.token, &[SYSTEM_TITLE_BAR, NO_TITLE_BAR]).unwrap();
        assert_eq!(launch.document, absolute_document());
        assert_eq!(
            launch.args,
            vec![
                OsString::from(SYSTEM_TITLE_BAR),
                OsString::from(NO_TITLE_BAR),
                OsString::from(NEW_WINDOW),
                absolute_document().into_os_string(),
            ]
        );
    }

    #[test]
    fn without_title_bar_options_new_window_and_the_path_are_the_arguments() {
        let grants = PathGrants::default();
        let file = grants.grant(absolute_document(), GrantKind::Open);
        let launch = new_window_launch(&grants, &file.token, &[]).unwrap();
        assert_eq!(
            launch.args,
            vec![OsString::from(NEW_WINDOW), absolute_document().into_os_string()]
        );
    }

    #[test]
    fn a_save_token_is_refused() {
        let grants = PathGrants::default();
        let file = grants.grant(absolute_document(), GrantKind::Save);
        assert!(new_window_launch(&grants, &file.token, &[]).is_err());
    }

    #[test]
    fn an_unknown_token_is_refused() {
        let grants = PathGrants::default();
        grants.grant(absolute_document(), GrantKind::Open);
        assert!(new_window_launch(&grants, "0123456789abcdef0123456789abcdef", &[]).is_err());
    }

    #[test]
    fn a_relative_path_is_refused() {
        let grants = PathGrants::default();
        let file = grants.grant(PathBuf::from("report.docx"), GrantKind::Open);
        assert!(new_window_launch(&grants, &file.token, &[]).is_err());
    }

    #[test]
    fn the_same_file_by_another_spelling_is_the_open_document() {
        let dir = std::env::temp_dir().join(format!("eo-new-window-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("a.docx");
        std::fs::write(&file, b"x").unwrap();
        let other = dir.join("b.docx");
        std::fs::write(&other, b"x").unwrap();
        let roundabout = dir.join("sub").join("..").join("a.docx");

        assert!(is_open_document(&roundabout, Some(&file)));
        assert!(!is_open_document(&other, Some(&file)));
        assert!(!is_open_document(&file, None));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn scratch_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("eo-save-dest-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn document_in(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::write(&path, b"x").unwrap();
        path
    }

    #[test]
    fn a_save_over_a_file_open_in_another_window_is_refused() {
        let root = scratch_root("other");
        let file = document_in(&root, "a.docx");
        let own = crate::temp_layout::begin_instance(&root, 1000, 1).unwrap();
        let other = crate::temp_layout::begin_instance(&root, 1000, 2).unwrap();
        crate::temp_layout::publish_open_document(&other.dir, Some(&file)).unwrap();

        let claimed = claim_document(&root, &own.id, &file);

        assert_eq!(claimed.err(), Some(OPEN_ELSEWHERE.to_string()));
        drop((own, other));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_save_over_the_file_open_in_this_window_is_allowed() {
        let root = scratch_root("own");
        let file = document_in(&root, "a.docx");
        let own = crate::temp_layout::begin_instance(&root, 1000, 1).unwrap();
        crate::temp_layout::publish_open_document(&own.dir, Some(&file)).unwrap();

        assert!(claim_document(&root, &own.id, &file).is_ok());
        drop(own);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_save_to_another_file_or_a_new_one_is_allowed() {
        let root = scratch_root("free");
        let open = document_in(&root, "a.docx");
        let free = document_in(&root, "b.docx");
        let new = root.join("c.docx");
        let own = crate::temp_layout::begin_instance(&root, 1000, 1).unwrap();
        let other = crate::temp_layout::begin_instance(&root, 1000, 2).unwrap();
        crate::temp_layout::publish_open_document(&other.dir, Some(&open)).unwrap();

        assert!(claim_document(&root, &own.id, &free).is_ok());
        assert!(claim_document(&root, &own.id, &new).is_ok());
        drop((own, other));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn window(root: &Path, pid: u32) -> AppState {
        let instance = crate::temp_layout::begin_instance(root, 1000, pid).unwrap();
        crate::file_ops::test_state_for(root, instance)
    }

    // What save_file_as does once x2t has written the file.
    fn finish_save(state: &AppState, dest: &Path, claim: DocumentClaim) {
        crate::file_ops::set_current_file(state, Some(dest.to_path_buf()));
        drop(claim);
    }

    #[test]
    fn a_file_another_window_is_saving_to_does_not_open() {
        let root = scratch_root("saving");
        let dest = document_in(&root, "a.docx");
        let saving = window(&root, 1);
        let opening = window(&root, 2);

        let save = claim_for_save(&saving, &dest).unwrap();
        assert_eq!(claim_for_open(&opening, &dest).err(), Some(OPEN_ELSEWHERE.to_string()));
        finish_save(&saving, &dest, save);
        assert_eq!(claim_for_open(&opening, &dest).err(), Some(OPEN_ELSEWHERE.to_string()));

        drop((saving, opening));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_another_window_is_opening_is_not_saved_over() {
        let root = scratch_root("opening");
        let dest = document_in(&root, "a.docx");
        let opening = window(&root, 1);
        let saving = window(&root, 2);

        let open = claim_for_open(&opening, &dest).unwrap();
        assert_eq!(claim_for_save(&saving, &dest).err(), Some(OPEN_ELSEWHERE.to_string()));
        drop(open);

        drop((saving, opening));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_save_over_the_own_file_and_to_a_new_one_leaves_it_open_here() {
        let root = scratch_root("own-and-new");
        let own_file = document_in(&root, "a.docx");
        let new_file = root.join("b.docx");
        let saving = window(&root, 1);
        let other = window(&root, 2);
        crate::file_ops::set_current_file(&saving, Some(own_file.clone()));

        let over_own = claim_for_save(&saving, &own_file).unwrap();
        finish_save(&saving, &own_file, over_own);
        let to_new = claim_for_save(&saving, &new_file).unwrap();
        // x2t's output renamed into place.
        std::fs::write(&new_file, b"x").unwrap();
        finish_save(&saving, &new_file, to_new);

        assert_eq!(claim_for_open(&other, &new_file).err(), Some(OPEN_ELSEWHERE.to_string()));
        assert!(claim_for_open(&other, &own_file).is_ok());
        drop((saving, other));
        let _ = std::fs::remove_dir_all(&root);
    }
}
