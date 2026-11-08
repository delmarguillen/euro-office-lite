// The native file dialogs. What the user chooses stays in PathGrants
// (path_grants.rs); the webview gets a token and the file name.

use crate::file_ops::AppState;
use crate::path_grants::{file_name, GrantKind, GrantedFile, PathGrants};
use std::path::{Path, PathBuf};
use tauri::State;
use tauri_plugin_dialog::{
    DialogExt, FileDialogBuilder, MessageDialogBuilder, MessageDialogButtons, MessageDialogKind,
};

const ALL_FILES: [&str; 1] = ["*"];

// Every one of them converts to Editor.bin with x2t 10.0.0.169.
const OPEN_EXTENSIONS: [&str; 28] = [
    "docx", "doc", "docm", "dotx", "dotm", "odt", "ott", "fodt", "rtf", "txt",
    "xlsx", "xls", "xlsm", "xltx", "xlsb", "ods", "ots", "fods", "csv",
    "pptx", "ppt", "pptm", "ppsx", "potx", "odp", "otp", "fodp", "pdf",
];

// The start screen's Edit PDF.
const PDF_EXTENSIONS: [&str; 1] = ["pdf"];

// The formats Save As writes. A chosen name without one of them gets the
// format asked for.
const SAVE_EXTENSIONS: [&str; 13] = [
    "docx", "doc", "odt", "rtf", "txt", "xlsx", "xls", "ods", "csv", "pptx", "ppt", "odp", "pdf",
];

// The formats Ctrl+S writes back to the document's own path. Its own list,
// not SAVE_EXTENSIONS: x2t 10 refuses to write .doc and .ppt (exit 80/88)
// and its .xls output is not checked against the original, so a .doc, .xls
// or .ppt goes through Save As. _EO_SAVE_IN_PLACE in src/bridge.js must match
// (scripts/test-bridge-save-unwritable-format.mjs checks it).
const SAVE_IN_PLACE_EXTENSIONS: [&str; 10] = [
    "docx", "odt", "rtf", "txt", "xlsx", "ods", "csv", "pptx", "odp", "pdf",
];

// The GTK file chooser rfd uses on Linux matches each "*.ext" pattern with
// the case it is given, so REPORT.XLSM is hidden by an "xlsm" filter. Windows
// and macOS dialogs ignore case. Mixed case (Report.Docx) is still hidden.
fn open_filter_extensions(extensions: &[&str]) -> Vec<String> {
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        return extensions.iter().map(|ext| ext.to_string()).collect();
    }
    with_upper_case(extensions)
}

// Each extension followed by its upper-case form.
fn with_upper_case(extensions: &[&str]) -> Vec<String> {
    let mut both = Vec::with_capacity(extensions.len() * 2);
    for ext in extensions {
        both.push(ext.to_string());
        both.push(ext.to_uppercase());
    }
    both
}

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
    let extensions = open_filter_extensions(&OPEN_EXTENSIONS);
    let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let dialog = file_dialog(&window)
        .add_filter(documents, &extensions)
        .add_filter(all, &ALL_FILES);
    let Some(path) = pick_one(dialog).await? else {
        return Ok(None);
    };
    Ok(Some(state.grants.grant(path, GrantKind::Open)))
}

// The start screen's Edit PDF: PDFs first, then everything. Whatever is
// chosen is granted like Open's choice, so a non-PDF opens as Open would.
#[tauri::command]
pub async fn pick_pdf(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    pdf_label: String,
    all: String,
) -> Result<Option<GrantedFile>, String> {
    let extensions = open_filter_extensions(&PDF_EXTENSIONS);
    let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let dialog = file_dialog(&window)
        .add_filter(pdf_label, &extensions)
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
    let Some(chosen) = pick_save(dialog).await? else {
        return Ok(None);
    };
    let path = with_save_extension(chosen.clone(), &fallback_ext);
    if asks_before_replacing(&chosen, &path) && !confirm_replace(&window, &path).await? {
        return Ok(None);
    }
    Ok(Some(state.grants.grant(path, GrantKind::Save)))
}

// Asked from here, not from the webview: the answer decides the grant.
async fn confirm_replace(window: &tauri::WebviewWindow, path: &Path) -> Result<bool, String> {
    let dialog = replace_dialog(window, path);
    tauri::async_runtime::spawn_blocking(move || dialog.blocking_show())
        .await
        .map_err(|e| e.to_string())
}

fn replace_dialog(window: &tauri::WebviewWindow, path: &Path) -> MessageDialogBuilder<tauri::Wry> {
    let buttons = MessageDialogButtons::OkCancelCustom("Replace".into(), "Cancel".into());
    window
        .dialog()
        .message(replace_question(path))
        .title("Replace file?")
        .kind(MessageDialogKind::Warning)
        .buttons(buttons)
        .parent(window)
}

fn replace_question(path: &Path) -> String {
    format!(
        "\"{}\" already exists. Do you want to replace it?",
        file_name(path)
    )
}

fn first_extension(filters: &[DialogFilter]) -> Option<String> {
    filters.first()?.extensions.first().cloned()
}

fn save_start(state: &AppState, requested_ext: Option<&str>) -> Option<PathBuf> {
    let current = state.current_file.lock().unwrap().clone();
    let suggestion = state.save_suggestion.lock().unwrap().clone();
    save_start_path(current.as_deref(), suggestion.as_deref(), requested_ext)
}

// Where the dialog starts and the name it offers, in the format asked for: the
// open document, else where a recovered document was saved before. Only a
// starting point: the grant still comes from the path the user picks.
fn save_start_path(
    current: Option<&Path>,
    suggestion: Option<&Path>,
    requested_ext: Option<&str>,
) -> Option<PathBuf> {
    let start = current.or(suggestion)?;
    Some(suggestion_with_ext(start, requested_ext))
}

// The path with its file name's extension replaced by `ext`, or added when the
// name has none.
pub fn suggestion_with_ext(path: &Path, ext: Option<&str>) -> PathBuf {
    let Some(ext) = ext else {
        return path.to_path_buf();
    };
    path.with_extension(ext)
}

// Whether Ctrl+S may write the open document back to its own path. Formats
// outside SAVE_IN_PLACE_EXTENSIONS (a .doc, a .dotx, an .xlsm, a .fodt) go
// through Save As: the bridge never sends them here, and save_file refuses them.
pub fn saves_in_place(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    SAVE_IN_PLACE_EXTENSIONS.contains(&ext.as_str())
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

// The native dialog asks before replacing the name it returns. A name the app
// extended afterwards is another file the dialog never checked.
pub fn needs_replace_confirmation(chosen: &Path, final_path: &Path, exists: bool) -> bool {
    if chosen == final_path {
        return false;
    }
    exists
}

// Whether the final path holds something the save would replace. The link
// itself is read, not followed: a link counts even when broken. A folder does
// not count: saving onto it fails later, as before.
fn replace_target_exists(final_path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(final_path) else {
        return false;
    };
    !metadata.is_dir()
}

fn asks_before_replacing(chosen: &Path, final_path: &Path) -> bool {
    needs_replace_confirmation(chosen, final_path, replace_target_exists(final_path))
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
    fn the_open_filter_offers_the_legacy_and_variant_office_formats() {
        let offered = [
            "doc", "docm", "dotx", "dotm", "ott", "fodt", "xls", "xlsm", "xltx", "xlsb",
            "ots", "fods", "ppt", "pptm", "ppsx", "potx", "otp", "fodp",
        ];
        for ext in offered {
            assert!(OPEN_EXTENSIONS.contains(&ext), "{}", ext);
        }
        for ext in OPEN_EXTENSIONS {
            let path = std::path::PathBuf::from(format!("/tmp/a.{}", ext));
            assert_ne!(crate::x2t_task::detect_format(&path), 0, "{}", ext);
        }
    }

    #[test]
    fn only_the_formats_save_as_writes_are_saved_in_place() {
        for name in ["a.docx", "a.DOCX", "a.xlsx", "a.pptx", "a.odt", "a.csv", "a.pdf"] {
            assert!(saves_in_place(Path::new(name)), "{}", name);
        }
        for name in ["a.dotx", "a.docm", "a.xlsm", "a.xlsb", "a.pptm", "a.fodt", "a.ots", "a"] {
            assert!(!saves_in_place(Path::new(name)), "{}", name);
        }
    }

    #[test]
    fn legacy_binary_formats_go_through_save_as() {
        for name in ["a.doc", "a.DOC", "a.xls", "a.ppt"] {
            assert!(!saves_in_place(Path::new(name)), "{}", name);
        }
    }

    #[test]
    fn legacy_binary_formats_stay_save_as_targets() {
        for ext in ["doc", "xls", "ppt"] {
            assert!(SAVE_EXTENSIONS.contains(&ext), "{}", ext);
        }
        for ext in SAVE_IN_PLACE_EXTENSIONS {
            assert!(SAVE_EXTENSIONS.contains(&ext), "{}", ext);
        }
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

    #[test]
    fn the_upper_case_form_follows_each_extension() {
        assert_eq!(with_upper_case(&["xlsm", "pdf"]), ["xlsm", "XLSM", "pdf", "PDF"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_linux_open_filters_show_upper_case_names() {
        let documents = open_filter_extensions(&OPEN_EXTENSIONS);
        for ext in ["xlsm", "XLSM", "docx", "DOCX", "fodt", "FODT"] {
            assert!(documents.contains(&ext.to_string()), "{}", ext);
        }
        assert_eq!(documents.len(), OPEN_EXTENSIONS.len() * 2);
        assert_eq!(open_filter_extensions(&PDF_EXTENSIONS), ["pdf", "PDF"]);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn the_windows_and_macos_open_filters_are_not_doubled() {
        assert_eq!(open_filter_extensions(&PDF_EXTENSIONS), ["pdf"]);
    }

    #[test]
    fn pdf_filter_lists_only_pdf() {
        assert_eq!(PDF_EXTENSIONS, ["pdf"]);
    }

    #[test]
    fn open_filter_also_accepts_pdf() {
        assert!(OPEN_EXTENSIONS.contains(&"pdf"));
    }

    #[test]
    fn an_extended_name_that_exists_asks_before_replacing() {
        let chosen = PathBuf::from("/docs/Budget");
        let final_path = with_save_extension(chosen.clone(), "docx");
        assert_eq!(final_path, PathBuf::from("/docs/Budget.docx"));
        assert!(needs_replace_confirmation(&chosen, &final_path, true));
    }

    #[test]
    fn an_extended_name_that_does_not_exist_does_not_ask() {
        let chosen = PathBuf::from("/docs/Budget");
        let final_path = with_save_extension(chosen.clone(), "docx");
        assert!(!needs_replace_confirmation(&chosen, &final_path, false));
    }

    #[test]
    fn a_name_the_dialog_returned_unchanged_is_not_asked_again() {
        let chosen = PathBuf::from("/docs/Budget.docx");
        let final_path = with_save_extension(chosen.clone(), "docx");
        assert_eq!(final_path, chosen);
        assert!(!needs_replace_confirmation(&chosen, &final_path, true));
    }

    #[test]
    fn an_upper_case_extension_keeps_the_name_and_is_not_asked_again() {
        let chosen = PathBuf::from("/docs/Budget.DOCX");
        let final_path = with_save_extension(chosen.clone(), "docx");
        assert_eq!(final_path, chosen);
        assert!(!needs_replace_confirmation(&chosen, &final_path, true));
    }

    // .dotx is not a format Save As writes, so the name gains .docx.
    #[test]
    fn an_unknown_extension_gains_the_format_and_asks_if_that_exists() {
        let chosen = PathBuf::from("/docs/plantilla.dotx");
        let final_path = with_save_extension(chosen.clone(), "docx");
        assert_eq!(final_path, PathBuf::from("/docs/plantilla.dotx.docx"));
        assert!(needs_replace_confirmation(&chosen, &final_path, true));
    }

    #[test]
    fn the_question_names_the_file_and_not_its_folder() {
        let question = replace_question(Path::new("/home/u/docs/Budget.docx"));
        assert_eq!(
            question,
            "\"Budget.docx\" already exists. Do you want to replace it?"
        );
    }

    #[test]
    fn an_existing_file_asks_and_an_existing_folder_does_not() {
        let dir = std::env::temp_dir().join(format!("eo-replace-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Report.docx")).unwrap();
        std::fs::write(dir.join("Budget.docx"), b"kept").unwrap();

        let budget = dir.join("Budget");
        let budget_docx = with_save_extension(budget.clone(), "docx");
        assert!(asks_before_replacing(&budget, &budget_docx));

        let report = dir.join("Report");
        let report_docx = with_save_extension(report.clone(), "docx");
        assert!(!asks_before_replacing(&report, &report_docx));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_open_document_starts_in_its_folder_with_the_format_asked_for() {
        let current = Path::new("/home/u/Docs/Budget.docx");
        assert_eq!(
            save_start_path(Some(current), None, Some("pdf")),
            Some(PathBuf::from("/home/u/Docs/Budget.pdf"))
        );
    }

    #[test]
    fn an_open_document_without_a_format_starts_on_its_own_name() {
        let current = Path::new("/home/u/Docs/Budget.docx");
        assert_eq!(
            save_start_path(Some(current), None, None),
            Some(PathBuf::from("/home/u/Docs/Budget.docx"))
        );
    }

    #[test]
    fn an_open_document_wins_over_a_recovery_suggestion() {
        let current = Path::new("/home/u/Docs/Budget.docx");
        let suggestion = Path::new("/old/report.docx");
        assert_eq!(
            save_start_path(Some(current), Some(suggestion), Some("pdf")),
            Some(PathBuf::from("/home/u/Docs/Budget.pdf"))
        );
    }

    #[test]
    fn after_a_recovery_the_suggestion_starts_in_the_format_asked_for() {
        let suggestion = Path::new("/old/report.docx");
        assert_eq!(
            save_start_path(None, Some(suggestion), Some("pdf")),
            Some(PathBuf::from("/old/report.pdf"))
        );
    }

    #[test]
    fn without_a_document_or_a_suggestion_there_is_no_start() {
        assert_eq!(save_start_path(None, None, Some("pdf")), None);
    }

    fn temp_dir_for(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eo-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_link_on_the_extended_name_asks_before_replacing() {
        let dir = temp_dir_for("broken-link");
        let link = dir.join("Budget.pdf");
        std::os::unix::fs::symlink(dir.join("missing.pdf"), &link).unwrap();
        assert!(!link.exists());

        let chosen = dir.join("Budget");
        let final_path = with_save_extension(chosen.clone(), "pdf");
        assert_eq!(final_path, link);
        assert!(asks_before_replacing(&chosen, &final_path));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_file_counts_as_existing() {
        let dir = temp_dir_for("file-link");
        std::fs::write(dir.join("target.pdf"), b"kept").unwrap();
        let link = dir.join("Budget.pdf");
        std::os::unix::fs::symlink(dir.join("target.pdf"), &link).unwrap();
        assert!(replace_target_exists(&link));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_counts_as_existing_and_a_folder_or_nothing_does_not() {
        let dir = temp_dir_for("exists");
        std::fs::write(dir.join("Budget.pdf"), b"kept").unwrap();
        std::fs::create_dir_all(dir.join("Report.pdf")).unwrap();

        assert!(replace_target_exists(&dir.join("Budget.pdf")));
        assert!(!replace_target_exists(&dir.join("Report.pdf")));
        assert!(!replace_target_exists(&dir.join("Missing.pdf")));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
