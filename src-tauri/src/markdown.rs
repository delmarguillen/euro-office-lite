// The Markdown page (src/markdown/). A .md is read and written as it is on
// disk, never through x2t: x2t opens Markdown but does not write it back
// faithfully. The webview names files by token (path_grants.rs) and gets the
// bytes; turning them into text and back (BOM, line ends) is done by
// src/markdown/markdown-text.js, so what is saved is exactly what it sends.

use crate::file_ops::{log_event, AppState};
use crate::path_grants::{file_name, GrantKind, PathGrants};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::State;

pub const MARKDOWN_EXTENSIONS: [&str; 2] = ["md", "markdown"];

// The whole text sits in a textarea and is rendered again after each pause in
// typing; 10 MB is far beyond a hand-written document.
pub const MARKDOWN_MAX_BYTES: u64 = 10 * 1024 * 1024;

pub const MARKDOWN_IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;

// The exported HTML of a document up to MARKDOWN_MAX_BYTES: the markup and the
// page's stylesheet, images left as references.
pub const HTML_EXPORT_MAX_BYTES: u64 = 4 * MARKDOWN_MAX_BYTES;

pub const HTML_EXTENSIONS: [&str; 2] = ["html", "htm"];

// Start of the error for a file over MARKDOWN_MAX_BYTES, so the page can say why.
pub const TOO_LARGE_ERROR: &str = "markdown-too-large";

// The image types an <img> shows, with the type its data: URL declares.
const IMAGE_TYPES: [(&str, &str); 7] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
];

fn lower_extension(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn is_markdown(path: &Path) -> bool {
    MARKDOWN_EXTENSIONS.contains(&lower_extension(path).as_str())
}

fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = lower_extension(path);
    IMAGE_TYPES
        .iter()
        .find(|(known, _)| *known == ext)
        .map(|(_, mime)| *mime)
}

fn too_large(max: u64) -> String {
    format!("{}: the file is larger than {} bytes", TOO_LARGE_ERROR, max)
}

// Reads at most `max` bytes; a larger file is refused, also when it grew
// after the size was checked.
fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("not a file".to_string());
    }
    if metadata.len() > max {
        return Err(too_large(max));
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(too_large(max));
    }
    Ok(bytes)
}

// The Markdown file an Open token names, and its bytes.
pub(crate) fn read_markdown_file(
    grants: &PathGrants,
    token: &str,
) -> Result<(PathBuf, Vec<u8>), String> {
    let path = grants.resolve(token, GrantKind::Open)?;
    if !is_markdown(&path) {
        return Err("not a Markdown file".to_string());
    }
    let bytes = read_limited(&path, MARKDOWN_MAX_BYTES)?;
    Ok((path, bytes))
}

// The open document, when it is a Markdown file.
fn open_markdown_file(current: Option<PathBuf>) -> Result<PathBuf, String> {
    let Some(path) = current else {
        return Err("No file is currently open".to_string());
    };
    if !is_markdown(&path) {
        return Err("The open file is not a Markdown file".to_string());
    }
    Ok(path)
}

// The document relative images are read against: the open file, or for a
// recovered document (no open file yet) the file it was last saved as.
fn image_reference_document(
    current: Option<PathBuf>,
    suggestion: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if current.is_some() {
        return open_markdown_file(current);
    }
    let Some(suggested) = suggestion else {
        return Err("No file is currently open".to_string());
    };
    if !is_markdown(&suggested) {
        return Err("The suggested file is not a Markdown file".to_string());
    }
    Ok(suggested)
}

// Save writes over the open file; Save As writes exactly the file its token
// names. A name not ending in .md or .markdown is refused, not renamed: the
// renamed file was never confirmed in the dialog and may already exist. A new
// document (no open file) can only be saved with a token.
fn markdown_save_destination(
    grants: &PathGrants,
    token: Option<&str>,
    current: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let Some(token) = token else {
        return open_markdown_file(current);
    };
    let chosen = grants.resolve(token, GrantKind::Save)?;
    if !is_markdown(&chosen) {
        return Err("Markdown can only be saved to a name ending in .md or .markdown".to_string());
    }
    Ok(chosen)
}

fn decode_limited(label: &str, data: &str, max: u64) -> Result<Vec<u8>, String> {
    if crate::protocol_limits::check_base64_size(label, data, max as usize).is_err() {
        return Err(too_large(max));
    }
    let bytes = STANDARD.decode(data).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(too_large(max));
    }
    Ok(bytes)
}

fn decode_markdown_data(data: &str) -> Result<Vec<u8>, String> {
    decode_limited("Markdown text", data, MARKDOWN_MAX_BYTES)
}

// Export as HTML writes exactly the file a Save token names, ending in .html
// or .htm (pick_save_target adds .html to a name without an extension).
fn html_export_destination(grants: &PathGrants, token: &str) -> Result<PathBuf, String> {
    let chosen = grants.resolve(token, GrantKind::Save)?;
    if !HTML_EXTENSIONS.contains(&lower_extension(&chosen).as_str()) {
        return Err("HTML can only be exported to a name ending in .html or .htm".to_string());
    }
    Ok(chosen)
}

// The file an image reference `src` names, relative to the folder of
// `document`. Only a file inside that folder is accepted: an absolute path, a
// drive or a scheme, a `..` that climbs out, or a symbolic link that points
// out is refused.
pub(crate) fn markdown_image_path(document: &Path, src: &str) -> Result<PathBuf, String> {
    let outside = || "the image is outside the document's folder".to_string();
    if src.is_empty() || src.contains(':') || src.starts_with('/') || src.starts_with('\\') {
        return Err(outside());
    }
    let relative = Path::new(src);
    if relative.has_root() || relative.is_absolute() {
        return Err(outside());
    }
    let Some(folder) = document.parent() else {
        return Err(outside());
    };
    let folder = std::fs::canonicalize(folder).map_err(|e| e.to_string())?;
    let image =
        std::fs::canonicalize(folder.join(relative)).map_err(|_| "image not found".to_string())?;
    if !image.starts_with(&folder) {
        return Err(outside());
    }
    Ok(image)
}

#[derive(serde::Serialize)]
pub struct MarkdownFile {
    data: String,
    name: String,
}

#[derive(serde::Serialize)]
pub struct MarkdownImage {
    mime: &'static str,
    data: String,
}

// What the backend knows of the open document, for a Markdown file (`path`)
// or a new Markdown document (None), as an open in the editors resets it.
fn start_markdown_document(state: &AppState, path: Option<PathBuf>) {
    crate::recovery::end_session(state, true);
    *state.pending_recent.lock().unwrap() = None;
    crate::file_ops::set_current_file(state, path);
    *state.save_suggestion.lock().unwrap() = None;
    *state.modified.lock().unwrap() = false;
    *state.text_options.lock().unwrap() = None;
}

// The Markdown file an Open token names becomes the open document, unless
// another window has it open: two windows saving one file overwrite each other.
fn open_markdown(state: &AppState, token: &str) -> Result<(PathBuf, Vec<u8>), String> {
    let (path, bytes) = read_markdown_file(&state.grants, token)?;
    crate::new_window::refuse_if_open_elsewhere(state, &path)?;
    start_markdown_document(state, Some(path.clone()));
    Ok((path, bytes))
}

// Opens the Markdown file an Open token names: it becomes the open document
// (the close warning, Save, Recents) and its bytes go to the page in base64.
#[tauri::command]
pub async fn read_markdown(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    token: String,
) -> Result<MarkdownFile, String> {
    let (path, bytes) = open_markdown(&state, &token).map_err(|error| {
        log_event(&state, &format!("[OPEN] markdown failed: {}", error));
        error
    })?;
    crate::recent::record(&app, &path.to_string_lossy());
    log_event(
        &state,
        &format!("[OPEN] markdown success bytes={}", bytes.len()),
    );
    Ok(MarkdownFile {
        data: STANDARD.encode(&bytes),
        name: file_name(&path),
    })
}

// New Markdown on the start screen: a document with no file yet. Its first
// save goes through Save As (save_markdown with a token).
#[tauri::command]
pub async fn new_markdown(state: State<'_, AppState>) -> Result<(), String> {
    start_markdown_document(&state, None);
    log_event(&state, "[OPEN] new markdown");
    Ok(())
}

struct SavedMarkdown {
    path: PathBuf,
    // The save went to another file, which is now the open document.
    new_file: bool,
}

// Writes `data` (base64) as it is: over the open Markdown file, or to the file
// a Save token names.
fn store_markdown(
    state: &AppState,
    data: &str,
    token: Option<&str>,
) -> Result<SavedMarkdown, String> {
    let current = state.current_file.lock().unwrap().clone();
    let dest = markdown_save_destination(&state.grants, token, current.clone())?;
    // As the editors' Save As (save_file_as): the other window would write
    // its own copy over this one on its next save.
    if token.is_some() {
        crate::new_window::refuse_save_if_open_elsewhere(state, &dest)?;
    }
    let written = decode_markdown_data(data)
        .and_then(|bytes| crate::save_target::write_bytes_beside_and_rename(&bytes, &dest));
    if let Err(error) = written {
        log_event(state, &format!("[SAVE] markdown failed: {}", error));
        return Err(error);
    }
    let new_file = current.as_ref() != Some(&dest);
    if new_file {
        crate::file_ops::set_current_file(state, Some(dest.clone()));
    }
    *state.modified.lock().unwrap() = false;
    // The text is in the file now: its recovery copy goes.
    crate::recovery::end_session(state, true);
    Ok(SavedMarkdown { path: dest, new_file })
}

// Save and Save As of the Markdown page. Returns the saved file's name.
#[tauri::command]
pub async fn save_markdown(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    data: String,
    token: Option<String>,
) -> Result<String, String> {
    let saved = store_markdown(&state, &data, token.as_deref())?;
    if saved.new_file {
        crate::recent::record(&app, &saved.path.to_string_lossy());
    }
    log_event(&state, "[SAVE] markdown success");
    Ok(file_name(&saved.path))
}

// Writes the page's exported HTML (base64 of the UTF-8 text) to the file a
// Save token names. The open document, its modified state and Recents stay
// as they are. Returns the written file's name.
#[tauri::command]
pub async fn export_markdown_html(
    state: State<'_, AppState>,
    data: String,
    token: String,
) -> Result<String, String> {
    let written = html_export_destination(&state.grants, &token).and_then(|dest| {
        let bytes = decode_limited("Exported HTML", &data, HTML_EXPORT_MAX_BYTES)?;
        crate::save_target::write_bytes_beside_and_rename(&bytes, &dest)?;
        Ok(dest)
    });
    match written {
        Ok(dest) => {
            log_event(&state, "[EXPORT] markdown html success");
            Ok(file_name(&dest))
        }
        Err(error) => {
            log_event(&state, &format!("[EXPORT] markdown html failed: {}", error));
            Err(error)
        }
    }
}

// The recovery copy of the page's text while it is modified: `data` is
// base64 of the bytes Save would write. The first call starts the session
// (recovery.rs, "Markdown sessions"); later ones replace the copy. Ended by
// save_markdown, by recovery_end when the page discards, and at exit.
#[tauri::command]
pub async fn markdown_recovery_write(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    data: String,
) -> Result<(), String> {
    let bytes = decode_markdown_data(&data)?;
    let root = crate::recovery::recover_root(&app)?;
    let current = state.current_file.lock().unwrap().clone();
    let version = app.package_info().version.to_string();
    let written = {
        let mut live = state.recovery.lock().unwrap();
        if live.as_ref().is_some_and(|session| !session.markdown) {
            return Err("an editor's recovery session is live".to_string());
        }
        if live.is_none() {
            let session = crate::recovery::begin_markdown_session(
                &root,
                current.as_deref(),
                &version,
                &state.instance.id,
            )?;
            *live = Some(session);
        }
        let session = live.as_ref().expect("a session was just started");
        crate::recovery::write_markdown_copy(session, &bytes)
    };
    if let Err(error) = &written {
        log_event(&state, &format!("[RECOVER] markdown copy failed: {}", error));
    }
    written
}

// Reopens a Markdown document offered on the start screen. The folder
// becomes this process's session, so the copy stays until the user saves or
// discards. Nothing is tied to the file it came from: the first save goes
// through Save As, which starts at that file (save_suggestion).
#[tauri::command]
pub async fn markdown_recovery_load(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<MarkdownFile, String> {
    let root = crate::recovery::recover_root(&app)?;
    let dir = crate::recovery::markdown_recovery_dir(&root, &id)?;
    let bytes = read_limited(&crate::recovery::markdown_text_path(&dir), MARKDOWN_MAX_BYTES)?;
    let (session, saved_path) = crate::recovery::adopt_markdown_session(&dir)?;
    start_markdown_document(&state, None);
    let name = crate::recovery::recovered_name(&dir).unwrap_or_else(|| id.clone());
    *state.save_suggestion.lock().unwrap() = saved_path.map(PathBuf::from);
    *state.recovery.lock().unwrap() = Some(session);
    log_event(
        &state,
        &format!("[RECOVER] markdown load session={} bytes={}", id, bytes.len()),
    );
    Ok(MarkdownFile {
        data: STANDARD.encode(&bytes),
        name,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReloadedMarkdown {
    data: String,
    name: String,
    has_file: bool,
}

// The text file of the live Markdown session, when it holds a copy.
fn live_markdown_copy(session: Option<&crate::recovery::RecoverySession>) -> Option<PathBuf> {
    let session = session?;
    if !session.markdown {
        return None;
    }
    let text = crate::recovery::markdown_text_path(&session.dir);
    if !text.is_file() {
        return None;
    }
    Some(text)
}

// A reload of the page (F5, devtools) while the document has a recovery
// copy: the copy comes back and the session stays, so the open file, its
// save suggestion and the copy are as they were. None when there is no copy;
// the page then opens the file as before (read_markdown).
#[tauri::command]
pub async fn markdown_recovery_reload(
    state: State<'_, AppState>,
) -> Result<Option<ReloadedMarkdown>, String> {
    let copy = live_markdown_copy(state.recovery.lock().unwrap().as_ref());
    let Some(copy) = copy else {
        return Ok(None);
    };
    let bytes = read_limited(&copy, MARKDOWN_MAX_BYTES)?;
    let current = state.current_file.lock().unwrap().clone();
    let name = match &current {
        Some(path) => file_name(path),
        None => crate::recovery::recovered_name(copy.parent().unwrap_or(&copy))
            .unwrap_or_else(|| "Untitled.md".to_string()),
    };
    log_event(
        &state,
        &format!("[RECOVER] markdown reload bytes={}", bytes.len()),
    );
    Ok(Some(ReloadedMarkdown {
        data: STANDARD.encode(&bytes),
        name,
        has_file: current.is_some(),
    }))
}

// An image of the Markdown document, as a data: URL's parts. `src` is the
// reference written in the document, resolved by markdown_image_path against
// image_reference_document.
#[tauri::command]
pub async fn markdown_image(
    state: State<'_, AppState>,
    src: String,
) -> Result<MarkdownImage, String> {
    let current = state.current_file.lock().unwrap().clone();
    let suggestion = state.save_suggestion.lock().unwrap().clone();
    let document = image_reference_document(current, suggestion)?;
    let path = markdown_image_path(&document, &src)?;
    let Some(mime) = image_mime(&path) else {
        return Err("not an image type the preview shows".to_string());
    };
    let bytes = read_limited(&path, MARKDOWN_IMAGE_MAX_BYTES)?;
    Ok(MarkdownImage {
        mime,
        data: STANDARD.encode(&bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_grants::GRANT_TTL;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    static DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn test_dir(label: &str) -> PathBuf {
        let n = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-markdown-{}-{}-{}",
            label,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_with(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    // The state of a window whose instance folder is under `root`, built as
    // main.rs builds it. Each `pid` is another window.
    fn window_state(root: &Path, pid: u32) -> AppState {
        let instance = crate::temp_layout::begin_instance(root, 1000, pid).unwrap();
        AppState {
            current_file: std::sync::Mutex::new(None),
            temp_dir: instance.dir.clone(),
            temp_root: root.to_path_buf(),
            font_root: root.to_path_buf(),
            fonts: std::sync::Arc::new(crate::fonts_state::FontsState::pending()),
            instance,
            doc_seq: std::sync::atomic::AtomicU64::new(1),
            modified: std::sync::Mutex::new(false),
            pending_recent: std::sync::Mutex::new(None),
            recovery: std::sync::Mutex::new(None),
            pending_open: std::sync::Mutex::new(None),
            grants: PathGrants::default(),
            text_options: std::sync::Mutex::new(None),
            last_print_pdf: std::sync::Mutex::new(None),
            pasted_images: std::sync::Mutex::new(Vec::new()),
            save_suggestion: std::sync::Mutex::new(None),
            title_bar_args: Vec::new(),
        }
    }

    // Whether the window `state` belongs to is seen by the others as having `file` open.
    fn published(state: &AppState, file: &Path) -> bool {
        crate::temp_layout::open_in_another_instance(&state.temp_root, "someone-else", file)
    }

    const TEXT_B64: &str = "dGV4dAo="; // "text\n"

    #[test]
    fn opening_publishes_the_file_to_the_other_windows() {
        let dir = test_dir("open-publish");
        let file = file_with(&dir, "notes.md", b"text\n");
        let state = window_state(&dir, 1);
        let token = state.grants.grant(file.clone(), GrantKind::Open).token;
        open_markdown(&state, &token).unwrap();
        assert!(published(&state, &file));
        assert_eq!(*state.current_file.lock().unwrap(), Some(file));
    }

    #[test]
    fn a_file_open_in_another_window_is_not_opened() {
        let dir = test_dir("open-elsewhere");
        let file = file_with(&dir, "notes.md", b"text\n");
        let other = window_state(&dir, 2);
        crate::file_ops::set_current_file(&other, Some(file.clone()));
        let state = window_state(&dir, 1);
        let token = state.grants.grant(file.clone(), GrantKind::Open).token;
        let result = open_markdown(&state, &token);
        assert_eq!(result.err().as_deref(), Some(crate::new_window::OPEN_ELSEWHERE));
        assert_eq!(*state.current_file.lock().unwrap(), None);
    }

    #[test]
    fn a_file_another_window_closed_opens() {
        let dir = test_dir("closed-elsewhere");
        let file = file_with(&dir, "notes.md", b"text\n");
        let other = window_state(&dir, 2);
        crate::file_ops::set_current_file(&other, Some(file.clone()));
        drop(other);
        let state = window_state(&dir, 1);
        let token = state.grants.grant(file.clone(), GrantKind::Open).token;
        assert!(open_markdown(&state, &token).is_ok());
    }

    #[test]
    fn save_as_over_a_file_open_in_another_window_is_refused() {
        let dir = test_dir("save-elsewhere");
        let taken = file_with(&dir, "taken.md", b"theirs\n");
        let other = window_state(&dir, 2);
        crate::file_ops::set_current_file(&other, Some(taken.clone()));
        let state = window_state(&dir, 1);
        let token = state.grants.grant(taken.clone(), GrantKind::Save).token;
        let result = store_markdown(&state, TEXT_B64, Some(&token));
        assert_eq!(result.err().as_deref(), Some(crate::new_window::OPEN_ELSEWHERE));
        assert_eq!(std::fs::read(&taken).unwrap(), b"theirs\n");
        assert_eq!(*state.current_file.lock().unwrap(), None);
    }

    #[test]
    fn save_as_over_the_file_open_here_is_allowed() {
        let dir = test_dir("save-here");
        let file = file_with(&dir, "notes.md", b"old\n");
        let state = window_state(&dir, 1);
        let open = state.grants.grant(file.clone(), GrantKind::Open).token;
        open_markdown(&state, &open).unwrap();
        let save = state.grants.grant(file.clone(), GrantKind::Save).token;
        assert!(store_markdown(&state, TEXT_B64, Some(&save)).is_ok());
        assert_eq!(std::fs::read(&file).unwrap(), b"text\n");
    }

    #[test]
    fn a_new_document_publishes_no_file() {
        let dir = test_dir("new-publish");
        let file = file_with(&dir, "notes.md", b"text\n");
        let state = window_state(&dir, 1);
        crate::file_ops::set_current_file(&state, Some(file.clone()));
        start_markdown_document(&state, None);
        assert!(!published(&state, &file));
    }

    #[test]
    fn save_as_publishes_the_new_file_and_stops_publishing_the_old_one() {
        let dir = test_dir("save-publish");
        let old = file_with(&dir, "notes.md", b"text\n");
        let state = window_state(&dir, 1);
        let open = state.grants.grant(old.clone(), GrantKind::Open).token;
        open_markdown(&state, &open).unwrap();
        let copy = dir.join("copy.md");
        let save = state.grants.grant(copy.clone(), GrantKind::Save).token;
        let saved = store_markdown(&state, TEXT_B64, Some(&save)).unwrap();
        assert!(saved.new_file);
        assert!(published(&state, &copy));
        assert!(!published(&state, &old));
    }

    #[test]
    fn an_open_token_reads_the_file_as_it_is() {
        let dir = test_dir("read");
        let bytes = b"\xEF\xBB\xBF# Title\r\n\r\ntext\r\n";
        let path = file_with(&dir, "notes.md", bytes);
        let grants = PathGrants::default();
        let token = grants.grant(path.clone(), GrantKind::Open).token;
        let (read_path, read_bytes) = read_markdown_file(&grants, &token).unwrap();
        assert_eq!(read_path, path);
        assert_eq!(read_bytes, bytes);
    }

    #[test]
    fn upper_case_and_long_extensions_are_markdown() {
        assert!(is_markdown(Path::new("/d/README.MD")));
        assert!(is_markdown(Path::new("/d/notes.markdown")));
        assert!(!is_markdown(Path::new("/d/report.docx")));
        assert!(!is_markdown(Path::new("/d/md")));
    }

    #[test]
    fn a_token_of_another_kind_is_refused() {
        let dir = test_dir("kind");
        let path = file_with(&dir, "notes.md", b"x");
        let grants = PathGrants::default();
        for kind in [GrantKind::Save, GrantKind::Insert, GrantKind::Image] {
            let token = grants.grant(path.clone(), kind).token;
            assert!(read_markdown_file(&grants, &token).is_err(), "{kind:?}");
        }
    }

    #[test]
    fn an_unknown_token_or_a_path_is_refused() {
        let dir = test_dir("unknown");
        let path = file_with(&dir, "notes.md", b"x");
        let grants = PathGrants::default();
        grants.grant(path.clone(), GrantKind::Open);
        assert!(read_markdown_file(&grants, "0123456789abcdef0123456789abcdef").is_err());
        assert!(read_markdown_file(&grants, &path.to_string_lossy()).is_err());
    }

    #[test]
    fn an_expired_token_is_refused() {
        let dir = test_dir("expired");
        let path = file_with(&dir, "notes.md", b"x");
        let grants = PathGrants::default();
        let Some(long_ago) = Instant::now().checked_sub(GRANT_TTL + Duration::from_secs(1)) else {
            return;
        };
        let token = grants.grant_at(path, GrantKind::Open, long_ago).token;
        assert!(read_markdown_file(&grants, &token).is_err());
    }

    #[test]
    fn a_token_for_another_format_is_not_read_as_markdown() {
        let dir = test_dir("docx");
        let path = file_with(&dir, "report.docx", b"PK");
        let grants = PathGrants::default();
        let token = grants.grant(path, GrantKind::Open).token;
        assert!(read_markdown_file(&grants, &token).is_err());
    }

    #[test]
    fn a_file_over_the_limit_is_refused_with_its_own_error() {
        let dir = test_dir("large");
        let path = dir.join("big.md");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MARKDOWN_MAX_BYTES + 1).unwrap();
        let grants = PathGrants::default();
        let token = grants.grant(path.clone(), GrantKind::Open).token;
        let error = read_markdown_file(&grants, &token).unwrap_err();
        assert!(error.starts_with(TOO_LARGE_ERROR), "{error}");

        file.set_len(MARKDOWN_MAX_BYTES).unwrap();
        let token = grants.grant(path, GrantKind::Open).token;
        assert!(read_markdown_file(&grants, &token).is_ok());
    }

    #[test]
    fn text_over_the_limit_is_not_written() {
        let small = STANDARD.encode(b"# ok\n");
        assert_eq!(decode_markdown_data(&small).unwrap(), b"# ok\n");
        let big = STANDARD.encode(vec![b'a'; MARKDOWN_MAX_BYTES as usize + 3]);
        let error = decode_markdown_data(&big).unwrap_err();
        assert!(error.starts_with(TOO_LARGE_ERROR), "{error}");
    }

    #[test]
    fn saving_writes_the_bytes_with_their_bom_and_crlf() {
        let dir = test_dir("write");
        let dest = file_with(&dir, "notes.md", b"old\n");
        for bytes in [
            &b"# LF\n\ntext\n"[..],
            &b"# CRLF\r\n\r\ntext\r\n"[..],
            &b"\xEF\xBB\xBF# BOM\r\ntext"[..],
        ] {
            let data = decode_markdown_data(&STANDARD.encode(bytes)).unwrap();
            crate::save_target::write_bytes_beside_and_rename(&data, &dest).unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), bytes);
        }
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no temporary file is left behind");
    }

    #[test]
    fn save_goes_over_the_open_file_and_save_as_to_the_save_token() {
        let current = Some(PathBuf::from("/home/u/notes.md"));
        let grants = PathGrants::default();
        assert_eq!(
            markdown_save_destination(&grants, None, current.clone()),
            Ok(PathBuf::from("/home/u/notes.md"))
        );
        let chosen = grants.grant(PathBuf::from("/home/u/copy.markdown"), GrantKind::Save);
        assert_eq!(
            markdown_save_destination(&grants, Some(&chosen.token), current),
            Ok(PathBuf::from("/home/u/copy.markdown"))
        );
    }

    #[test]
    fn save_as_refuses_open_tokens_and_other_extensions() {
        let current = || Some(PathBuf::from("/home/u/notes.md"));
        let grants = PathGrants::default();
        let open = grants.grant(PathBuf::from("/home/u/other.md"), GrantKind::Open);
        assert!(markdown_save_destination(&grants, Some(&open.token), current()).is_err());
        let docx = grants.grant(PathBuf::from("/home/u/notes.docx"), GrantKind::Save);
        assert!(markdown_save_destination(&grants, Some(&docx.token), current()).is_err());
        assert!(markdown_save_destination(&grants, Some("/home/u/x.md"), current()).is_err());
    }

    #[test]
    fn a_new_document_is_saved_only_with_a_save_token() {
        let grants = PathGrants::default();
        assert!(markdown_save_destination(&grants, None, None).is_err());
        let chosen = grants.grant(PathBuf::from("/home/u/Untitled.md"), GrantKind::Save);
        assert_eq!(
            markdown_save_destination(&grants, Some(&chosen.token), None),
            Ok(PathBuf::from("/home/u/Untitled.md"))
        );
        let open = grants.grant(PathBuf::from("/home/u/other.md"), GrantKind::Open);
        assert!(markdown_save_destination(&grants, Some(&open.token), None).is_err());
        assert!(markdown_save_destination(&grants, Some("0123456789abcdef"), None).is_err());
    }

    #[test]
    fn html_is_exported_only_to_the_html_file_a_save_token_names() {
        let grants = PathGrants::default();
        for name in ["/home/u/notes.html", "/home/u/notes.HTM"] {
            let chosen = grants.grant(PathBuf::from(name), GrantKind::Save);
            assert_eq!(html_export_destination(&grants, &chosen.token), Ok(PathBuf::from(name)));
        }
        for kind in [GrantKind::Open, GrantKind::Insert, GrantKind::Image] {
            let other = grants.grant(PathBuf::from("/home/u/notes.html"), kind);
            assert!(html_export_destination(&grants, &other.token).is_err(), "{kind:?}");
        }
        let markdown = grants.grant(PathBuf::from("/home/u/notes.md"), GrantKind::Save);
        assert!(html_export_destination(&grants, &markdown.token).is_err());
        assert!(html_export_destination(&grants, "0123456789abcdef0123456789abcdef").is_err());
        assert!(html_export_destination(&grants, "/home/u/notes.html").is_err());
    }

    #[test]
    fn exported_html_over_its_limit_is_not_written() {
        let max = HTML_EXPORT_MAX_BYTES as usize;
        // check_base64_size estimates from whole groups of three bytes.
        let near_limit = max - max % 3;
        let at_limit = STANDARD.encode(vec![b'a'; near_limit]);
        assert_eq!(decode_limited("HTML", &at_limit, HTML_EXPORT_MAX_BYTES).unwrap().len(), near_limit);
        let over = STANDARD.encode(vec![b'a'; max + 3]);
        let error = decode_limited("HTML", &over, HTML_EXPORT_MAX_BYTES).unwrap_err();
        assert!(error.starts_with(TOO_LARGE_ERROR), "{error}");
    }

    #[test]
    fn exporting_writes_the_html_in_one_step() {
        let dir = test_dir("export");
        let dest = file_with(&dir, "notes.html", b"old");
        let html = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head></html>".as_bytes();
        let bytes = decode_limited("HTML", &STANDARD.encode(html), HTML_EXPORT_MAX_BYTES).unwrap();
        crate::save_target::write_bytes_beside_and_rename(&bytes, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), html);
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no temporary file is left behind");
    }

    #[test]
    fn only_an_open_markdown_file_is_saved_over() {
        assert!(open_markdown_file(None).is_err());
        assert!(open_markdown_file(Some(PathBuf::from("/home/u/report.docx"))).is_err());
        assert!(open_markdown_file(Some(PathBuf::from("/home/u/notes.MD"))).is_ok());
    }

    #[test]
    fn a_reload_finds_the_live_markdown_copy_only() {
        let root = test_dir("reload");
        assert_eq!(live_markdown_copy(None), None);
        let session = crate::recovery::begin_markdown_session(&root, None, "0.0.0", "1-1").unwrap();
        assert_eq!(live_markdown_copy(Some(&session)), None, "no text written yet");
        crate::recovery::write_markdown_copy(&session, b"# draft\n").unwrap();
        let copy = live_markdown_copy(Some(&session)).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"# draft\n");
        let editor = crate::recovery::RecoverySession { markdown: false, ..session };
        assert_eq!(live_markdown_copy(Some(&editor)), None, "an editor's session");
    }

    #[test]
    fn images_are_read_against_the_open_file_first() {
        let current = Some(PathBuf::from("/home/u/notes.md"));
        let suggestion = Some(PathBuf::from("/home/old/draft.md"));
        assert_eq!(
            image_reference_document(current, suggestion),
            Ok(PathBuf::from("/home/u/notes.md"))
        );
    }

    #[test]
    fn a_recovered_document_reads_images_against_its_markdown_suggestion() {
        let suggestion = Some(PathBuf::from("/home/old/draft.MD"));
        assert_eq!(
            image_reference_document(None, suggestion),
            Ok(PathBuf::from("/home/old/draft.MD"))
        );
    }

    #[test]
    fn a_suggestion_that_is_not_markdown_is_refused() {
        let suggestion = Some(PathBuf::from("/home/old/report.docx"));
        assert!(image_reference_document(None, suggestion).is_err());
    }

    #[test]
    fn a_new_document_has_no_image_folder() {
        assert!(image_reference_document(None, None).is_err());
    }

    #[test]
    fn a_recovered_document_finds_images_beside_its_suggestion() {
        let dir = test_dir("recovered-images");
        let image = file_with(&dir, "a.png", b"png");
        let suggested = dir.join("draft.md");
        let document = image_reference_document(None, Some(suggested)).unwrap();
        let canonical = std::fs::canonicalize(&image).unwrap();
        assert_eq!(markdown_image_path(&document, "a.png"), Ok(canonical));
        assert!(markdown_image_path(&document, "../a.png").is_err());
    }

    #[test]
    fn a_suggestion_whose_folder_is_gone_finds_no_image() {
        let dir = test_dir("gone");
        let suggested = dir.join("draft.md");
        std::fs::remove_dir_all(&dir).unwrap();
        let document = image_reference_document(None, Some(suggested)).unwrap();
        assert!(markdown_image_path(&document, "a.png").is_err());
    }

    #[test]
    fn images_inside_the_folder_are_found() {
        let dir = test_dir("images");
        let document = file_with(&dir, "notes.md", b"");
        std::fs::create_dir_all(dir.join("img")).unwrap();
        let image = file_with(&dir.join("img"), "a.png", b"png");
        let canonical = std::fs::canonicalize(&image).unwrap();
        assert_eq!(
            markdown_image_path(&document, "img/a.png"),
            Ok(canonical.clone())
        );
        assert_eq!(
            markdown_image_path(&document, "./img/../img/a.png"),
            Ok(canonical)
        );
        assert_eq!(image_mime(&image), Some("image/png"));
        assert_eq!(image_mime(Path::new("x.SVG")), Some("image/svg+xml"));
        assert_eq!(image_mime(Path::new("x.emf")), None);
    }

    #[test]
    fn images_outside_the_folder_are_refused() {
        let root = test_dir("outside");
        let docs = root.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        let document = file_with(&docs, "notes.md", b"");
        let secret = file_with(&root, "secret.png", b"png");
        for src in [
            "../secret.png",
            "img/../../secret.png",
            &secret.to_string_lossy().to_string(),
            "/etc/passwd",
            "\\\\server\\share\\a.png",
            "C:/Windows/a.png",
            "file:///etc/passwd",
            "https://example.com/a.png",
            "",
        ] {
            assert!(markdown_image_path(&document, src).is_err(), "{src}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_that_leaves_the_folder_is_refused() {
        let root = test_dir("symlink");
        let docs = root.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        let document = file_with(&docs, "notes.md", b"");
        file_with(&root, "secret.png", b"png");
        std::os::unix::fs::symlink(root.join("secret.png"), docs.join("link.png")).unwrap();
        std::os::unix::fs::symlink(&root, docs.join("up")).unwrap();
        assert!(markdown_image_path(&document, "link.png").is_err());
        assert!(markdown_image_path(&document, "up/secret.png").is_err());
    }

    #[test]
    fn an_image_over_its_limit_is_not_read() {
        let dir = test_dir("image-size");
        let image = file_with(&dir, "a.png", b"0123456789");
        assert!(read_limited(&image, 10).is_ok());
        assert!(read_limited(&image, 9).is_err());
        assert!(read_limited(&dir, 100).is_err(), "a folder is not read");
    }
}
