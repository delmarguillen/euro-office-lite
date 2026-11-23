// The folders the file dialogs start in, set in the start screen's Settings:
// one for Open and Save As, one for New from template. A folder set here only
// moves where a dialog starts. It gives the webview no access to the folder or
// to anything in it: a file is still reached only with the token a dialog
// hands out for it (path_grants.rs).
//
// Where each dialog starts:
//   Save As, the document has a file       its file's folder
//   Save As, the document has no file yet  the documents folder
//   Open, Edit PDF                         the documents folder
//   New from template                      the templates folder
// A folder that is not set, is missing or cannot be read leaves the dialog
// where the system puts it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

// In app_config_dir(), next to recent-files.json.
const STORE_FILE: &str = "default-folders.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderKind {
    Documents,
    Templates,
}

const FOLDER_KINDS: [FolderKind; 2] = [FolderKind::Documents, FolderKind::Templates];

impl FolderKind {
    // The name the webview sends and the log shows.
    fn name(self) -> &'static str {
        match self {
            FolderKind::Documents => "documents",
            FolderKind::Templates => "templates",
        }
    }

    fn from_name(name: &str) -> Result<Self, String> {
        FOLDER_KINDS
            .into_iter()
            .find(|kind| kind.name() == name)
            .ok_or_else(|| format!("Unknown folder setting: {name}"))
    }
}

// What the store file holds and the Settings panel shows. None: no folder is
// set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultFolders {
    documents: Option<String>,
    templates: Option<String>,
}

impl DefaultFolders {
    fn get(&self, kind: FolderKind) -> Option<&Path> {
        let folder = match kind {
            FolderKind::Documents => &self.documents,
            FolderKind::Templates => &self.templates,
        };
        folder.as_deref().map(Path::new)
    }

    fn set(&mut self, kind: FolderKind, folder: Option<String>) {
        match kind {
            FolderKind::Documents => self.documents = folder,
            FolderKind::Templates => self.templates = folder,
        }
    }
}

// A dialog that can start in a folder from Settings.
#[derive(Clone, Copy, Debug)]
pub enum Dialog<'a> {
    // Open and Edit PDF.
    Open,
    // New from template.
    Template,
    // Save As and Export. `own` is the file the document already has: the open
    // document's, or the one a recovered document was saved to before.
    SaveAs { own: Option<&'a Path> },
}

impl Dialog<'_> {
    fn folder_kind(self) -> FolderKind {
        match self {
            Dialog::Open | Dialog::SaveAs { .. } => FolderKind::Documents,
            Dialog::Template => FolderKind::Templates,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StartFolder {
    // Nothing decides: the dialog starts where the system puts it.
    System,
    Folder(PathBuf),
    // The folder set in Settings is missing or cannot be read. The dialog
    // starts where the system puts it and the setting stays: the folder may be
    // on a drive that is not connected.
    Unavailable(PathBuf),
}

// Where `dialog` starts. `is_readable` is asked only about a folder from
// Settings: a document's own folder is used as it is.
pub fn start_folder(
    dialog: Dialog,
    folders: &DefaultFolders,
    is_readable: impl Fn(&Path) -> bool,
) -> StartFolder {
    if let Dialog::SaveAs { own: Some(file) } = dialog {
        return match file.parent() {
            Some(folder) => StartFolder::Folder(folder.to_path_buf()),
            None => StartFolder::System,
        };
    }
    let Some(folder) = folders.get(dialog.folder_kind()) else {
        return StartFolder::System;
    };
    if !is_readable(folder) {
        return StartFolder::Unavailable(folder.to_path_buf());
    }
    StartFolder::Folder(folder.to_path_buf())
}

fn is_readable_folder(folder: &Path) -> bool {
    std::fs::read_dir(folder).is_ok()
}

// The folder to give the dialog, if any.
pub fn dialog_start(app: &AppHandle, dialog: Dialog) -> Option<PathBuf> {
    match start_folder(dialog, &load(app), is_readable_folder) {
        StartFolder::Folder(folder) => Some(folder),
        StartFolder::System => None,
        StartFolder::Unavailable(folder) => {
            log(&format!(
                "{} folder not available, the dialog starts without it: {}",
                dialog.folder_kind().name(),
                folder.display()
            ));
            None
        }
    }
}

fn log(line: &str) {
    crate::temp_layout::append_log(&format!("[folders] {line}"));
}

// The folders in the store's text. One that is not an absolute path is not
// used: a relative path names a different folder each time the app starts
// somewhere else.
fn parse_store(raw: &str) -> Result<DefaultFolders, String> {
    let mut folders: DefaultFolders = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    for kind in FOLDER_KINDS {
        let Some(folder) = folders.get(kind) else {
            continue;
        };
        if folder.is_absolute() {
            continue;
        }
        log(&format!(
            "{} folder ignored, not an absolute path: {}",
            kind.name(),
            folder.display()
        ));
        folders.set(kind, None);
    }
    Ok(folders)
}

// A missing store is no folder set. One that cannot be read or is not valid
// is logged and counts the same, so it never keeps a dialog from opening.
fn load_from(store: &Path) -> DefaultFolders {
    let raw = match std::fs::read_to_string(store) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return DefaultFolders::default();
        }
        Err(error) => {
            log(&format!("settings not read: {error}"));
            return DefaultFolders::default();
        }
    };
    match parse_store(&raw) {
        Ok(folders) => folders,
        Err(error) => {
            log(&format!("settings ignored, not valid: {error}"));
            DefaultFolders::default()
        }
    }
}

// Written through a temporary file, so a crash mid-write leaves the store as
// it was.
fn save_to(store: &Path, folders: &DefaultFolders) -> Result<(), String> {
    if let Some(parent) = store.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(folders).map_err(|e| e.to_string())?;
    let tmp = store.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes()).map_err(|e| e.to_string())?;
    if let Err(error) = std::fs::rename(&tmp, store) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error.to_string());
    }
    Ok(())
}

// The text the store keeps for a folder.
fn stored_folder(folder: &Path) -> Result<String, String> {
    if !folder.is_absolute() {
        return Err("The folder is not an absolute path".to_string());
    }
    let Some(text) = folder.to_str() else {
        return Err("The folder's path is not valid Unicode".to_string());
    };
    Ok(text.to_string())
}

// Sets the folder for `kind`, or clears it with None. Returns what the store
// holds afterwards.
fn set_folder(
    store: &Path,
    kind: FolderKind,
    folder: Option<&Path>,
) -> Result<DefaultFolders, String> {
    let folder = folder.map(stored_folder).transpose()?;
    let mut folders = load_from(store);
    folders.set(kind, folder);
    save_to(store, &folders)?;
    Ok(folders)
}

fn store_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(STORE_FILE))
}

fn load(app: &AppHandle) -> DefaultFolders {
    match store_path(app) {
        Ok(store) => load_from(&store),
        Err(_) => DefaultFolders::default(),
    }
}

// What the Settings panel shows.
#[tauri::command]
pub async fn default_folders_state(app: AppHandle) -> DefaultFolders {
    load(&app)
}

// The Settings panel's Choose: the folder comes from the system's folder
// dialog, opened here, and never from the webview. Cancelling changes nothing.
#[tauri::command]
pub async fn pick_default_folder(
    window: tauri::WebviewWindow,
    kind: String,
    title: String,
) -> Result<DefaultFolders, String> {
    let kind = FolderKind::from_name(&kind)?;
    let store = store_path(window.app_handle())?;
    let folders = load_from(&store);
    let mut dialog = window.dialog().file().set_parent(&window).set_title(title);
    if let Some(current) = folders.get(kind).filter(|folder| is_readable_folder(folder)) {
        dialog = dialog.set_directory(current);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    let Some(folder) = picked.and_then(|choice| choice.into_path().ok()) else {
        return Ok(folders);
    };
    set_folder(&store, kind, Some(&folder))
}

// The Settings panel's Reset: back to no folder set.
#[tauri::command]
pub async fn clear_default_folder(app: AppHandle, kind: String) -> Result<DefaultFolders, String> {
    let kind = FolderKind::from_name(&kind)?;
    set_folder(&store_path(&app)?, kind, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_grants::{GrantKind, PathGrants};

    const DOCUMENTS: &str = "/home/u/Documents";
    const TEMPLATES: &str = "/home/u/Templates";

    fn both_set() -> DefaultFolders {
        DefaultFolders {
            documents: Some(DOCUMENTS.to_string()),
            templates: Some(TEMPLATES.to_string()),
        }
    }

    fn every_folder_is_readable(_: &Path) -> bool {
        true
    }

    fn no_folder_is_readable(_: &Path) -> bool {
        false
    }

    fn folder(path: &str) -> StartFolder {
        StartFolder::Folder(PathBuf::from(path))
    }

    fn unavailable(path: &str) -> StartFolder {
        StartFolder::Unavailable(PathBuf::from(path))
    }

    fn new_document() -> Dialog<'static> {
        Dialog::SaveAs { own: None }
    }

    fn temp_dir_for(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eo-folders-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn text(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn save_as_of_a_document_with_a_file_starts_in_its_own_folder() {
        let own = Path::new("/home/u/Work/Budget.pdf");
        let dialog = Dialog::SaveAs { own: Some(own) };
        let not_asked = |_: &Path| -> bool { panic!("a document's own folder is not checked") };
        assert_eq!(start_folder(dialog, &both_set(), not_asked), folder("/home/u/Work"));
        assert_eq!(
            start_folder(dialog, &DefaultFolders::default(), not_asked),
            folder("/home/u/Work")
        );
    }

    #[test]
    fn save_as_of_a_document_with_no_file_starts_in_the_documents_folder() {
        assert_eq!(
            start_folder(new_document(), &both_set(), every_folder_is_readable),
            folder(DOCUMENTS)
        );
    }

    #[test]
    fn open_starts_in_the_documents_folder() {
        assert_eq!(
            start_folder(Dialog::Open, &both_set(), every_folder_is_readable),
            folder(DOCUMENTS)
        );
    }

    #[test]
    fn new_from_template_starts_in_the_templates_folder() {
        assert_eq!(
            start_folder(Dialog::Template, &both_set(), every_folder_is_readable),
            folder(TEMPLATES)
        );
    }

    #[test]
    fn each_folder_is_only_for_its_own_dialogs() {
        let only_documents = DefaultFolders {
            documents: Some(DOCUMENTS.to_string()),
            templates: None,
        };
        assert_eq!(
            start_folder(Dialog::Template, &only_documents, every_folder_is_readable),
            StartFolder::System
        );
        let only_templates = DefaultFolders {
            documents: None,
            templates: Some(TEMPLATES.to_string()),
        };
        for dialog in [Dialog::Open, new_document()] {
            assert_eq!(
                start_folder(dialog, &only_templates, every_folder_is_readable),
                StartFolder::System,
                "{dialog:?}"
            );
        }
    }

    #[test]
    fn with_no_folder_set_the_system_decides() {
        let not_asked = |_: &Path| -> bool { panic!("there is no folder to check") };
        for dialog in [Dialog::Open, Dialog::Template, new_document()] {
            assert_eq!(
                start_folder(dialog, &DefaultFolders::default(), not_asked),
                StartFolder::System,
                "{dialog:?}"
            );
        }
    }

    #[test]
    fn a_folder_that_cannot_be_read_leaves_the_dialog_to_the_system() {
        let cases = [
            (Dialog::Open, DOCUMENTS),
            (new_document(), DOCUMENTS),
            (Dialog::Template, TEMPLATES),
        ];
        for (dialog, set) in cases {
            assert_eq!(
                start_folder(dialog, &both_set(), no_folder_is_readable),
                unavailable(set),
                "{dialog:?}"
            );
        }
    }

    #[test]
    fn a_file_at_the_root_has_no_folder_of_its_own() {
        let dialog = Dialog::SaveAs { own: Some(Path::new("/")) };
        assert_eq!(
            start_folder(dialog, &both_set(), every_folder_is_readable),
            StartFolder::System
        );
    }

    #[test]
    fn only_a_folder_that_is_there_is_readable() {
        let dir = temp_dir_for("readable");
        std::fs::write(dir.join("Letter.docx"), b"a file").unwrap();

        assert!(is_readable_folder(&dir));
        assert!(!is_readable_folder(&dir.join("Missing")));
        assert!(!is_readable_folder(&dir.join("Letter.docx")));
        assert!(!is_readable_folder(Path::new("")));
        assert!(!is_readable_folder(Path::new("/home/u/Doc\0uments")));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_that_is_gone_is_not_used_and_stays_set() {
        let dir = temp_dir_for("gone");
        let store = dir.join(STORE_FILE);
        let external = dir.join("External");
        std::fs::create_dir_all(&external).unwrap();
        set_folder(&store, FolderKind::Documents, Some(&external)).unwrap();
        std::fs::remove_dir_all(&external).unwrap();

        let folders = load_from(&store);
        assert_eq!(
            start_folder(Dialog::Open, &folders, is_readable_folder),
            StartFolder::Unavailable(external.clone())
        );
        assert_eq!(load_from(&store).documents, Some(text(&external)));

        std::fs::create_dir_all(&external).unwrap();
        assert_eq!(
            start_folder(Dialog::Open, &load_from(&store), is_readable_folder),
            StartFolder::Folder(external)
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_store_is_no_folder_set() {
        let dir = temp_dir_for("missing");
        assert_eq!(load_from(&dir.join(STORE_FILE)), DefaultFolders::default());
        assert_eq!(
            load_from(&dir.join("no-such-folder").join(STORE_FILE)),
            DefaultFolders::default()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_or_corrupt_store_is_no_folder_set() {
        let dir = temp_dir_for("corrupt");
        let store = dir.join(STORE_FILE);
        let contents: [&[u8]; 9] = [
            b"",
            b"  \n",
            b"{",
            b"null",
            b"[]",
            b"\"/home/u/Documents\"",
            b"{\"documents\": 5}",
            b"{\"documents\": [\"/home/u/Documents\"]}",
            b"\xff\xfe\x00not text",
        ];
        for content in contents {
            std::fs::write(&store, content).unwrap();
            assert_eq!(load_from(&store), DefaultFolders::default(), "{content:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_store_does_not_keep_a_folder_from_being_set() {
        let dir = temp_dir_for("corrupt-set");
        let store = dir.join(STORE_FILE);
        std::fs::write(&store, b"{ not json").unwrap();

        let folders = set_folder(&store, FolderKind::Templates, Some(&dir)).unwrap();
        assert_eq!(folders.templates, Some(text(&dir)));
        assert_eq!(folders.documents, None);
        assert_eq!(load_from(&store), folders);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_that_is_not_an_absolute_path_is_not_used() {
        let absolute = text(&std::env::temp_dir());
        for odd in ["", ".", "..", "Documents", "./Documents", "../Documents", "~/Documents"] {
            let raw = serde_json::json!({ "documents": odd, "templates": absolute }).to_string();
            let folders = parse_store(&raw).unwrap();
            assert_eq!(folders.documents, None, "{odd:?}");
            assert_eq!(folders.templates, Some(absolute.clone()), "{odd:?}");
        }
    }

    #[test]
    fn the_store_reads_null_and_unknown_entries() {
        let absolute = text(&std::env::temp_dir());
        let raw = serde_json::json!({ "documents": null, "templates": absolute, "other": 1 });
        let folders = parse_store(&raw.to_string()).unwrap();
        assert_eq!(folders.documents, None);
        assert_eq!(folders.templates, Some(absolute));
        assert_eq!(parse_store("{}").unwrap(), DefaultFolders::default());
    }

    #[test]
    fn a_folder_set_is_read_back_and_the_other_one_is_kept() {
        let dir = temp_dir_for("round-trip");
        let store = dir.join("config").join(STORE_FILE);
        let documents = dir.join("Docs with spaces");
        let templates = dir.join("Plantillas ñ 文档");

        set_folder(&store, FolderKind::Documents, Some(&documents)).unwrap();
        let folders = set_folder(&store, FolderKind::Templates, Some(&templates)).unwrap();
        assert_eq!(folders.documents, Some(text(&documents)));
        assert_eq!(folders.templates, Some(text(&templates)));
        assert_eq!(load_from(&store), folders);
        assert!(!store.with_extension("json.tmp").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clearing_a_folder_keeps_the_other_one() {
        let dir = temp_dir_for("clear");
        let store = dir.join(STORE_FILE);
        set_folder(&store, FolderKind::Documents, Some(&dir)).unwrap();
        set_folder(&store, FolderKind::Templates, Some(&dir)).unwrap();

        let folders = set_folder(&store, FolderKind::Documents, None).unwrap();
        assert_eq!(folders.documents, None);
        assert_eq!(folders.templates, Some(text(&dir)));
        assert_eq!(load_from(&store), folders);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clearing_with_no_store_sets_nothing() {
        let dir = temp_dir_for("clear-missing");
        let store = dir.join(STORE_FILE);
        let folders = set_folder(&store, FolderKind::Templates, None).unwrap();
        assert_eq!(folders, DefaultFolders::default());
        assert_eq!(load_from(&store), DefaultFolders::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_relative_folder_is_refused_and_the_store_is_left_alone() {
        let dir = temp_dir_for("relative");
        let store = dir.join(STORE_FILE);
        let before = set_folder(&store, FolderKind::Documents, Some(&dir)).unwrap();

        for odd in ["", "Documents", "../Documents"] {
            let result = set_folder(&store, FolderKind::Documents, Some(Path::new(odd)));
            assert!(result.is_err(), "{odd:?}");
        }
        assert_eq!(load_from(&store), before);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_whose_path_is_not_unicode_is_refused() {
        use std::os::unix::ffi::OsStrExt;
        let odd = Path::new(std::ffi::OsStr::from_bytes(b"/home/u/Doc\xff"));
        assert!(stored_folder(odd).is_err());
    }

    #[test]
    fn the_webview_names_a_folder_setting_by_its_kind() {
        assert_eq!(FolderKind::from_name("documents"), Ok(FolderKind::Documents));
        assert_eq!(FolderKind::from_name("templates"), Ok(FolderKind::Templates));
        for odd in ["", "Documents", "document", "/home/u", "templates "] {
            assert!(FolderKind::from_name(odd).is_err(), "{odd:?}");
        }
    }

    #[test]
    fn the_webview_gets_both_folders_by_name() {
        let shown = serde_json::to_value(both_set()).unwrap();
        assert_eq!(
            shown,
            serde_json::json!({ "documents": DOCUMENTS, "templates": TEMPLATES })
        );
        let none = serde_json::to_value(DefaultFolders::default()).unwrap();
        assert_eq!(none, serde_json::json!({ "documents": null, "templates": null }));
    }

    #[test]
    fn setting_a_folder_grants_no_file() {
        let dir = temp_dir_for("grants");
        let store = dir.join(STORE_FILE);
        let inside = dir.join("Budget.xlsx");
        std::fs::write(&inside, b"kept").unwrap();
        let grants = PathGrants::default();

        let folders = set_folder(&store, FolderKind::Documents, Some(&dir)).unwrap();
        set_folder(&store, FolderKind::Templates, Some(&dir)).unwrap();
        let start = start_folder(Dialog::Open, &folders, is_readable_folder);

        assert_eq!(start, StartFolder::Folder(dir.clone()));
        assert_eq!(grants.len(), 0);
        for kind in [GrantKind::Open, GrantKind::Save, GrantKind::Insert, GrantKind::Image] {
            assert!(grants.resolve(&text(&dir), kind).is_err());
            assert!(grants.resolve(&text(&inside), kind).is_err());
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The commands here take no AppState, so they have no PathGrants to add to.
    #[test]
    fn nothing_outside_the_tests_reaches_the_grants() {
        let source = include_str!("default_folders.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        for name in ["PathGrants", "GrantKind", "AppState", "State<"] {
            assert!(!code.contains(name), "{name}");
        }
    }
}
