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

// Save writes over the open file; Save As writes exactly the file its token
// names. A name not ending in .md or .markdown is refused, not renamed: the
// renamed file was never confirmed in the dialog and may already exist.
fn markdown_save_destination(
    grants: &PathGrants,
    token: Option<&str>,
    current: &Path,
) -> Result<PathBuf, String> {
    let Some(token) = token else {
        return Ok(current.to_path_buf());
    };
    let chosen = grants.resolve(token, GrantKind::Save)?;
    if !is_markdown(&chosen) {
        return Err("Markdown can only be saved to a name ending in .md or .markdown".to_string());
    }
    Ok(chosen)
}

fn decode_markdown_data(data: &str) -> Result<Vec<u8>, String> {
    let max = MARKDOWN_MAX_BYTES as usize;
    if crate::protocol_limits::check_base64_size("Markdown text", data, max).is_err() {
        return Err(too_large(MARKDOWN_MAX_BYTES));
    }
    let bytes = STANDARD.decode(data).map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err(too_large(MARKDOWN_MAX_BYTES));
    }
    Ok(bytes)
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

// Opens the Markdown file an Open token names: it becomes the open document
// (the close warning, Save, Recents) and its bytes go to the page in base64.
#[tauri::command]
pub async fn read_markdown(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    token: String,
) -> Result<MarkdownFile, String> {
    let (path, bytes) = read_markdown_file(&state.grants, &token).map_err(|error| {
        log_event(&state, &format!("[OPEN] markdown failed: {}", error));
        error
    })?;
    crate::recovery::end_session(&state, true);
    *state.pending_recent.lock().unwrap() = None;
    *state.current_file.lock().unwrap() = Some(path.clone());
    *state.modified.lock().unwrap() = false;
    *state.text_options.lock().unwrap() = None;
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

// Writes `data` (base64) as it is: over the open Markdown file, or to the file
// a Save token names. Returns the saved file's name.
#[tauri::command]
pub async fn save_markdown(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    data: String,
    token: Option<String>,
) -> Result<String, String> {
    let current = open_markdown_file(state.current_file.lock().unwrap().clone())?;
    let dest = markdown_save_destination(&state.grants, token.as_deref(), &current)?;
    let written = decode_markdown_data(&data)
        .and_then(|bytes| crate::save_target::write_bytes_beside_and_rename(&bytes, &dest));
    if let Err(error) = written {
        log_event(&state, &format!("[SAVE] markdown failed: {}", error));
        return Err(error);
    }
    if dest != current {
        *state.current_file.lock().unwrap() = Some(dest.clone());
        crate::recent::record(&app, &dest.to_string_lossy());
    }
    *state.modified.lock().unwrap() = false;
    log_event(&state, "[SAVE] markdown success");
    Ok(file_name(&dest))
}

// An image of the open Markdown file, as a data: URL's parts. `src` is the
// reference written in the document, resolved by markdown_image_path.
#[tauri::command]
pub async fn markdown_image(
    state: State<'_, AppState>,
    src: String,
) -> Result<MarkdownImage, String> {
    let document = open_markdown_file(state.current_file.lock().unwrap().clone())?;
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
        let current = PathBuf::from("/home/u/notes.md");
        let grants = PathGrants::default();
        assert_eq!(
            markdown_save_destination(&grants, None, &current),
            Ok(current.clone())
        );
        let chosen = grants.grant(PathBuf::from("/home/u/copy.markdown"), GrantKind::Save);
        assert_eq!(
            markdown_save_destination(&grants, Some(&chosen.token), &current),
            Ok(PathBuf::from("/home/u/copy.markdown"))
        );
    }

    #[test]
    fn save_as_refuses_open_tokens_and_other_extensions() {
        let current = PathBuf::from("/home/u/notes.md");
        let grants = PathGrants::default();
        let open = grants.grant(PathBuf::from("/home/u/other.md"), GrantKind::Open);
        assert!(markdown_save_destination(&grants, Some(&open.token), &current).is_err());
        let docx = grants.grant(PathBuf::from("/home/u/notes.docx"), GrantKind::Save);
        assert!(markdown_save_destination(&grants, Some(&docx.token), &current).is_err());
        assert!(markdown_save_destination(&grants, Some("/home/u/x.md"), &current).is_err());
    }

    #[test]
    fn only_an_open_markdown_file_is_saved_over() {
        assert!(open_markdown_file(None).is_err());
        assert!(open_markdown_file(Some(PathBuf::from("/home/u/report.docx"))).is_err());
        assert!(open_markdown_file(Some(PathBuf::from("/home/u/notes.MD"))).is_ok());
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
