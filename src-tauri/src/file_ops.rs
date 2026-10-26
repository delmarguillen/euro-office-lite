use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Manager, State};

pub use crate::x2t_task::{blank_template_for, detect_format, is_empty_file};

pub struct AppState {
    pub current_file: Mutex<Option<PathBuf>>,
    // This instance's own folder: everything that belongs to the open
    // document (temp_layout.rs).
    pub temp_dir: PathBuf,
    // Shared by every instance: the font cache and the logs.
    pub temp_root: PathBuf,
    pub instance: crate::temp_layout::Instance,
    // Number of the document open in this instance: part of its images'
    // addresses (temp_layout.rs, "Document images").
    pub doc_seq: std::sync::atomic::AtomicU64,
    pub modified: Mutex<bool>,
    // The path a 0-byte file was opened from (#33). It is not in the recent
    // list yet: the file still has no content, and an entry that opens to
    // nothing is worse than no entry. The first save that actually writes it
    // records it.
    pub pending_recent: Mutex<Option<PathBuf>>,
    // Crash recovery session of the open document, if any (recovery.rs).
    pub recovery: Mutex<Option<crate::recovery::RecoverySession>>,
    // Path passed as a launch argument that the frontend has not opened yet.
    pub pending_open: Mutex<Option<String>>,
    // Encoding, delimiter and line breaks detected when the open document came
    // from a CSV or TXT, so saving it back writes the file the way it was read.
    pub text_options: Mutex<Option<crate::text_import::TextOptions>>,
    // The PDF the last successful print wrote: the only file open_pdf_viewer
    // hands to the system viewer. Cleared when a print starts.
    pub last_print_pdf: Mutex<Option<PathBuf>>,
    // Images the user chose in the insert dialog: the only absolute paths
    // copy-to-media reads.
    pub picked_images: Mutex<Vec<PathBuf>>,
}

pub(crate) fn log_event(_state: &AppState, msg: &str) {
    println!("{}", msg);
    crate::temp_layout::append_log(msg);
}

pub(crate) fn open_document(state: &AppState) -> crate::temp_layout::OpenDocument<'_> {
    crate::temp_layout::OpenDocument {
        root: &state.temp_root,
        id: &state.instance.id,
        dir: &state.temp_dir,
        seq: state.doc_seq.load(std::sync::atomic::Ordering::SeqCst),
    }
}

// The instance is about to hold another document. Its images get a new
// address, the outgoing ones stay pasteable under theirs, and the rest of the
// outgoing document's files go as before. Err stops the open: media/ could not
// be emptied, and the next document must not land among the old images.
pub(crate) fn switch_document(state: &AppState) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let old = state.doc_seq.load(Ordering::SeqCst);
    match crate::temp_layout::begin_document(&state.temp_dir, old, old + 1) {
        Ok(outcome) => {
            if let crate::temp_layout::OutgoingImages::Dropped(reason) = outcome {
                log_event(
                    state,
                    &format!("[OPEN] previous document's images not kept: {}", reason),
                );
            }
            state.doc_seq.store(old + 1, Ordering::SeqCst);
        }
        Err(error) => {
            log_event(state, &format!("[OPEN] failed error={}", error));
            return Err(error);
        }
    }
    clear_document_temp(&state.temp_dir);
    Ok(())
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
// changes/ is in the list only to sweep what older versions left there: the
// editor's changes now live in the recovery session folder (recovery.rs), never
// in the temp dir.
pub(crate) fn clear_document_temp(temp_dir: &std::path::Path) {
    for dir in ["changes", "media", "insert_tmp", "downloads"] {
        let path = temp_dir.join(dir);
        if path.exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
    clear_x2t_scratch(temp_dir);
}

// x2t unpacks the document it is converting into <ext>_unpacked/ inside the temp
// dir it is given, under a name that depends only on the extension, and leaves it
// there. The folder outlives each document, so that directory is still sitting
// there the next time a document of the same type is opened.
//
// That is what made opening a 0-byte file show the previous document (#33):
// x2t cannot unpack an empty file, but it finds the leftover docx_unpacked/,
// converts THAT, and exits 0 — so the conversion reports success and hands back
// an Editor.bin holding the previous document. Verified on the test machine with
// the same binary and the same params: a clean temp dir exits 80 and writes
// nothing, one holding the previous docx_unpacked/ exits 0 and writes the
// previous document byte for byte.
fn clear_x2t_scratch(temp_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(temp_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        if is_x2t_scratch_dir(&entry.file_name().to_string_lossy()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

// The app's own directories in the temp dir (media, changes, fontdata,
// x2t-workdir, insert_tmp, downloads) must survive this: they are cleared, kept
// or rebuilt on their own terms. Only what x2t scatters goes.
fn is_x2t_scratch_dir(name: &str) -> bool {
    // docx_unpacked, xlsx_unpacked, pptx_unpacked...
    name.ends_with("_unpacked")
        // Randomly named scratch dirs x2t creates per run (asc9gz1zK and the
        // like). Nothing of ours starts with "asc".
        || name.starts_with("asc")
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

    clear_pending_recent(&state.pending_recent);

    // The document on screen is being replaced, so its recovery folder goes
    // with it: by this point the frontend has confirmed there was nothing to
    // lose, or the user chose to discard. The new document opens its own
    // session from the bridge, once the editor has the bytes.
    crate::recovery::end_session(&state, true);

    // An empty file has nothing to convert, so a blank template of the matching
    // type is converted instead and current_file still points at the file the
    // user opened: the first save writes the document there, in the format its
    // extension asks for (#33). The check is a stat taken here, so a file that
    // stopped being empty between the double click and this call opens normally.
    let mut source = path.clone();
    let mut opened_blank = false;
    if is_empty_file(&input) {
        if let Some(template) = blank_template_for(&input) {
            let template_path = app
                .path()
                .resource_dir()
                .map_err(|e| e.to_string())?
                .join(template);
            source = template_path.to_string_lossy().to_string();
            opened_blank = true;
        }
    }

    switch_document(&state)?;

    // Editor.bin also outlives the process, and it is read back after the
    // conversion. Dropping it first means a conversion that reports success
    // without writing anything can no longer serve the previous document (#33):
    // there is nothing left to serve.
    let _ = std::fs::remove_file(&output);

    let format_from = detect_format(&PathBuf::from(&source));
    let format_to = 8192;
    let file_name = input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");

    let text_kind = crate::text_import::TextKind::for_path(Path::new(&source));
    let text_options = match text_kind {
        Some(kind) => Some(
            crate::text_import::detect_file(Path::new(&source), kind).map_err(|e| {
                log_event(
                    &state,
                    &format!("[OPEN] failed file={} error=detect: {}", file_name, e),
                );
                e.to_string()
            })?,
        ),
        None => None,
    };

    log_event(
        &state,
        &format!(
            "[OPEN] start file={} format={}{}{}",
            file_name,
            format_from,
            if opened_blank { " empty=blank" } else { "" },
            text_options
                .map(|o| format!(
                    " encoding={} delimiter={:?} crlf={} trailing_breaks={}",
                    o.encoding, o.delimiter, o.crlf, o.trailing_breaks
                ))
                .unwrap_or_default()
        ),
    );

    // x2t's UTF-8 CSV reader adds an empty row after a final line break, and it
    // loses the text of a UTF-16 TXT, so those are read from a copy that avoids
    // both. current_file and the recent list keep the original path, and saving
    // keeps the original's options; only the conversion sees the copy.
    let read_copy = match crate::text_import::prepare_read_copy(
        Path::new(&source),
        &state.temp_dir,
        text_kind,
        text_options,
    ) {
        Ok(copy) => copy,
        Err(e) => {
            log_event(
                &state,
                &format!("[OPEN] failed file={} error=read copy: {}", file_name, e),
            );
            return Err(e.to_string());
        }
    };
    let convert_from = read_copy
        .as_ref()
        .map(|copy| copy.path.to_string_lossy().to_string())
        .unwrap_or_else(|| source.clone());
    let convert_options = read_copy.as_ref().map(|copy| copy.options).or(text_options);

    let converted = super::converter::convert_file(
        &app,
        &convert_from,
        &output.to_string_lossy(),
        format_from,
        format_to,
        &state.temp_dir.to_string_lossy(),
        &crate::temp_layout::fontdata_dir(&state.temp_root),
        convert_options,
    )
    .await;
    if let Some(copy) = &read_copy {
        let _ = std::fs::remove_file(&copy.path);
    }
    converted.map_err(|error| {
        log_event(
            &state,
            &format!("[OPEN] failed file={} error={}", file_name, error),
        );
        error
    })?;

    let bin_size = std::fs::metadata(&output)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    // A successful exit code is not proof that there is a document to show.
    if bin_size == 0 {
        let error = format!("{} could not be opened: it has no content", file_name);
        log_event(
            &state,
            &format!("[OPEN] failed file={} error=empty Editor.bin", file_name),
        );
        return Err(error);
    }
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

    *state.current_file.lock().unwrap() = Some(input.clone());
    *state.modified.lock().unwrap() = false;
    // None for anything that is not CSV or TXT: the options of the previous
    // document must not leak into how this one is saved.
    *state.text_options.lock().unwrap() = text_options;

    if opened_blank {
        // Nothing is on disk yet, so the entry waits for the first real save.
        arm_pending_recent(&state.pending_recent, &input);
    } else if record_recent {
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
    let text_options = text_options_for_save(&state, &dest);

    super::converter::convert_file(
        &app,
        &input.to_string_lossy(),
        &dest.to_string_lossy(),
        format_from,
        format_to,
        &state.temp_dir.to_string_lossy(),
        &crate::temp_layout::fontdata_dir(&state.temp_root),
        text_options,
    )
    .await?;
    finish_text_save(&state, &dest, text_options);

    // The 0-byte file this document was opened from now holds a real document,
    // so it earns its place in the recent list (#33).
    if let Some(recorded) = take_pending_recent(&state.pending_recent, &dest) {
        super::recent::record(&app, &recorded.to_string_lossy());
    }

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
    let text_options = text_options_for_save(&state, &dest);

    super::converter::convert_file(
        &app,
        &input.to_string_lossy(),
        &dest.to_string_lossy(),
        format_from,
        format_to,
        &state.temp_dir.to_string_lossy(),
        &crate::temp_layout::fontdata_dir(&state.temp_root),
        text_options,
    )
    .await?;
    finish_text_save(&state, &dest, text_options);

    // A PDF export leaves the document itself untouched (current_file keeps
    // pointing at the editable file), so it does not belong in the list either.
    if format_to != 513 {
        // Same reason a PDF export does not end the pending 0-byte entry, while
        // a real Save As does: record below files the path actually written,
        // which when the entry is consumed is that same path (#33).
        end_pending_recent(&state.pending_recent, &dest);
        *state.current_file.lock().unwrap() = Some(dest);
        *state.modified.lock().unwrap() = false;
        super::recent::record(&app, &path);
    }
    Ok("ok".to_string())
}

// Without this, x2t writes every CSV as UTF-8 with commas, so saving over a
// semicolon or windows-1252 file would rewrite it in another dialect. A Save As
// to CSV or TXT reuses the options too: it is the same data, and whatever reads
// the original most likely expects the copy in the same shape. A document that
// did not come from CSV or TXT keeps x2t's defaults.
fn text_options_for_save(state: &AppState, dest: &Path) -> Option<crate::text_import::TextOptions> {
    let kind = crate::text_import::TextKind::for_path(dest)?;
    state
        .text_options
        .lock()
        .unwrap()
        .map(|options| options.for_kind(kind))
}

// x2t does not write CSV and TXT back the way they were read (BOM, line
// endings, final breaks, and for TXT the encoding), so the saved file is put
// back into the original's shape. It is already saved by now, so a failure
// here is only logged.
fn finish_text_save(
    state: &AppState,
    dest: &Path,
    text_options: Option<crate::text_import::TextOptions>,
) {
    let (Some(options), Some(kind)) = (text_options, crate::text_import::TextKind::for_path(dest))
    else {
        return;
    };
    match crate::text_import::finish_saved_text(dest, kind, options) {
        Ok(0) => {}
        Ok(unmappable) => log_event(
            state,
            &format!(
                "[SAVE] {} characters windows-1252 cannot hold were written as '?'",
                unmappable
            ),
        ),
        Err(e) => log_event(
            state,
            &format!("[SAVE] could not restore the original text format: {}", e),
        ),
    }
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
    // A failed print leaves nothing for the viewer to open.
    *state.last_print_pdf.lock().unwrap() = None;

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

    let pdf_path = state.temp_dir.join(PRINT_PDF_NAME);
    if pdf_path.exists() {
        let _ = std::fs::remove_file(&pdf_path);
    }
    super::converter::convert_file(
        &app,
        &editor_bin.to_string_lossy(),
        &pdf_path.to_string_lossy(),
        8192,
        513,
        &state.temp_dir.to_string_lossy(),
        &crate::temp_layout::fontdata_dir(&state.temp_root),
        None,
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
    *state.last_print_pdf.lock().unwrap() = Some(pdf_path.clone());
    Ok(pdf_path.to_string_lossy().to_string())
}

const PRINT_PDF_NAME: &str = "print_output.pdf";

// The only file the viewer and the printer commands hand on: the PDF the last
// print wrote, still a regular file (not a link) directly inside this
// instance's folder.
pub(crate) fn last_printed_pdf(
    last_print: Option<&Path>,
    temp_dir: &Path,
) -> Result<PathBuf, String> {
    let pdf = last_print.ok_or("No printed PDF to open")?;
    if pdf.file_name() != Some(std::ffi::OsStr::new(PRINT_PDF_NAME)) {
        return Err("Not the printed PDF".to_string());
    }
    let is_file = std::fs::symlink_metadata(pdf)
        .map(|meta| meta.file_type().is_file())
        .unwrap_or(false);
    if !is_file {
        return Err("The printed PDF is gone".to_string());
    }
    let dir = temp_dir.canonicalize().map_err(|e| e.to_string())?;
    let parent = pdf
        .canonicalize()
        .map_err(|e| e.to_string())?
        .parent()
        .map(Path::to_path_buf);
    if parent.as_deref() != Some(dir.as_path()) {
        return Err("The printed PDF is outside the instance folder".to_string());
    }
    // The path as print_document built it: the canonical form on Windows is a
    // \\?\ path, which not every program takes.
    Ok(pdf.to_path_buf())
}

#[tauri::command]
pub fn open_pdf_viewer(state: State<'_, AppState>) -> Result<String, String> {
    let last_print = state.last_print_pdf.lock().unwrap().clone();
    let path = last_printed_pdf(last_print.as_deref(), &state.temp_dir).map_err(|e| {
        log_event(&state, &format!("[PRINT] viewer refused: {}", e));
        e
    })?;

    // explorer opens the file with its default program and, unlike cmd, does
    // not read & ^ % in the path as its own syntax.
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer.exe")
        .arg(&path)
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

// Printer names in the list the printer plugin reads from Windows: Get-Printer
// as JSON, one object for a single printer and an array for several.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn printer_names(listed: &str) -> Vec<String> {
    let entries = match serde_json::from_str::<serde_json::Value>(listed.trim()) {
        Ok(serde_json::Value::Array(items)) => items,
        Ok(single) => vec![single],
        Err(_) => Vec::new(),
    };
    entries
        .iter()
        .filter_map(|entry| entry.get("Name").or_else(|| entry.get("name")))
        .filter_map(|name| name.as_str())
        .map(str::to_string)
        .collect()
}

// A printer is only printed to when Windows lists it under exactly that name.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn listed_printer(requested: &str, names: &[String]) -> Result<String, String> {
    names
        .iter()
        .find(|name| name.as_str() == requested)
        .cloned()
        .ok_or_else(|| "Not a listed printer".to_string())
}

// Prints the PDF the last print wrote on a printer Windows lists.
#[tauri::command]
pub async fn print_to_printer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    printer: String,
) -> Result<String, String> {
    let last_print = state.last_print_pdf.lock().unwrap().clone();
    let result = match last_printed_pdf(last_print.as_deref(), &state.temp_dir) {
        Ok(pdf) => send_to_printer(app, &printer, &pdf).await,
        Err(e) => Err(e),
    };
    match &result {
        Ok(()) => log_event(&state, "[PRINT] sent to printer"),
        Err(e) => log_event(&state, &format!("[PRINT] printer refused: {}", e)),
    }
    // As the plugin call this replaces did, the PDF goes once the printer has
    // it; a refused request leaves it in place.
    if let (Ok(()), Some(pdf)) = (&result, last_print) {
        let _ = std::fs::remove_file(pdf);
        *state.last_print_pdf.lock().unwrap() = None;
    }
    result.map(|()| "ok".to_string())
}

// Runs the SumatraPDF copy the printer plugin unpacks into the system temp
// folder at startup. No shell sits between the arguments and the program.
#[cfg(target_os = "windows")]
async fn send_to_printer(app: tauri::AppHandle, printer: &str, pdf: &Path) -> Result<(), String> {
    use tauri_plugin_printer_v2::PrinterExt;

    let listed = tauri::async_runtime::spawn_blocking(move || app.printer().get_printers())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let names = printer_names(&listed);
    let printer = listed_printer(printer, &names)?;

    let sumatra = std::env::temp_dir().join("sm.exe");
    let target = pdf.to_path_buf();
    let status = tauri::async_runtime::spawn_blocking(move || {
        std::process::Command::new(&sumatra)
            .arg("-print-to")
            .arg(&printer)
            .args(["-print-settings", "{}", "-silent"])
            .arg(&target)
            .status()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the print program ended with {}", status))
    }
}

#[cfg(not(target_os = "windows"))]
async fn send_to_printer(_app: tauri::AppHandle, _printer: &str, _pdf: &Path) -> Result<(), String> {
    Err("Printing to a named printer is only available on Windows".to_string())
}

// The insert image dialog. The choice is remembered so copy-to-media can tell
// a path the user picked from any other.
#[tauri::command]
pub async fn pick_image_files(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    multiple: bool,
) -> Result<Option<Vec<String>>, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = window
        .dialog()
        .file()
        .set_parent(&window)
        .add_filter("Images", &crate::protocol_limits::IMAGE_EXTENSIONS);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        if multiple {
            dialog.blocking_pick_files()
        } else {
            dialog.blocking_pick_file().map(|choice| vec![choice])
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let paths: Vec<PathBuf> = picked
        .into_iter()
        .filter_map(|choice| choice.into_path().ok())
        .collect();
    state.picked_images.lock().unwrap().extend(paths.iter().cloned());
    Ok(Some(
        paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
    ))
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

// Hands over the launch argument once: a page reload must not open it again.
#[tauri::command]
pub fn take_pending_open_file(state: State<'_, AppState>) -> Option<String> {
    state.pending_open.lock().unwrap().take()
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
        &crate::temp_layout::fontdata_dir(&state.temp_root),
        None,
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
        // Under the open document's address, or sdkjs would take it for an
        // image of another document.
        let doc = open_document(&state);
        let img_url = format!(
            "ascdesktop://docmedia/{}/media/{}",
            crate::temp_layout::doc_key(doc.id, doc.seq),
            staged
        );
        images.insert(name, serde_json::Value::String(img_url));
    }

    Ok(serde_json::json!({
        "data": b64,
        "images": images
    }))
}

#[tauri::command]
pub fn get_system_fonts(state: State<'_, AppState>) -> Result<String, String> {
    let path = crate::temp_layout::fontdata_dir(&state.temp_root).join("AllFonts.js");
    std::fs::read_to_string(&path).map_err(|e| format!("Cannot read AllFonts.js: {}", e))
}

// Case-insensitive on Windows only, even though the default macOS filesystem is
// case-insensitive too. Both paths compared here come from the same source (the
// path the document was opened from, and the destination the save writes), so
// they differ in case only if the user retyped one by hand in the Save As
// dialog. On Windows that is one drive-letter case away and worth covering; on
// macOS it would mean also honouring the per-volume setting, which is a real
// filesystem query for a case that costs a missing recent-list entry.
fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(target_os = "windows"))]
    {
        a == b
    }
}

fn arm_pending_recent(pending: &Mutex<Option<PathBuf>>, path: &Path) {
    *pending.lock().unwrap() = Some(path.to_path_buf());
}

// Cleared on every open: the pending path belongs to the document that is on
// screen, and opening another one ends that document. Without this, saving the
// next document would file the abandoned 0-byte path in the recent list.
fn clear_pending_recent(pending: &Mutex<Option<PathBuf>>) {
    *pending.lock().unwrap() = None;
}

// Consumed only by a save that writes the pending path itself. A Save As to a
// different path is a different file, and the 0-byte one was never filled.
fn take_pending_recent(pending: &Mutex<Option<PathBuf>>, dest: &Path) -> Option<PathBuf> {
    let mut slot = pending.lock().unwrap();
    match slot.as_ref() {
        Some(path) if same_path(path, dest) => slot.take(),
        _ => None,
    }
}

// How a Save As ends the pending entry, either way: saving onto the 0-byte file
// itself materialises it and consumes the entry, saving anywhere else leaves it
// empty and drops it. Returns what was consumed so the caller can tell the two
// apart; save_file_as needs no more than that its record call, which files the
// path actually written, is the only one that runs.
fn end_pending_recent(pending: &Mutex<Option<PathBuf>>, dest: &Path) -> Option<PathBuf> {
    let consumed = take_pending_recent(pending, dest);
    if consumed.is_none() {
        clear_pending_recent(pending);
    }
    consumed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    // A throwaway temp dir laid out like the app's: media/ with one inserted
    // image, plus the changes/ an older version left behind.
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

    // Switching documents does invalidate the images, so the full reset still
    // has to take media/ with it. Nothing else may: x2t resolves the images
    // from media/ on every export, and an export that finds it emptied draws
    // each one as a solid black rectangle (#31).
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

    // The #33 regression: x2t leaves docx_unpacked/ in the temp dir, and given a
    // 0-byte file it converts that leftover instead of failing, exits 0 and hands
    // back the previous document. Opening another document has to take it.
    #[test]
    fn clear_document_temp_drops_x2t_unpacked_dirs() {
        let dir = temp_dir_with_changes_and_media();
        for scratch in ["docx_unpacked", "xlsx_unpacked", "pptx_unpacked", "asc9gz1zK"] {
            std::fs::create_dir_all(dir.join(scratch)).unwrap();
            std::fs::write(dir.join(scratch).join("document.xml"), b"<w:document/>").unwrap();
        }

        clear_document_temp(&dir);

        for scratch in ["docx_unpacked", "xlsx_unpacked", "pptx_unpacked", "asc9gz1zK"] {
            assert!(
                !dir.join(scratch).exists(),
                "{} must not survive into the next document: x2t converts it and \
                 reports success when the file it was given is empty (#33)",
                scratch
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // The scratch sweep runs over the whole temp dir, so it has to leave what the
    // app itself keeps there. fontdata/ and x2t-workdir/ in particular are
    // rebuilt per export, not per document.
    #[test]
    fn clear_document_temp_keeps_the_apps_own_directories() {
        let dir = temp_dir_with_changes_and_media();
        for kept in ["fontdata", "x2t-workdir"] {
            std::fs::create_dir_all(dir.join(kept)).unwrap();
            std::fs::write(dir.join(kept).join("AllFonts.js"), b"//").unwrap();
        }
        std::fs::write(dir.join("js-debug.log"), b"log").unwrap();

        clear_document_temp(&dir);

        for kept in ["fontdata", "x2t-workdir"] {
            assert!(
                dir.join(kept).join("AllFonts.js").exists(),
                "{} belongs to the app, not to the open document",
                kept
            );
        }
        assert!(dir.join("js-debug.log").exists(), "the log is not scratch");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Runs on a temp dir that a fresh profile has not created yet.
    #[test]
    fn clearing_is_a_noop_when_nothing_exists() {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-file-ops-test-empty-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();

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

    // Leak 1: the pending path must not outlive its document. Opening another
    // file ends the 0-byte one, and saving THAT document would otherwise file
    // the abandoned empty path in the recent list.
    #[test]
    fn opening_another_document_drops_the_pending_recent_entry() {
        let pending = Mutex::new(None);
        let empty = PathBuf::from("/home/user/nuevo.docx");

        arm_pending_recent(&pending, &empty);
        clear_pending_recent(&pending); // what every open does first

        let other = PathBuf::from("/home/user/informe.docx");
        assert_eq!(
            take_pending_recent(&pending, &other),
            None,
            "saving the next document must not record the abandoned empty path"
        );
        assert_eq!(
            take_pending_recent(&pending, &empty),
            None,
            "the entry is gone for its own path too: that document was closed"
        );
    }

    // Leak 2: Save As writes somewhere else, so the 0-byte file stays empty and
    // stops being this document's path. Its entry is dropped, not consumed, and
    // a later plain save must not resurrect it.
    #[test]
    fn save_as_elsewhere_drops_the_pending_recent_entry() {
        let pending = Mutex::new(None);
        let empty = PathBuf::from("/home/user/nuevo.docx");
        arm_pending_recent(&pending, &empty);

        let elsewhere = PathBuf::from("/home/user/copia.docx");
        assert_eq!(
            end_pending_recent(&pending, &elsewhere),
            None,
            "a different destination is a different file"
        );

        assert_eq!(
            take_pending_recent(&pending, &empty),
            None,
            "the empty file was never written, so it never enters the list"
        );
    }

    // The sub-case of leak 2: Save As can also pick the 0-byte file itself, and
    // then the file IS materialised. Its entry is consumed, not dropped, and the
    // record call that follows in save_file_as files that same path.
    #[test]
    fn save_as_onto_the_empty_file_consumes_the_pending_entry() {
        let pending = Mutex::new(None);
        let empty = PathBuf::from("/home/user/nuevo.docx");
        arm_pending_recent(&pending, &empty);

        assert_eq!(
            end_pending_recent(&pending, &empty),
            Some(empty.clone()),
            "saving onto the empty file writes it, so the entry is earned \
             rather than dropped with the rest of the Save As branch"
        );
        assert_eq!(
            end_pending_recent(&pending, &empty),
            None,
            "and it is spent: later saves must not record it again"
        );
    }

    // The path the whole fix exists for, and it fires exactly once.
    #[test]
    fn saving_the_empty_file_itself_records_it_once() {
        let pending = Mutex::new(None);
        let empty = PathBuf::from("/home/user/nuevo.odt");
        arm_pending_recent(&pending, &empty);

        assert_eq!(take_pending_recent(&pending, &empty), Some(empty.clone()));
        assert_eq!(
            take_pending_recent(&pending, &empty),
            None,
            "every later save would otherwise re-record the same file"
        );
    }

    fn print_dir() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-print-test-{}-{}",
            std::process::id(),
            unique
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn printed_pdf_is_handed_on() {
        let dir = print_dir();
        let pdf = dir.join(PRINT_PDF_NAME);
        std::fs::write(&pdf, b"%PDF-1.7").unwrap();
        assert_eq!(last_printed_pdf(Some(&pdf), &dir).unwrap(), pdf);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn printed_pdf_refused_without_a_print() {
        let dir = print_dir();
        // A leftover from an earlier run is not this print's output.
        std::fs::write(dir.join(PRINT_PDF_NAME), b"%PDF-1.7").unwrap();
        assert!(last_printed_pdf(None, &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn printed_pdf_refuses_any_other_file() {
        let dir = print_dir();
        let exe = dir.join("tool.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        assert!(last_printed_pdf(Some(&exe), &dir).is_err());

        let other = print_dir();
        let outside = other.join(PRINT_PDF_NAME);
        std::fs::write(&outside, b"%PDF-1.7").unwrap();
        assert!(last_printed_pdf(Some(&outside), &dir).is_err());

        let sub = dir.join("downloads");
        std::fs::create_dir_all(&sub).unwrap();
        let nested = sub.join(PRINT_PDF_NAME);
        std::fs::write(&nested, b"%PDF-1.7").unwrap();
        assert!(last_printed_pdf(Some(&nested), &dir).is_err());

        let dotted = dir.join("downloads").join("..").join(PRINT_PDF_NAME);
        std::fs::write(dir.join(PRINT_PDF_NAME), b"%PDF-1.7").unwrap();
        assert!(
            last_printed_pdf(Some(&dotted), &dir).is_ok(),
            "the same file reached through .. is still the printed PDF"
        );

        // Gone since the print: nothing to open.
        let _ = std::fs::remove_file(dir.join(PRINT_PDF_NAME));
        assert!(last_printed_pdf(Some(&dir.join(PRINT_PDF_NAME)), &dir).is_err());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[cfg(unix)]
    #[test]
    fn printed_pdf_refuses_a_link_in_its_place() {
        let dir = print_dir();
        let target = dir.join("tool.sh");
        std::fs::write(&target, b"#!/bin/sh").unwrap();
        let pdf = dir.join(PRINT_PDF_NAME);
        std::os::unix::fs::symlink(&target, &pdf).unwrap();
        assert!(last_printed_pdf(Some(&pdf), &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // What Get-Printer | ConvertTo-Json prints, trimmed to the fields read here.
    const TWO_PRINTERS: &str = r#"[
        {"Name": "Microsoft Print to PDF", "DriverName": "Microsoft Print To PDF"},
        {"Name": "Office \"Laser\"", "DriverName": "Generic"}
    ]"#;

    #[test]
    fn printer_list_reads_one_or_several_printers() {
        assert_eq!(
            printer_names(TWO_PRINTERS),
            vec!["Microsoft Print to PDF".to_string(), "Office \"Laser\"".to_string()]
        );
        assert_eq!(
            printer_names("{\"Name\": \"Only one\"}\r\n"),
            vec!["Only one".to_string()]
        );
        assert!(printer_names("").is_empty());
        assert!(printer_names("Unsupported OS").is_empty());
    }

    #[test]
    fn printing_takes_a_listed_printer() {
        let names = printer_names(TWO_PRINTERS);
        assert_eq!(
            listed_printer("Microsoft Print to PDF", &names).unwrap(),
            "Microsoft Print to PDF"
        );
        assert_eq!(listed_printer("Office \"Laser\"", &names).unwrap(), "Office \"Laser\"");
    }

    #[test]
    fn printing_refuses_any_other_printer_name() {
        let names = printer_names(TWO_PRINTERS);
        for requested in [
            "Missing printer",
            "",
            "default",
            "microsoft print to pdf",
            "Microsoft Print to PDF ",
            "Microsoft Print to PDF\"; calc; \"",
            "Microsoft Print to PDF; calc",
            "$(calc)",
            "Microsoft Print to PDF$(calc)",
            "x\" -silent \"C:\\Windows\\win.ini",
        ] {
            assert!(listed_printer(requested, &names).is_err(), "{:?}", requested);
        }
        assert!(listed_printer("Microsoft Print to PDF", &[]).is_err());
    }
}
