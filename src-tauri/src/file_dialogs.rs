// The native file dialogs. What the user chooses stays in PathGrants
// (path_grants.rs); the webview gets a token and the file name.

use crate::file_ops::AppState;
use crate::path_grants::{file_name, GrantKind, GrantedFile, PathGrants};
use std::path::{Path, PathBuf};
use tauri::State;
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};

const ALL_FILES: [&str; 1] = ["*"];

const OPEN_EXTENSIONS: [&str; 10] = [
    "docx", "xlsx", "pptx", "odt", "ods", "odp", "rtf", "txt", "csv", "pdf",
];

// The formats Save As writes. A chosen name without one of them gets the
// format asked for.
const SAVE_EXTENSIONS: [&str; 13] = [
    "docx", "doc", "odt", "rtf", "txt", "xlsx", "xls", "ods", "csv", "pptx", "ppt", "odp", "pdf",
];

// One kind of the editor's OpenFilenameDialog.
struct PickKind {
    filter_name: &'static str,
    extensions: &'static [&'static str],
    grant: GrantKind,
    // The editor writes the chosen path into the document (a hyperlink, an
    // external reference), so it gets the path as well as the token.
    reference: bool,
}

fn pick_kind(kind: &str) -> PickKind {
    let insert = |filter_name, extensions| PickKind {
        filter_name,
        extensions,
        grant: GrantKind::Insert,
        reference: false,
    };
    match kind {
        "images" => PickKind {
            filter_name: "Images",
            extensions: &crate::protocol_limits::IMAGE_EXTENSIONS,
            grant: GrantKind::Image,
            reference: false,
        },
        "word" => insert("Documents", &["docx", "doc", "odt", "rtf", "txt"]),
        "video" => insert("Video", &["mp4", "avi", "mov", "wmv", "mkv", "webm"]),
        "audio" => insert("Audio", &["mp3", "wav", "ogg", "flac", "aac", "wma"]),
        "csv/txt" => insert("CSV / Text", &["csv", "txt"]),
        "(*.xml)" => insert("XML", &["xml"]),
        "cell" => PickKind {
            reference: true,
            ..insert("Spreadsheets", &["xlsx", "xls", "ods", "csv"])
        },
        _ => PickKind {
            reference: true,
            ..insert("All files", &ALL_FILES)
        },
    }
}

#[derive(serde::Deserialize)]
pub struct DialogFilter {
    name: String,
    extensions: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct PickedFile {
    token: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

fn file_dialog(window: &tauri::WebviewWindow) -> FileDialogBuilder<tauri::Wry> {
    window.dialog().file().set_parent(window)
}

async fn pick_one(dialog: FileDialogBuilder<tauri::Wry>) -> Result<Option<PathBuf>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .map_err(|e| e.to_string())?;
    Ok(picked.and_then(|choice| choice.into_path().ok()))
}

async fn pick_many(dialog: FileDialogBuilder<tauri::Wry>) -> Result<Vec<PathBuf>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_files())
        .await
        .map_err(|e| e.to_string())?;
    let paths = picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|choice| choice.into_path().ok())
        .collect();
    Ok(paths)
}

async fn pick_save(dialog: FileDialogBuilder<tauri::Wry>) -> Result<Option<PathBuf>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| e.to_string())?;
    Ok(picked.and_then(|choice| choice.into_path().ok()))
}

// The start screen's Open and the editor's File > Open.
#[tauri::command]
pub async fn pick_document(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    documents: String,
    all: String,
) -> Result<Option<GrantedFile>, String> {
    let dialog = file_dialog(&window)
        .add_filter(documents, &OPEN_EXTENSIONS)
        .add_filter(all, &ALL_FILES);
    let Some(path) = pick_one(dialog).await? else {
        return Ok(None);
    };
    Ok(Some(state.grants.grant(path, GrantKind::Open)))
}

// The editor's OpenFilenameDialog: images, documents to insert, compare or
// merge, and files a hyperlink or an external reference points to.
#[tauri::command]
pub async fn pick_files(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    kind: String,
    multiple: bool,
) -> Result<Vec<PickedFile>, String> {
    let kind = pick_kind(&kind);
    let mut dialog = file_dialog(&window).add_filter(kind.filter_name, kind.extensions);
    if kind.grant != GrantKind::Image {
        dialog = dialog.add_filter("All files", &ALL_FILES);
    }
    let paths = if multiple {
        pick_many(dialog).await?
    } else {
        pick_one(dialog).await?.into_iter().collect()
    };
    let picked = paths
        .into_iter()
        .map(|path| picked_file(&state.grants, path, &kind))
        .collect();
    Ok(picked)
}

fn picked_file(grants: &PathGrants, path: PathBuf, kind: &PickKind) -> PickedFile {
    let shown = kind.reference.then(|| path.to_string_lossy().to_string());
    let granted = grants.grant(path, kind.grant);
    PickedFile {
        token: granted.token,
        name: granted.name,
        path: shown,
    }
}

// A document names a file the user has not chosen in this session (an
// external reference saved in it): the user finds it, starting from its name.
#[tauri::command]
pub async fn locate_linked_file(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    title: String,
    name: String,
) -> Result<Option<GrantedFile>, String> {
    let dialog = file_dialog(&window)
        .set_title(title)
        .set_file_name(file_name(Path::new(&name)))
        .add_filter("All files", &ALL_FILES);
    let Some(path) = pick_one(dialog).await? else {
        return Ok(None);
    };
    Ok(Some(state.grants.grant(path, GrantKind::Insert)))
}

// Save As and Export. The filters come from the editor; where the dialog
// starts and the extension the chosen name ends with are decided here.
#[tauri::command]
pub async fn pick_save_target(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    filters: Vec<DialogFilter>,
    requested_ext: Option<String>,
) -> Result<Option<GrantedFile>, String> {
    let fallback_ext = requested_ext
        .clone()
        .or_else(|| first_extension(&filters))
        .unwrap_or_else(|| "docx".to_string());
    let mut dialog = file_dialog(&window);
    for filter in &filters {
        let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&filter.name, &extensions);
    }
    if let Some(start) = save_start(&state, requested_ext.as_deref()) {
        if let Some(dir) = start.parent() {
            dialog = dialog.set_directory(dir);
        }
        dialog = dialog.set_file_name(file_name(&start));
    }
    let Some(path) = pick_save(dialog).await? else {
        return Ok(None);
    };
    let path = with_save_extension(path, &fallback_ext);
    Ok(Some(state.grants.grant(path, GrantKind::Save)))
}

fn first_extension(filters: &[DialogFilter]) -> Option<String> {
    filters.first()?.extensions.first().cloned()
}

// After a recovery there is no open file, only where the document was saved
// before: the dialog starts there, in the format asked for.
fn save_start(state: &AppState, requested_ext: Option<&str>) -> Option<PathBuf> {
    if state.current_file.lock().unwrap().is_some() {
        return None;
    }
    let suggestion = state.save_suggestion.lock().unwrap().clone()?;
    Some(suggestion_with_ext(&suggestion, requested_ext))
}

// The path with its file name's extension replaced by `ext`, or added when the
// name has none.
pub fn suggestion_with_ext(path: &Path, ext: Option<&str>) -> PathBuf {
    let Some(ext) = ext else {
        return path.to_path_buf();
    };
    path.with_extension(ext)
}

// A name typed without a known extension gets the format's: x2t picks the
// format from the extension.
pub fn with_save_extension(path: PathBuf, fallback_ext: &str) -> PathBuf {
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if SAVE_EXTENSIONS.contains(&ext.as_str()) {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(fallback_ext);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_suggestion_takes_the_extension_asked_for() {
        assert_eq!(
            suggestion_with_ext(Path::new("/docs/report.v2.docx"), Some("pdf")),
            PathBuf::from("/docs/report.v2.pdf")
        );
        assert_eq!(
            suggestion_with_ext(Path::new("/home/u/my.docs/report"), Some("odt")),
            PathBuf::from("/home/u/my.docs/report.odt")
        );
        assert_eq!(
            suggestion_with_ext(Path::new("/docs/report.docx"), None),
            PathBuf::from("/docs/report.docx")
        );
    }

    #[test]
    fn a_saved_name_keeps_a_known_extension_and_gains_one_otherwise() {
        assert_eq!(
            with_save_extension(PathBuf::from("/docs/other.ODT"), "docx"),
            PathBuf::from("/docs/other.ODT")
        );
        assert_eq!(
            with_save_extension(PathBuf::from("/docs/report"), "pdf"),
            PathBuf::from("/docs/report.pdf")
        );
        assert_eq!(
            with_save_extension(PathBuf::from("/docs/v1.2"), "docx"),
            PathBuf::from("/docs/v1.2.docx")
        );
    }

    #[test]
    fn reference_dialogs_give_the_path_and_the_others_do_not() {
        assert!(pick_kind("any").reference);
        assert!(pick_kind("cell").reference);
        for kind in ["images", "word", "video", "audio", "csv/txt", "(*.xml)"] {
            assert!(!pick_kind(kind).reference, "{kind}");
        }
        assert_eq!(pick_kind("images").grant, GrantKind::Image);
        assert_eq!(pick_kind("word").grant, GrantKind::Insert);
    }

    #[test]
    fn a_reference_path_handed_back_is_not_accepted() {
        let grants = PathGrants::default();
        let picked = picked_file(
            &grants,
            PathBuf::from("/home/u/book.xlsx"),
            &pick_kind("cell"),
        );
        let shown = picked.path.clone().expect("a reference gets its path");
        assert_eq!(shown, "/home/u/book.xlsx");
        for kind in [
            GrantKind::Open,
            GrantKind::Save,
            GrantKind::Insert,
            GrantKind::Image,
        ] {
            assert!(grants.resolve(&shown, kind).is_err());
        }
        assert!(grants.resolve(&picked.token, GrantKind::Insert).is_ok());
    }
}
