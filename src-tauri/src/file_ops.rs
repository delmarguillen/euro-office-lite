use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};

pub struct AppState {
    pub current_file: Mutex<Option<PathBuf>>,
    pub temp_dir: PathBuf,
    pub modified: Mutex<bool>,
}

fn log_event(state: &AppState, msg: &str) {
    println!("{}", msg);
    use std::io::Write;
    let log_path = state.temp_dir.join("js-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "{}", msg);
    }
}

fn clear_changes(temp_dir: &std::path::Path) {
    let changes_dir = temp_dir.join("changes");
    if changes_dir.exists() {
        let _ = std::fs::remove_dir_all(&changes_dir);
    }
}

// media/ holds the images inserted from disk (copy-to-media in main.rs) and x2t
// resolves them from there on every export, so it must outlive a PDF export and
// only goes away when the open document changes. Clearing it before an export
// left x2t unable to load them, and the editor drew each one as a solid black
// rectangle (#31).
// insert_tmp/ and downloads/ hold what the previous document pulled in from a
// compare, a merge or a url. They belong to that document just like media/ does,
// and nothing else ever emptied them: the temp dir is fixed, so they grew across
// documents and across sessions, and a stale image stayed reachable through
// ascdesktop://abs/ while a different document was open.
fn clear_document_temp(temp_dir: &std::path::Path) {
    clear_changes(temp_dir);
    for dir in ["media", "insert_tmp", "downloads"] {
        let path = temp_dir.join(dir);
        if path.exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

// Stages an image into media/, where x2t resolves it from on every export, and
// returns the name the document must reference.
//
// The name is not always the source's: media/ is shared by everything the
// document pulls in, and x2t already fills it with image1.png, image2.png...
// when opening a document, so two unrelated images landing on the same name is
// the common case, not the rare one. Copying over the existing file would break
// the document that referenced it first, and skipping the copy (what
// copy-to-media did until now) served the first image in place of the second.
// So: same content under that name means it is already staged and the name is
// reused, which keeps repeated calls for the same source idempotent; different
// content gets a _1, _2... suffix.
pub fn stage_into_media(media_dir: &std::path::Path, src: &std::path::Path) -> Option<String> {
    let file_name = src.file_name()?.to_string_lossy().to_string();
    let bytes = std::fs::read(src).ok()?;
    stage_bytes_into_media(media_dir, &file_name, &bytes)
}

// Same staging, for callers that already hold the bytes and have no file to
// read: a downloaded image only becomes a file once it has a name, and the name
// is what this decides.
pub fn stage_bytes_into_media(
    media_dir: &std::path::Path,
    file_name: &str,
    bytes: &[u8],
) -> Option<String> {
    let file_name = file_name.to_string();

    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{}", e)),
        _ => (file_name.clone(), String::new()),
    };

    let mut candidate = file_name;
    let mut n = 0u32;
    loop {
        let dest = media_dir.join(&candidate);
        match std::fs::read(&dest) {
            // Free name: stage it here.
            Err(_) => {
                std::fs::write(&dest, bytes).ok()?;
                return Some(candidate);
            }
            // Already staged, byte for byte. Reusing the name is what makes a
            // second call for the same file return the same answer.
            Ok(existing) if existing == bytes => return Some(candidate),
            Ok(_) => {
                n += 1;
                candidate = format!("{}_{}{}", stem, n, ext);
            }
        }
    }
}

// Copies the images x2t extracted from an inserted document into the media/ the
// exporter reads, and reports each one as (name the inserted binary references,
// name it ended up under). The two differ whenever the host document already
// staged something else under that name, which is the normal case: x2t numbers
// media image1, image2... per document, so an inserted document arrives with the
// same names the host document already used.
//
// Copies rather than moves, on purpose. sdkjs does not always register the map
// this feeds it and then asks copy-to-media for the bare name instead, which is
// resolved against insert_tmp/media/ (main.rs): the original has to still be
// sitting there for that second request to find anything. Content keying is what
// makes it answer with the name assigned here instead of staging a duplicate.
pub fn stage_insert_media(
    insert_media_dir: &std::path::Path,
    doc_media_dir: &std::path::Path,
) -> Vec<(String, String)> {
    let mut staged = Vec::new();
    let Ok(entries) = std::fs::read_dir(insert_media_dir) else {
        return staged;
    };
    let _ = std::fs::create_dir_all(doc_media_dir);
    // Sorted, so image1, image2... keep their order and a rerun of the same
    // insert reproduces the same names.
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for img_path in paths {
        let name = img_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if let Some(final_name) = stage_into_media(doc_media_dir, &img_path) {
            staged.push((name, final_name));
        }
    }
    staged
}

#[tauri::command]
pub async fn open_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    open_file_inner(app, state, path, true).await
}

// Every frontend open path funnels through open_file, so the recent list is fed
// here rather than in JS. create_new converts a bundled blank template through
// the same code and must stay out of the list, hence the flag.
async fn open_file_inner(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    record_recent: bool,
) -> Result<String, String> {
    let input = PathBuf::from(&path);
    let output = state.temp_dir.join("Editor.bin");

    clear_document_temp(&state.temp_dir);

    let format_from = detect_format(&input);
    let format_to = 8192;
    let file_name = input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");

    log_event(
        &state,
        &format!("[OPEN] start file={} format={}", file_name, format_from),
    );

    super::converter::convert_file(
        &app,
        &path,
        &output.to_string_lossy(),
        format_from,
        format_to,
        &state.temp_dir.to_string_lossy(),
    )
    .await
    .map_err(|error| {
        log_event(
            &state,
            &format!("[OPEN] failed file={} error={}", file_name, error),
        );
        error
    })?;

    let bin_size = std::fs::metadata(&output)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let media_count = std::fs::read_dir(state.temp_dir.join("media"))
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(0);
    log_event(
        &state,
        &format!(
            "[OPEN] success editor_bin={} bytes media_files={}",
            bin_size, media_count
        ),
    );

    let bin_data = std::fs::read(&output).map_err(|e| e.to_string())?;
    let b64 = STANDARD.encode(&bin_data);

    *state.current_file.lock().unwrap() = Some(input);
    *state.modified.lock().unwrap() = false;

    if record_recent {
        super::recent::record(&app, &path);
    }

    Ok(b64)
}

#[tauri::command]
pub async fn save_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    _data: String,
) -> Result<String, String> {
    let current = state.current_file.lock().unwrap().clone();
    let dest = current.ok_or("No file is currently open")?;

    let input = state.temp_dir.join("Editor.bin");
    let format_from = 8192;
    let format_to = detect_format(&dest);

    super::converter::convert_file(
        &app,
        &input.to_string_lossy(),
        &dest.to_string_lossy(),
        format_from,
        format_to,
        &state.temp_dir.to_string_lossy(),
    )
    .await?;

    *state.modified.lock().unwrap() = false;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn save_file_as(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let dest = PathBuf::from(&path);
    let input = state.temp_dir.join("Editor.bin");
    let format_from = 8192;
    let format_to = detect_format(&dest);

    if format_to == 513 {
        clear_changes(&state.temp_dir);
    }

    super::converter::convert_file(
        &app,
        &input.to_string_lossy(),
        &dest.to_string_lossy(),
        format_from,
        format_to,
        &state.temp_dir.to_string_lossy(),
    )
    .await?;

    // A PDF export leaves the document itself untouched (current_file keeps
    // pointing at the editable file), so it does not belong in the list either.
    if format_to != 513 {
        *state.current_file.lock().unwrap() = Some(dest);
        *state.modified.lock().unwrap() = false;
        super::recent::record(&app, &path);
    }
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn save_changes(
    state: State<'_, AppState>,
    changes: String,
    _delete_index: Option<i32>,
    count: i32,
) -> Result<String, String> {
    let changes_dir = state.temp_dir.join("changes");
    std::fs::create_dir_all(&changes_dir).map_err(|e| e.to_string())?;

    let filename = format!("change_{}.json", count);
    std::fs::write(changes_dir.join(&filename), &changes).map_err(|e| e.to_string())?;

    Ok("ok".to_string())
}

#[tauri::command]
pub async fn write_editor_bin(
    state: State<'_, AppState>,
    data: String,
) -> Result<String, String> {
    let bin_data = STANDARD.decode(&data).map_err(|e| e.to_string())?;
    let output = state.temp_dir.join("Editor.bin");
    std::fs::write(&output, &bin_data).map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn print_document(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let editor_bin = state.temp_dir.join("Editor.bin");
    if !editor_bin.exists() {
        log_event(&state, "[PRINT] failed: Editor.bin not found");
        return Err("Editor.bin not found".to_string());
    }

    let bin_size = std::fs::metadata(&editor_bin)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    log_event(
        &state,
        &format!("[PRINT] start editor_bin={} bytes", bin_size),
    );

    let pdf_path = state.temp_dir.join("print_output.pdf");
    if pdf_path.exists() {
        let _ = std::fs::remove_file(&pdf_path);
    }
    clear_changes(&state.temp_dir);

    super::converter::convert_file(
        &app,
        &editor_bin.to_string_lossy(),
        &pdf_path.to_string_lossy(),
        8192,
        513,
        &state.temp_dir.to_string_lossy(),
    )
    .await
    .map_err(|error| {
        log_event(&state, &format!("[PRINT] failed: {}", error));
        error
    })?;

    let pdf_size = std::fs::metadata(&pdf_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if pdf_size == 0 {
        log_event(&state, "[PRINT] failed: generated PDF is empty");
        return Err("PDF file is empty".to_string());
    }

    log_event(
        &state,
        &format!("[PRINT] success pdf_size={} bytes", pdf_size),
    );
    Ok(pdf_path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_pdf_viewer(state: State<'_, AppState>, path: String) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/c", "start", "", &path])
        .spawn();

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg(&path)
        .spawn();

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn();

    result.map_err(|e| {
        log_event(&state, &format!("[PRINT] viewer failed: {}", e));
        e.to_string()
    })?;
    Ok("ok".to_string())
}

#[tauri::command]
pub fn get_current_path(state: State<'_, AppState>) -> Option<String> {
    state
        .current_file
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn create_new(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    doc_type: String,
) -> Result<String, String> {
    let template = match doc_type.as_str() {
        "word" => "templates/blank.docx",
        "cell" => "templates/blank.xlsx",
        "slide" => "templates/blank.pptx",
        _ => return Err(format!("Unknown type: {}", doc_type)),
    };

    let template_path = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join(template);

    *state.current_file.lock().unwrap() = None;
    *state.modified.lock().unwrap() = false;

    let result = open_file_inner(
        app,
        state.clone(),
        template_path.to_string_lossy().to_string(),
        false,
    )
    .await;
    *state.current_file.lock().unwrap() = None;
    result
}


#[tauri::command]
pub fn write_download_temp(
    state: State<'_, AppState>,
    data: String,
    url: String,
) -> Result<String, String> {
    let download_dir = state.temp_dir.join("downloads");
    let _ = std::fs::create_dir_all(&download_dir);

    let file_name = url
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .split('?')
        .next()
        .unwrap_or("download");
    let safe_name = file_name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
        .collect::<String>();
    let dest = download_dir.join(if safe_name.is_empty() {
        "download".to_string()
    } else {
        safe_name
    });

    let bytes = STANDARD.decode(&data).map_err(|e| e.to_string())?;
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn convert_for_insert(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    let input = PathBuf::from(&path);
    let insert_dir = state.temp_dir.join("insert_tmp");
    let _ = std::fs::create_dir_all(&insert_dir);

    let output = insert_dir.join("Editor.bin");

    let format_from = detect_format(&input);
    let format_to = 8192;

    super::converter::convert_file(
        &app,
        &path,
        &output.to_string_lossy(),
        format_from,
        format_to,
        &insert_dir.to_string_lossy(),
    )
    .await?;

    let bin_data = std::fs::read(&output).map_err(|e| e.to_string())?;
    let b64 = STANDARD.encode(&bin_data);

    // x2t drops the inserted document's images in insert_tmp/media/, but the
    // exporter only ever looks in temp_dir/media/, so anything left here came out
    // of the PDF as a black rectangle: the same #31 mechanism through another
    // door. Staging them into media/ is what makes them exportable, and it has to
    // go through the collision-safe primitive rather than copy under the original
    // name, because collision is the normal case here and not an edge one: x2t
    // names these image1.png, image2.png... per document, which is exactly what it
    // already named the host document's own images.
    let mut images = serde_json::Map::new();
    for (name, staged) in stage_insert_media(&insert_dir.join("media"), &state.temp_dir.join("media"))
    {
        let img_url = format!("ascdesktop://docmedia/media/{}", staged);
        images.insert(name, serde_json::Value::String(img_url));
    }

    Ok(serde_json::json!({
        "data": b64,
        "images": images
    }))
}

#[tauri::command]
pub fn get_system_fonts(state: State<'_, AppState>) -> Result<String, String> {
    let path = state.temp_dir.join("fontdata").join("AllFonts.js");
    std::fs::read_to_string(&path).map_err(|e| format!("Cannot read AllFonts.js: {}", e))
}

// x2t format ids, shared with the editors: web-apps uses the same numbers in
// utils.defines.FileFormat to pick the icon and to filter the Open Recent list.
pub fn detect_format(path: &PathBuf) -> i32 {
    match path.extension().and_then(|e| e.to_str()) {
        Some("docx") => 65,
        Some("doc") => 66,
        Some("odt") => 67,
        Some("rtf") => 68,
        Some("txt") => 69,
        Some("xlsx") => 257,
        Some("xls") => 258,
        Some("ods") => 259,
        Some("csv") => 260,
        Some("pptx") => 129,
        Some("ppt") => 130,
        Some("odp") => 131,
        Some("pdf") => 513,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    // A throwaway temp dir laid out like the app's: changes/ with one pending
    // change and media/ with one inserted image.
    fn temp_dir_with_changes_and_media() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-file-ops-test-{}-{}",
            std::process::id(),
            unique
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("changes")).unwrap();
        std::fs::create_dir_all(dir.join("media")).unwrap();
        std::fs::write(dir.join("changes/change_0.json"), b"[]").unwrap();
        std::fs::write(dir.join("media/image1.png"), b"not really a png").unwrap();
        dir
    }

    // The #31 regression: the PDF paths (save_file_as with format_to == 513, and
    // print_document) call clear_changes right before handing the document to
    // x2t. If that call also wiped media/, x2t could no longer load the images
    // and drew each one as a solid black rectangle.
    #[test]
    fn clear_changes_keeps_media() {
        let dir = temp_dir_with_changes_and_media();

        clear_changes(&dir);

        assert!(
            !dir.join("changes").exists(),
            "clear_changes must drop pending changes"
        );
        assert!(
            dir.join("media/image1.png").exists(),
            "clear_changes must keep inserted images: x2t resolves them from \
             media/ on every export (#31)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Switching documents does invalidate the images, so the full reset still
    // has to take media/ with it.
    #[test]
    fn clear_document_temp_drops_media() {
        let dir = temp_dir_with_changes_and_media();

        clear_document_temp(&dir);

        assert!(!dir.join("changes").exists());
        assert!(
            !dir.join("media").exists(),
            "opening another document must not leave the previous document's images behind"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Both run on temp dirs that a fresh profile has not created yet.
    #[test]
    fn clearing_is_a_noop_when_nothing_exists() {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-file-ops-test-empty-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();

        clear_changes(&dir);
        clear_document_temp(&dir);

        assert!(dir.exists(), "clearing must not remove the temp dir itself");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn staging_dir() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-stage-test-{}-{}",
            std::process::id(),
            unique
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("media")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    // Two different images that happen to share a file name. Skipping the copy
    // (what copy-to-media did) answered with the same name twice and the second
    // image was shown, and exported, as the first one.
    #[test]
    fn staging_a_colliding_name_keeps_both_images() {
        let dir = staging_dir();
        let media = dir.join("media");
        std::fs::create_dir_all(dir.join("src/a")).unwrap();
        std::fs::create_dir_all(dir.join("src/b")).unwrap();
        std::fs::write(dir.join("src/a/photo.png"), b"red image").unwrap();
        std::fs::write(dir.join("src/b/photo.png"), b"blue image").unwrap();

        let first = stage_into_media(&media, &dir.join("src/a/photo.png")).unwrap();
        let second = stage_into_media(&media, &dir.join("src/b/photo.png")).unwrap();

        assert_eq!(first, "photo.png");
        assert_ne!(
            second, first,
            "a different image must not be handed the name of one already staged"
        );
        assert_eq!(second, "photo_1.png");
        assert_eq!(std::fs::read(media.join(&first)).unwrap(), b"red image");
        assert_eq!(std::fs::read(media.join(&second)).unwrap(), b"blue image");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // The skip that caused the bug above was there for a reason: sdkjs asks for
    // the same file more than once and must keep getting the same name back.
    #[test]
    fn staging_the_same_file_twice_returns_the_same_name() {
        let dir = staging_dir();
        let media = dir.join("media");
        let src = dir.join("src/photo.png");
        std::fs::write(&src, b"red image").unwrap();

        assert_eq!(stage_into_media(&media, &src).unwrap(), "photo.png");
        assert_eq!(stage_into_media(&media, &src).unwrap(), "photo.png");
        assert_eq!(
            std::fs::read_dir(&media).unwrap().count(),
            1,
            "an unchanged source must not pile up copies"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // The realistic collision: x2t names a document's own media image1.png,
    // image2.png..., so anything inserted under those names lands on top.
    #[test]
    fn staging_walks_past_every_taken_name() {
        let dir = staging_dir();
        let media = dir.join("media");
        std::fs::write(media.join("image1.png"), b"host document image").unwrap();
        std::fs::write(media.join("image1_1.png"), b"another one").unwrap();
        let src = dir.join("src/image1.png");
        std::fs::write(&src, b"inserted image").unwrap();

        assert_eq!(stage_into_media(&media, &src).unwrap(), "image1_2.png");
        assert_eq!(
            std::fs::read(media.join("image1.png")).unwrap(),
            b"host document image",
            "the host document's own image must survive untouched"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn staging_handles_names_without_an_extension() {
        let dir = staging_dir();
        let media = dir.join("media");
        std::fs::write(media.join("clip"), b"first").unwrap();
        let src = dir.join("src/clip");
        std::fs::write(&src, b"second").unwrap();

        assert_eq!(stage_into_media(&media, &src).unwrap(), "clip_1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // An unreadable source must report failure, not a name pointing at nothing:
    // the caller answers 500 and the bridge falls back to the original url.
    // download-to-media used to write the downloaded bytes straight over
    // media/<name>, so a download named like an image already in there replaced
    // it, and the reply still carried the original name.
    #[test]
    fn staging_bytes_never_overwrites_a_staged_image() {
        let dir = staging_dir();
        let media = dir.join("media");
        std::fs::write(media.join("image1.png"), b"host document image").unwrap();

        let name = stage_bytes_into_media(&media, "image1.png", b"downloaded image").unwrap();

        assert_eq!(name, "image1_1.png");
        assert_eq!(
            std::fs::read(media.join("image1.png")).unwrap(),
            b"host document image"
        );
        assert_eq!(
            std::fs::read(media.join(&name)).unwrap(),
            b"downloaded image"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Both entry points have to agree, or a file staged from disk and the same
    // file downloaded would end up duplicated under two names.
    #[test]
    fn staging_bytes_and_staging_a_file_agree() {
        let dir = staging_dir();
        let media = dir.join("media");
        let src = dir.join("src/photo.png");
        std::fs::write(&src, b"same image").unwrap();

        let from_file = stage_into_media(&media, &src).unwrap();
        let from_bytes = stage_bytes_into_media(&media, "photo.png", b"same image").unwrap();

        assert_eq!(from_file, from_bytes);
        assert_eq!(std::fs::read_dir(&media).unwrap().count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn staging_a_missing_source_fails() {
        let dir = staging_dir();
        assert!(stage_into_media(&dir.join("media"), &dir.join("src/gone.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The residual of #31: images of a compared or merged document stayed in
    // insert_tmp/media/, which the exporter never reads, so they came out of the
    // PDF as black rectangles.
    #[test]
    fn insert_media_lands_where_the_exporter_reads_it() {
        let dir = staging_dir();
        let media = dir.join("media");
        let insert = dir.join("insert_tmp/media");
        std::fs::create_dir_all(&insert).unwrap();
        std::fs::write(insert.join("image1.png"), b"compared image").unwrap();

        let staged = stage_insert_media(&insert, &media);

        assert_eq!(staged, vec![("image1.png".into(), "image1.png".into())]);
        assert_eq!(
            std::fs::read(media.join("image1.png")).unwrap(),
            b"compared image",
            "x2t only resolves media/ when exporting, not insert_tmp/media/"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Copying under the original name would look fine against a blank host
    // document and quietly serve the wrong image against a real one: x2t numbers
    // both documents' images image1, image2...
    #[test]
    fn insert_media_does_not_take_over_the_host_document_names() {
        let dir = staging_dir();
        let media = dir.join("media");
        let insert = dir.join("insert_tmp/media");
        std::fs::create_dir_all(&insert).unwrap();
        std::fs::write(media.join("image1.png"), b"host image").unwrap();
        std::fs::write(insert.join("image1.png"), b"compared image").unwrap();
        std::fs::write(insert.join("image2.png"), b"second compared image").unwrap();

        let staged = stage_insert_media(&insert, &media);

        assert_eq!(
            staged,
            vec![
                ("image1.png".to_string(), "image1_1.png".to_string()),
                ("image2.png".to_string(), "image2.png".to_string()),
            ],
            "the map has to report the name each image ended up under"
        );
        assert_eq!(
            std::fs::read(media.join("image1.png")).unwrap(),
            b"host image",
            "the host document must keep its own image"
        );
        assert_eq!(
            std::fs::read(media.join("image1_1.png")).unwrap(),
            b"compared image"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // sdkjs does not register the url map after a compare and asks for the bare
    // name instead, which lands back on staging through copy-to-media. Content
    // keying is what makes that second request answer with the name already
    // assigned rather than a fresh copy.
    #[test]
    fn asking_again_for_an_inserted_image_returns_the_staged_name() {
        let dir = staging_dir();
        let media = dir.join("media");
        let insert = dir.join("insert_tmp/media");
        std::fs::create_dir_all(&insert).unwrap();
        std::fs::write(media.join("image1.png"), b"host image").unwrap();
        std::fs::write(insert.join("image1.png"), b"compared image").unwrap();

        let staged = stage_insert_media(&insert, &media);
        let asked_again = stage_into_media(&media, &insert.join("image1.png")).unwrap();

        assert_eq!(asked_again, staged[0].1);
        assert_eq!(
            std::fs::read_dir(&media).unwrap().count(),
            2,
            "the second request must not add a third file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn insert_media_is_a_noop_without_images() {
        let dir = staging_dir();
        assert!(stage_insert_media(&dir.join("insert_tmp/media"), &dir.join("media")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // insert_tmp/ and downloads/ belong to the open document just like media/,
    // and until now nothing ever emptied them.
    #[test]
    fn clear_document_temp_drops_inserted_and_downloaded_files() {
        let dir = temp_dir_with_changes_and_media();
        std::fs::create_dir_all(dir.join("insert_tmp/media")).unwrap();
        std::fs::create_dir_all(dir.join("downloads")).unwrap();
        std::fs::write(dir.join("insert_tmp/media/image1.png"), b"compared").unwrap();
        std::fs::write(dir.join("downloads/photo.png"), b"downloaded").unwrap();

        clear_document_temp(&dir);

        assert!(!dir.join("insert_tmp").exists());
        assert!(!dir.join("downloads").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_format_maps_pdf_to_the_x2t_export_id() {
        // 513 is the value both PDF paths branch on before calling the converter.
        assert_eq!(detect_format(&PathBuf::from("/tmp/out.pdf")), 513);
        assert_eq!(detect_format(&PathBuf::from("/tmp/out.docx")), 65);
        assert_eq!(detect_format(&PathBuf::from("/tmp/out.unknown")), 0);
    }
}
