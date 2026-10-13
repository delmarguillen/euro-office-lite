// Crash recovery: everything the app writes so that a process that dies without
// closing can be offered back to the user on the next start.
//
// Modelled on the upstream desktop shell (desktop-sdk,
// ChromiumBasedEditors/lib/src/cefview.cpp): one folder per session under
// <app_config_dir>/recover/, holding the Editor.bin the document was OPENED
// with, a single append-only changes0.json, a doc.info metadata file and a
// rec.lock the process keeps open while it lives. A folder whose lock is free
// is a folder whose process died: that is the crash detection, and it survives
// a kill -9 because the OS releases the lock on its own.
//
// The changes file is byte-compatible with the upstream's: every change is
// written as "<payload>", so the whole file is a JSON array once wrapped in
// brackets. deleteIndex is honoured as a truncation, which is what sdkjs means
// by it (common/collaborativeHistory.js truncates Changes to it), and what
// keeps undo and save from leaving ghost changes behind.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

use crate::file_ops::{log_event, AppState};
use crate::temp_layout::{is_locked, take_lock};

// Around 84 bytes per typed character, so half an hour of continuous typing is
// well under a megabyte: this is a stop against a runaway loop filling the
// user's disk, not a working limit.
pub const MAX_CHANGES_BYTES: u64 = 64 * 1024 * 1024;

// A candidate older than this is not offered: a crash the user forgot about is
// work they have already redone, and recovering it would be the surprise.
pub const MAX_AGE_MS: u64 = 14 * 24 * 60 * 60 * 1000;

const LOCK_FILE: &str = "rec.lock";
const INFO_FILE: &str = "doc.info";
const BIN_FILE: &str = "Editor.bin";

// ---------------------------------------------------------------------------
// On-disk shapes
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DocInfo {
    // None for a document that has never been written to disk.
    #[serde(default)]
    pub path: Option<String>,
    // x2t format id of `path`, None alongside it.
    #[serde(default)]
    pub format: Option<i32>,
    pub name: String,
    pub doc_type: String,
    #[serde(default)]
    pub app_version: String,
    // Number of changes that were already inside the saved file at the last
    // successful save. When it equals the number of changes on disk there is
    // nothing to recover and the folder is dropped on sight.
    #[serde(default)]
    pub saved_count: usize,
    #[serde(default)]
    pub created_ms: u64,
    // The instance whose temp folder holds the document's images (the
    // <millis>-<pid> of temp_layout). None in folders written before that.
    #[serde(default)]
    pub instance: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub doc_type: String,
    pub changes: usize,
    pub modified_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedSession {
    pub id: String,
    pub data: String,
    pub changes: Vec<String>,
    pub name: String,
    pub path: Option<String>,
    pub doc_type: String,
}

// ---------------------------------------------------------------------------
// Live session
// ---------------------------------------------------------------------------

pub struct RecoverySession {
    pub dir: PathBuf,
    // Held open for the whole life of the session. Dropping it is what makes
    // the folder look recoverable to the next process, so it is only dropped
    // when the session ends.
    pub lock: std::fs::File,
    // Changes already in the file when this session adopted it: 0 on a normal
    // open, n after a recovery. sdkjs numbers deleteIndex from the start of ITS
    // session, so every index it sends is read relative to this.
    pub base: usize,
    // Byte offset where each change record starts. Truncating to keep k
    // changes is a set_len to entries[k].
    pub entries: Vec<u64>,
    pub saved_count: usize,
    // The size cap is reported once, not on every batch.
    pub capped: bool,
}

pub fn changes_path(dir: &Path) -> PathBuf {
    dir.join("changes").join("changes0.json")
}

// ---------------------------------------------------------------------------
// Pure primitives (the part under test)
// ---------------------------------------------------------------------------

// A change is either plain base64 or the "<length>;<base64>" form sdkjs also
// produces (bridge.js builds fonts the same way). Anything outside that
// alphabet is a partially written tail, not a change.
pub fn is_valid_change(value: &str) -> bool {
    let payload = match value.split_once(';') {
        Some((prefix, rest)) if !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit()) => {
            rest
        }
        _ => value,
    };
    !payload.is_empty()
        && payload
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).rev().find(|&i| &haystack[i..i + needle.len()] == needle)
}

// Reads a changes file that may have been cut anywhere: the process can die
// between the opening quote and the trailing comma of the record it is writing.
// A lost change is acceptable, a broken editor is not, so this never fails: it
// walks back to the last record that closes cleanly and drops the tail from the
// first element that is not a change.
pub fn parse_changes_tolerant(raw: &[u8]) -> Vec<String> {
    // Records are written as "<payload>", so a complete one always ends with a
    // quote followed by a comma.
    let mut end = match find_last(raw, b"\",") {
        Some(index) => index + 1,
        None => return Vec::new(),
    };

    for _ in 0..4 {
        let mut json = Vec::with_capacity(end + 2);
        json.push(b'[');
        json.extend_from_slice(&raw[..end]);
        json.push(b']');

        if let Ok(values) = serde_json::from_slice::<Vec<String>>(&json) {
            let keep = values
                .iter()
                .position(|value| !is_valid_change(value))
                .unwrap_or(values.len());
            let mut values = values;
            values.truncate(keep);
            return values;
        }

        // Drop the last element and try again: the quote that closes the
        // previous record is the next candidate end.
        end = match find_last(&raw[..end.saturating_sub(1)], b"\",") {
            Some(index) => index + 1,
            None => return Vec::new(),
        };
    }

    Vec::new()
}

// Writes the file from scratch and returns the offsets of every record. Used
// when a session adopts a recovered folder: rewriting canonically is what makes
// the in-memory offsets match the bytes on disk after a partial tail was
// dropped.
pub fn write_changes_file(path: &Path, changes: &[String]) -> std::io::Result<Vec<u64>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    let mut entries = Vec::with_capacity(changes.len());
    let mut offset = 0u64;
    for change in changes {
        entries.push(offset);
        let record = format!("\"{}\",", change);
        file.write_all(record.as_bytes())?;
        offset += record.len() as u64;
    }
    file.sync_all()?;
    Ok(entries)
}

// deleteIndex means "keep the first `base + delete_index` changes and throw the
// rest away". It counts from the start of the EDITOR's session, so a recovered
// session that starts with `base` changes already on disk has to add them back.
pub fn truncate_changes(
    path: &Path,
    entries: &mut Vec<u64>,
    base: usize,
    delete_index: i64,
) -> std::io::Result<bool> {
    // The index is clamped before the base is added, not after: an index below
    // zero is nonsense from the editor, and it must fall back to "keep nothing
    // of this session", never to "drop the changes that were recovered".
    let keep = base + delete_index.max(0) as usize;
    if keep >= entries.len() {
        return Ok(false);
    }
    let offset = entries[keep];
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.set_len(offset)?;
    file.sync_all()?;
    entries.truncate(keep);
    Ok(true)
}

// Appends a batch in the upstream's format. Returns false when the size cap
// stopped the write, so the caller can log it once.
pub fn append_changes(
    path: &Path,
    entries: &mut Vec<u64>,
    changes: &[String],
) -> std::io::Result<bool> {
    if changes.is_empty() {
        return Ok(true);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut offset = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if offset >= MAX_CHANGES_BYTES {
        return Ok(false);
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    for change in changes {
        entries.push(offset);
        let record = format!("\"{}\",", change);
        file.write_all(record.as_bytes())?;
        offset += record.len() as u64;
    }
    file.sync_all()?;
    Ok(true)
}

// The only ids that exist are the ones recovery_begin makes: <unix_millis>-<pid>.
// Anything else reaching a command came from somewhere other than our own
// listing, and it is a path fragment about to be joined onto the recovery root,
// so it is refused rather than sanitised. Path::starts_with would not catch it:
// it compares components, and `..` is a component like any other.
pub fn is_valid_session_id(id: &str) -> bool {
    match id.split_once('-') {
        Some((millis, pid)) => {
            !millis.is_empty()
                && !pid.is_empty()
                && millis.bytes().all(|b| b.is_ascii_digit())
                && pid.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

// The first four bytes of Editor.bin name the editor that wrote it, which is
// also how the upstream shell reads the type. Fallback for an unreadable
// doc.info.
pub fn doc_type_from_magic(bin: &[u8]) -> Option<&'static str> {
    match bin.get(..4)? {
        b"DOCY" => Some("word"),
        b"XLSY" => Some("cell"),
        b"PPTY" => Some("slide"),
        _ => None,
    }
}

fn file_mtime_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

// Walks the recovery root and decides, folder by folder, between "someone else
// is using this", "there is nothing here worth offering" (deleted on the spot,
// so the root does not grow forever) and "offer it".
//
// `is_locked` is a parameter rather than a call so the policy can be tested
// without taking real OS locks.
pub fn scan_candidates(
    root: &Path,
    now_ms: u64,
    is_locked: impl Fn(&Path) -> bool,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return candidates;
    };

    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        // A live process owns this folder: not ours to read, not ours to
        // delete. An orphan rec.lock with no process behind it opens fine, so
        // this is a statement about processes, not about the file existing.
        if is_locked(&dir.join(LOCK_FILE)) {
            continue;
        }

        let changes_file = changes_path(&dir);
        let raw = std::fs::read(&changes_file).unwrap_or_default();
        let changes = parse_changes_tolerant(&raw);
        if changes.is_empty() {
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }

        let bin = std::fs::read(dir.join(BIN_FILE)).unwrap_or_default();
        let Some(magic_type) = doc_type_from_magic(&bin) else {
            // Under four bytes or an unknown magic: there is no document to
            // replay the changes onto.
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        };

        let info = std::fs::read_to_string(dir.join(INFO_FILE))
            .ok()
            .and_then(|raw| serde_json::from_str::<DocInfo>(&raw).ok());

        // Everything the document was saved with is already inside the user's
        // file: the folder is a clean-close residue, not a crash.
        if let Some(info) = &info {
            if info.saved_count == changes.len() {
                let _ = std::fs::remove_dir_all(&dir);
                continue;
            }
        }

        let created_ms = info
            .as_ref()
            .map(|info| info.created_ms)
            .filter(|value| *value > 0)
            .unwrap_or_else(|| file_mtime_ms(&changes_file));
        if now_ms.saturating_sub(created_ms) > MAX_AGE_MS {
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }

        let id = dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        candidates.push(Candidate {
            name: info
                .as_ref()
                .map(|info| info.name.clone())
                .unwrap_or_else(|| id.clone()),
            path: info.as_ref().and_then(|info| info.path.clone()),
            doc_type: info
                .as_ref()
                .map(|info| info.doc_type.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| magic_type.to_string()),
            changes: changes.len(),
            modified_ms: file_mtime_ms(&changes_file),
            id,
        });
    }

    candidates
}

// The instances whose temp folders hold the images of a document that is
// still in a recovery folder, so the sweep of dead instances leaves them.
pub fn referenced_instances(root: &Path) -> std::collections::HashSet<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return std::collections::HashSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| read_doc_info(&entry.path()))
        .filter_map(|info| info.instance)
        .collect()
}

// ---------------------------------------------------------------------------
// Session plumbing
// ---------------------------------------------------------------------------

pub fn recover_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("recover");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn write_doc_info(dir: &Path, info: &DocInfo) -> Result<(), String> {
    let json = serde_json::to_string_pretty(info).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(INFO_FILE), json).map_err(|e| e.to_string())
}

fn read_doc_info(dir: &Path) -> Option<DocInfo> {
    std::fs::read_to_string(dir.join(INFO_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str::<DocInfo>(&raw).ok())
}

// Ends whatever session is live. The lock is dropped before the folder goes:
// on Windows the file cannot be removed while this process still holds it open.
pub(crate) fn end_session(state: &AppState, discard: bool) -> Option<PathBuf> {
    let session = state.recovery.lock().unwrap().take()?;
    let dir = session.dir.clone();
    drop(session);
    if discard {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Some(dir)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn recovery_begin(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    path: Option<String>,
    format: Option<i32>,
    doc_type: String,
    data: String,
) -> Result<String, String> {
    // Changing documents ends the previous session: either it had no changes
    // or the user was asked and chose to discard.
    end_session(&state, true);

    let root = recover_root(&app)?;
    let id = format!("{}-{}", now_ms(), std::process::id());
    let dir = root.join(&id);
    std::fs::create_dir_all(dir.join("changes")).map_err(|e| e.to_string())?;

    let lock = take_lock(&dir.join(LOCK_FILE)).map_err(|e| e.to_string())?;

    let bin = STANDARD.decode(&data).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(BIN_FILE), &bin).map_err(|e| e.to_string())?;
    std::fs::write(changes_path(&dir), b"").map_err(|e| e.to_string())?;

    write_doc_info(
        &dir,
        &DocInfo {
            path,
            format,
            name: name.clone(),
            doc_type,
            app_version: app.package_info().version.to_string(),
            saved_count: 0,
            created_ms: now_ms(),
            instance: Some(state.instance.id.clone()),
        },
    )?;

    log_event(
        &state,
        &format!("[RECOVER] begin session={} bytes={}", id, bin.len()),
    );

    *state.recovery.lock().unwrap() = Some(RecoverySession {
        dir,
        lock,
        base: 0,
        entries: Vec::new(),
        saved_count: 0,
        capped: false,
    });

    Ok(id)
}

// One batch of editor changes. The bridge splits the string sdkjs joins with
// `","`, so that branch and the raw-array one arrive here in the same shape.
#[tauri::command]
pub fn save_changes(
    state: State<'_, AppState>,
    changes: Vec<String>,
    delete_index: Option<i64>,
    count: i64,
) -> Result<String, String> {
    let _ = count;
    // Everything that touches the file happens under the lock; the log lines it
    // produced are emitted after, because logging needs the state too.
    let (result, logs) = {
        let mut guard = state.recovery.lock().unwrap();
        let Some(session) = guard.as_mut() else {
            // No session: nothing is being recovered, and dropping the batch is
            // better than writing it where nobody will look for it.
            return Ok("no-session".to_string());
        };

        let path = changes_path(&session.dir);
        let mut logs: Vec<String> = Vec::new();

        // A save and an undo both arrive as an empty batch carrying only the
        // index: the truncation is the whole point of them.
        if let Some(delete_index) = delete_index {
            let base = session.base;
            let truncated = truncate_changes(&path, &mut session.entries, base, delete_index)
                .map_err(|e| e.to_string())?;
            if truncated {
                logs.push(format!(
                    "[RECOVER] truncate deleteIndex={} kept={}",
                    delete_index,
                    session.entries.len()
                ));
            }
        }

        let outcome = if changes.is_empty() {
            "ok"
        } else if append_changes(&path, &mut session.entries, &changes)
            .map_err(|e| e.to_string())?
        {
            "ok"
        } else {
            if !session.capped {
                session.capped = true;
                logs.push(
                    "[RECOVER] size cap reached: changes are no longer recorded for this session"
                        .to_string(),
                );
            }
            "capped"
        };

        (outcome.to_string(), logs)
    };

    for line in logs {
        log_event(&state, &line);
    }
    Ok(result)
}

// After a save that wrote the user's file, everything on disk is inside it. The
// count is recorded rather than the file emptied: sdkjs keeps numbering
// deleteIndex from the start of its session, not from the last save, so
// emptying would leave the next index pointing past the end.
#[tauri::command]
pub fn recovery_mark_saved(
    state: State<'_, AppState>,
    path: Option<String>,
    format: Option<i32>,
    name: Option<String>,
) -> Result<String, String> {
    let mut guard = state.recovery.lock().unwrap();
    let Some(session) = guard.as_mut() else {
        return Ok("no-session".to_string());
    };

    let saved_count = session.entries.len();
    session.saved_count = saved_count;
    let dir = session.dir.clone();
    drop(guard);

    if let Some(mut info) = read_doc_info(&dir) {
        info.saved_count = saved_count;
        // Save As moves the document: the folder has to point at where it
        // really lives now, or a recovery would offer to write the old path.
        if path.is_some() {
            info.path = path;
            info.format = format;
        }
        if let Some(name) = name {
            info.name = name;
        }
        write_doc_info(&dir, &info)?;
    }

    log_event(&state, &format!("[RECOVER] saved changes={}", saved_count));
    Ok("ok".to_string())
}

#[tauri::command]
pub fn recovery_end(state: State<'_, AppState>, discard: bool) -> Result<String, String> {
    match end_session(&state, discard) {
        Some(_) => {
            log_event(&state, &format!("[RECOVER] end discard={}", discard));
            Ok("ok".to_string())
        }
        None => Ok("no-session".to_string()),
    }
}

#[tauri::command]
pub fn recovery_candidates(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<Candidate>, String> {
    let root = recover_root(&app)?;
    let candidates = scan_candidates(&root, now_ms(), |path| is_locked(path));
    log_event(
        &state,
        &format!("[RECOVER] candidates={}", candidates.len()),
    );
    Ok(candidates)
}

// Hands the frontend everything the replay needs and adopts the folder as this
// process's session, so editing continues writing into the same file with
// `base` set to the changes that were replayed.
#[tauri::command]
pub fn recovery_load(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<LoadedSession, String> {
    if !is_valid_session_id(&id) {
        return Err(format!("Not a recovery session id: {}", id));
    }
    let root = recover_root(&app)?;
    let dir = root.join(&id);
    if !dir.is_dir() {
        return Err(format!("No recovery session named {}", id));
    }

    end_session(&state, true);

    let lock = take_lock(&dir.join(LOCK_FILE)).map_err(|e| e.to_string())?;

    let bin = std::fs::read(dir.join(BIN_FILE)).map_err(|e| e.to_string())?;
    let raw = std::fs::read(changes_path(&dir)).unwrap_or_default();
    let changes = parse_changes_tolerant(&raw);

    // Rewritten from what was actually accepted: a partially written tail must
    // not stay on disk under offsets that no longer describe it.
    let entries = write_changes_file(&changes_path(&dir), &changes).map_err(|e| e.to_string())?;

    let info = read_doc_info(&dir);
    let doc_type = info
        .as_ref()
        .map(|info| info.doc_type.clone())
        .filter(|value| !value.is_empty())
        .or_else(|| doc_type_from_magic(&bin).map(|value| value.to_string()))
        .unwrap_or_else(|| "word".to_string());
    let name = info
        .as_ref()
        .map(|info| info.name.clone())
        .unwrap_or_else(|| id.clone());
    let path = info.as_ref().and_then(|info| info.path.clone());

    // The folder only holds the document and its changes; the images it
    // points at are in the temp folder of the instance that had it open. The
    // recovered document replaces whatever this instance had, so its images
    // are cleared like any open does, then the crashed instance's copied in.
    crate::file_ops::switch_document(&state)?;

    if let Some(path) = &path {
        *state.current_file.lock().unwrap() = Some(PathBuf::from(path));
    }

    let media_src = crate::temp_layout::recovery_media_source(
        &state.temp_root,
        info.as_ref().and_then(|info| info.instance.as_deref()),
    );
    match crate::temp_layout::copy_media(&media_src, &state.temp_dir.join("media")) {
        Ok(copied) => log_event(&state, &format!("[RECOVER] images copied={}", copied)),
        Err(e) => log_event(&state, &format!("[RECOVER] images not copied: {}", e)),
    }
    // From now on the images live here, and the crashed instance's folder is
    // free to go.
    if let Some(mut info) = info.clone() {
        info.instance = Some(state.instance.id.clone());
        let _ = write_doc_info(&dir, &info);
    }

    log_event(
        &state,
        &format!(
            "[RECOVER] load session={} changes={} bytes={}",
            id,
            changes.len(),
            bin.len()
        ),
    );

    *state.recovery.lock().unwrap() = Some(RecoverySession {
        dir,
        lock,
        base: changes.len(),
        entries,
        saved_count: info.as_ref().map(|info| info.saved_count).unwrap_or(0),
        capped: false,
    });

    Ok(LoadedSession {
        id,
        data: STANDARD.encode(&bin),
        changes,
        name,
        path,
        doc_type,
    })
}

// `id` absent means "all of them": the start screen offers a discard-all next
// to the per-row discard. A folder held by a live instance is left alone.
#[tauri::command]
pub fn recovery_discard(
    app: AppHandle,
    state: State<'_, AppState>,
    id: Option<String>,
) -> Result<String, String> {
    let root = recover_root(&app)?;
    match id {
        Some(id) => {
            if !is_valid_session_id(&id) {
                log_event(&state, &format!("[RECOVER] discard ignored id={}", id));
                return Ok("ok".to_string());
            }
            let dir = root.join(&id);
            if dir.is_dir() && !is_locked(&dir.join(LOCK_FILE)) {
                std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            }
            log_event(&state, &format!("[RECOVER] discard session={}", id));
        }
        None => {
            let mut dropped = 0;
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let dir = entry.path();
                    if dir.is_dir() && !is_locked(&dir.join(LOCK_FILE)) {
                        if std::fs::remove_dir_all(&dir).is_ok() {
                            dropped += 1;
                        }
                    }
                }
            }
            log_event(&state, &format!("[RECOVER] discard all={}", dropped));
        }
    }
    Ok("ok".to_string())
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn scratch() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-recovery-test-{}-{}",
            std::process::id(),
            unique
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: &Path) -> String {
        String::from_utf8(std::fs::read(path).unwrap()).unwrap()
    }

    // --- append -----------------------------------------------------------

    // The format is the upstream's, byte for byte: every change is a quoted
    // record with a trailing comma, so the file is a JSON array once wrapped.
    #[test]
    fn appending_batches_writes_the_upstream_record_format() {
        let dir = scratch();
        let path = changes_path(&dir);
        let mut entries = Vec::new();

        append_changes(&path, &mut entries, &["aa".into(), "bb".into()]).unwrap();
        append_changes(&path, &mut entries, &["cc".into()]).unwrap();

        assert_eq!(read(&path), r#""aa","bb","cc","#);
        assert_eq!(entries, vec![0, 5, 10]);
        assert_eq!(parse_changes_tolerant(&std::fs::read(&path).unwrap()), vec!["aa", "bb", "cc"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_batch_writes_nothing() {
        let dir = scratch();
        let path = changes_path(&dir);
        let mut entries = Vec::new();

        append_changes(&path, &mut entries, &[]).unwrap();

        assert!(entries.is_empty());
        assert!(!path.exists(), "an empty batch must not even create the file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- truncation -------------------------------------------------------

    // What sdkjs means by deleteIndex: keep the first N, drop the rest. An undo
    // arrives in exactly this shape.
    #[test]
    fn delete_index_truncates_to_that_many_changes() {
        let dir = scratch();
        let path = changes_path(&dir);
        let mut entries = Vec::new();
        append_changes(&path, &mut entries, &["a".into(), "b".into(), "c".into(), "d".into()])
            .unwrap();

        assert!(truncate_changes(&path, &mut entries, 0, 2).unwrap());

        assert_eq!(read(&path), r#""a","b","#);
        assert_eq!(entries.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // The whole reason `base` exists: after a recovery the editor starts
    // counting from zero again, while the file already holds the replayed
    // changes. Ignoring the base would throw them away on the first batch.
    #[test]
    fn delete_index_counts_on_top_of_the_recovered_base() {
        let dir = scratch();
        let path = changes_path(&dir);
        let mut entries = write_changes_file(&path, &["r1".into(), "r2".into()]).unwrap();
        append_changes(&path, &mut entries, &["n1".into(), "n2".into()]).unwrap();

        // The editor says "keep 1": one of ITS changes, on top of the two that
        // were replayed.
        assert!(truncate_changes(&path, &mut entries, 2, 1).unwrap());

        assert_eq!(read(&path), r#""r1","r2","n1","#);
        assert_eq!(entries.len(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_negative_delete_index_clamps_to_the_base() {
        let dir = scratch();
        let path = changes_path(&dir);
        let mut entries = write_changes_file(&path, &["r1".into(), "n1".into()]).unwrap();

        assert!(truncate_changes(&path, &mut entries, 1, -5).unwrap());

        assert_eq!(read(&path), r#""r1","#, "the recovered base is never dropped");
        assert_eq!(entries.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // The first batch of a session always carries null, because sdkjs has no
    // saved index yet. Truncating on it would empty the file every time.
    #[test]
    fn a_null_delete_index_truncates_nothing() {
        let dir = scratch();
        let path = changes_path(&dir);
        let mut entries = Vec::new();
        append_changes(&path, &mut entries, &["a".into(), "b".into()]).unwrap();

        // A null index never reaches truncate_changes; what it must not do is
        // behave like a 0. This asserts the two are different.
        let before = read(&path);
        truncate_changes(&path, &mut entries.clone(), 0, 0).unwrap();
        assert_eq!(
            before,
            r#""a","b","#,
            "a delete index of 0 empties the file, so null must not be mapped to it"
        );
        assert_eq!(read(&path), "");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_delete_index_past_the_end_leaves_the_file_alone() {
        let dir = scratch();
        let path = changes_path(&dir);
        let mut entries = Vec::new();
        append_changes(&path, &mut entries, &["a".into(), "b".into()]).unwrap();

        assert!(!truncate_changes(&path, &mut entries, 0, 9).unwrap());

        assert_eq!(read(&path), r#""a","b","#);
        assert_eq!(entries.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- tolerant parsing -------------------------------------------------

    #[test]
    fn a_complete_file_parses_whole() {
        assert_eq!(
            parse_changes_tolerant(br#""aa","bb","cc","#),
            vec!["aa", "bb", "cc"]
        );
    }

    // Cut 1: the process died after the opening quote of a new record.
    #[test]
    fn a_tail_cut_after_the_opening_quote_is_dropped() {
        assert_eq!(parse_changes_tolerant(br#""aa","bb",""#), vec!["aa", "bb"]);
    }

    // Cut 2: died mid-payload.
    #[test]
    fn a_tail_cut_mid_payload_is_dropped() {
        assert_eq!(parse_changes_tolerant(br#""aa","bb","ccc"#), vec!["aa", "bb"]);
    }

    // Cut 3: died between the closing quote and the comma, which is the one
    // case where the record looks complete to a careless reader.
    #[test]
    fn a_tail_cut_before_the_trailing_comma_is_dropped() {
        assert_eq!(parse_changes_tolerant(br#""aa","bb","cc""#), vec!["aa", "bb"]);
    }

    #[test]
    fn an_empty_or_headless_file_yields_no_changes() {
        assert!(parse_changes_tolerant(b"").is_empty());
        assert!(parse_changes_tolerant(br#""half"#).is_empty());
    }

    // The payload sdkjs writes can carry a length prefix (`64;AgAAADEA...`), so
    // the validation has to accept that shape and still reject a record whose
    // bytes are not a payload at all.
    #[test]
    fn the_tail_is_dropped_from_the_first_record_that_is_not_a_change() {
        let raw = br#""64;AgAAADEA//8BANlRq","AgAAADEA","not a change!","AgAA","#;
        assert_eq!(
            parse_changes_tolerant(raw),
            vec!["64;AgAAADEA//8BANlRq", "AgAAADEA"],
            "everything after a corrupt record is suspect, not just that record"
        );
    }

    // --- session ids ------------------------------------------------------

    #[test]
    fn the_ids_we_generate_are_the_ones_accepted() {
        assert!(is_valid_session_id(&format!("{}-{}", 1_756_000_000_000u64, 4321)));
        assert!(is_valid_session_id("0-1"));
    }

    // The id is joined onto the recovery root, so anything that could walk out
    // of it, or name a file instead of a session, has to be refused outright.
    #[test]
    fn an_id_that_is_not_ours_is_refused() {
        for bad in [
            "..",
            "../../recent-files.json",
            "123-45/../..",
            r"123-45\..",
            "12 34-5",
            "abc-1",
            "123-",
            "-45",
            "123",
            "",
        ] {
            assert!(!is_valid_session_id(bad), "{} must not pass for a session id", bad);
        }
    }

    // --- magic ------------------------------------------------------------

    #[test]
    fn the_editor_type_comes_out_of_the_first_four_bytes() {
        assert_eq!(doc_type_from_magic(b"DOCY;v10;0;"), Some("word"));
        assert_eq!(doc_type_from_magic(b"XLSY;v10;0;"), Some("cell"));
        assert_eq!(doc_type_from_magic(b"PPTY;v10;0;"), Some("slide"));
        assert_eq!(doc_type_from_magic(b"PK\x03\x04"), None);
        assert_eq!(doc_type_from_magic(b"DOC"), None, "under four bytes is not a document");
        assert_eq!(doc_type_from_magic(b""), None);
    }

    // --- scanning ---------------------------------------------------------

    struct Folder {
        dir: PathBuf,
    }

    impl Folder {
        fn new(root: &Path, id: &str) -> Folder {
            let dir = root.join(id);
            std::fs::create_dir_all(dir.join("changes")).unwrap();
            Folder { dir }
        }
        fn bin(self, magic: &[u8]) -> Folder {
            std::fs::write(self.dir.join(BIN_FILE), magic).unwrap();
            self
        }
        fn changes(self, n: usize) -> Folder {
            let values: Vec<String> = (0..n).map(|i| format!("AAAA{}", i)).collect();
            write_changes_file(&changes_path(&self.dir), &values).unwrap();
            self
        }
        fn info(self, saved_count: usize, created_ms: u64) -> Folder {
            write_doc_info(
                &self.dir,
                &DocInfo {
                    path: Some("/home/user/informe.docx".into()),
                    format: Some(65),
                    name: "informe.docx".into(),
                    doc_type: "word".into(),
                    app_version: "0.17.19-alpha".into(),
                    saved_count,
                    created_ms,
                    instance: None,
                },
            )
            .unwrap();
            self
        }
    }

    // Real wall clock: a folder whose doc.info is missing dates itself from the
    // mtime of its own changes file, so a made-up "now" would read every such
    // folder as ancient.
    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    fn scan_unlocked(root: &Path) -> Vec<Candidate> {
        scan_candidates(root, now(), |_| false)
    }

    #[test]
    fn a_crashed_session_with_changes_is_offered() {
        let root = scratch();
        Folder::new(&root, "s1").bin(b"DOCY;v10").changes(3).info(0, now() - 1000);

        let found = scan_unlocked(&root);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "s1");
        assert_eq!(found[0].name, "informe.docx");
        assert_eq!(found[0].doc_type, "word");
        assert_eq!(found[0].changes, 3);
        assert_eq!(found[0].path.as_deref(), Some("/home/user/informe.docx"));

        let _ = std::fs::remove_dir_all(&root);
    }

    // A live instance owns its folder. Skipping is not enough: deleting it
    // would pull the recovery data out from under a running editor.
    #[test]
    fn a_locked_folder_is_neither_offered_nor_deleted() {
        let root = scratch();
        Folder::new(&root, "live").bin(b"DOCY;v10").changes(3).info(0, now() - 1000);

        let found = scan_candidates(&root, now(), |_| true);

        assert!(found.is_empty());
        assert!(root.join("live").exists(), "another instance is still using it");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_without_changes_is_deleted() {
        let root = scratch();
        Folder::new(&root, "empty").bin(b"DOCY;v10").changes(0).info(0, now() - 1000);

        assert!(scan_unlocked(&root).is_empty());
        assert!(!root.join("empty").exists(), "there is nothing to recover from it");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_without_a_usable_editor_bin_is_deleted() {
        let root = scratch();
        Folder::new(&root, "short").bin(b"DO").changes(3).info(0, now() - 1000);
        Folder::new(&root, "alien").bin(b"PK\x03\x04zip").changes(3).info(0, now() - 1000);

        assert!(scan_unlocked(&root).is_empty());
        assert!(!root.join("short").exists());
        assert!(
            !root.join("alien").exists(),
            "changes cannot be replayed onto something that is not a document"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // The clean-close residue: the last save wrote every change into the
    // user's file, so offering it back would be offering a copy of what they
    // already have.
    #[test]
    fn a_folder_whose_changes_are_all_saved_is_deleted() {
        let root = scratch();
        Folder::new(&root, "saved").bin(b"DOCY;v10").changes(3).info(3, now() - 1000);

        assert!(scan_unlocked(&root).is_empty());
        assert!(!root.join("saved").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    // One change past the last save is still work worth offering.
    #[test]
    fn a_folder_with_one_change_past_the_last_save_is_offered() {
        let root = scratch();
        Folder::new(&root, "dirty").bin(b"DOCY;v10").changes(4).info(3, now() - 1000);

        let found = scan_unlocked(&root);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].changes, 4);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_older_than_the_age_limit_is_deleted() {
        let root = scratch();
        Folder::new(&root, "old")
            .bin(b"DOCY;v10")
            .changes(3)
            .info(0, now() - MAX_AGE_MS - 1);
        Folder::new(&root, "fresh")
            .bin(b"DOCY;v10")
            .changes(3)
            .info(0, now() - MAX_AGE_MS + 60_000);

        let found = scan_unlocked(&root);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "fresh");
        assert!(!root.join("old").exists(), "a crash from a fortnight ago has been redone");

        let _ = std::fs::remove_dir_all(&root);
    }

    // doc.info is the metadata, not the evidence: a folder that lost it still
    // holds a replayable document, and the type is in the binary anyway.
    #[test]
    fn a_folder_without_doc_info_is_still_offered_with_the_type_from_the_magic() {
        let root = scratch();
        Folder::new(&root, "s1").bin(b"XLSY;v10").changes(2);

        let found = scan_unlocked(&root);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].doc_type, "cell");
        assert_eq!(found[0].path, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scanning_a_root_that_does_not_exist_yet_is_a_noop() {
        let root = scratch().join("never-created");
        assert!(scan_unlocked(&root).is_empty());
    }
}
