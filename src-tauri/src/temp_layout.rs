// Where the app keeps its temporary files.
//
// The root is shared by every running instance and only holds what is safe
// to share: the font cache and the logs. Everything that belongs to the open
// document (the Editor.bin the editor saves from, the images x2t resolves
// next to it, x2t's own scratch) lives in instances/<millis>-<pid>/, one
// folder per process. A folder is alive while its process holds the lock
// inside it; the OS drops that lock when the process dies, even on a kill,
// which is what lets the next start tell leftovers from live work.

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const INSTANCES_DIR: &str = "instances";
const INSTANCE_LOCK: &str = "instance.lock";
const FONTDATA_DIR: &str = "fontdata";
pub const FONTS_MARKER: &str = ".fonts_generated";
const FONTS_LOCK: &str = "fonts.lock";
const FONTS_STAGING_PREFIX: &str = "fontdata.new-";
const LOG_PREFIX: &str = "js-debug-";
const LOG_SUFFIX: &str = ".log";

// Logs of dead instances kept for reading after a crash.
pub const LOGS_KEPT: usize = 10;

// A folder without its lock file is either being created this instant or was
// left by a start that died between creating it and locking it. Only its age,
// which the id carries, tells the two apart.
pub const UNLOCKED_GRACE_MS: u64 = 24 * 60 * 60 * 1000;

// No folder younger than this is removed, lock or not: on Unix the lock file
// is created a moment before it is locked, and in that moment a starting
// instance looks exactly like a dead one.
pub const MIN_SWEEP_AGE_MS: u64 = 60 * 1000;

// ---------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------

// No crate for this on purpose: a file-locking crate would be a dependency for
// two calls. Windows gets it from the share mode, Unix from flock.
#[cfg(target_os = "windows")]
fn open_locked(path: &Path, create: bool) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .share_mode(0)
        .open(path)
}

#[cfg(not(target_os = "windows"))]
fn open_locked(path: &Path, create: bool) -> std::io::Result<File> {
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .open(path)?;
    // Non-blocking: a lock that is taken has to answer "taken" right away.
    let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if taken != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

pub fn take_lock(path: &Path) -> std::io::Result<File> {
    open_locked(path, true)
}

// A missing lock file is not a live process, and this must not create one:
// checking would then turn every folder it looked at into one holding a lock.
pub fn is_locked(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    open_locked(path, false).is_err()
}

// ---------------------------------------------------------------------------
// Instances
// ---------------------------------------------------------------------------

// <unix_millis>-<pid>: sorts by start time and stays unique when the OS reuses
// a pid.
fn parse_id(id: &str) -> Option<(u64, u32)> {
    let (millis, pid) = id.split_once('-')?;
    if millis.is_empty() || !millis.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((millis.parse().ok()?, pid.parse().ok()?))
}

pub fn instance_dir(root: &Path, id: &str) -> PathBuf {
    root.join(INSTANCES_DIR).join(id)
}

pub struct Instance {
    pub id: String,
    pub dir: PathBuf,
    // Held for the whole life of the process: releasing it is what marks the
    // folder as a leftover.
    lock: Mutex<Option<File>>,
}

impl Instance {
    pub fn holds_lock(&self) -> bool {
        self.lock.lock().map(|lock| lock.is_some()).unwrap_or(false)
    }

    // Ends the instance on a normal exit. The lock goes first: on Windows a
    // folder cannot be removed while this process still holds a file in it.
    pub fn end(&self) {
        if let Ok(mut lock) = self.lock.lock() {
            lock.take();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn begin_instance(root: &Path, now_ms: u64, pid: u32) -> std::io::Result<Instance> {
    let id = format!("{}-{}", now_ms, pid);
    let dir = instance_dir(root, &id);
    std::fs::create_dir_all(&dir)?;
    let lock = take_lock(&dir.join(INSTANCE_LOCK))?;
    Ok(Instance {
        id,
        dir,
        lock: Mutex::new(Some(lock)),
    })
}

// Last resort when the lock cannot be taken: the folder still works, and the
// grace period keeps other instances from removing it for a day.
pub fn unlocked_instance(root: &Path, now_ms: u64, pid: u32) -> Instance {
    let id = format!("{}-{}", now_ms, pid);
    let dir = instance_dir(root, &id);
    let _ = std::fs::create_dir_all(&dir);
    Instance {
        id,
        dir,
        lock: Mutex::new(None),
    }
}

pub fn instance_alive(root: &Path, id: &str) -> bool {
    is_locked(&instance_dir(root, id).join(INSTANCE_LOCK))
}

// Removes the folders of instances that are gone. In doubt a folder stays: a
// leftover only takes disk space, removing a live one loses the document's
// images. `referenced` are the instances whose images a crashed document still
// needs to be recovered with.
pub fn sweep_dead_instances(
    root: &Path,
    now_ms: u64,
    referenced: &HashSet<String>,
    is_locked: impl Fn(&Path) -> bool,
) -> Vec<String> {
    let mut removed = Vec::new();
    let Ok(entries) = std::fs::read_dir(root.join(INSTANCES_DIR)) else {
        return removed;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Only folders begin_instance makes.
        let Some((created_ms, _)) = parse_id(&name) else {
            continue;
        };
        if now_ms.saturating_sub(created_ms) < MIN_SWEEP_AGE_MS {
            continue;
        }
        let lock = dir.join(INSTANCE_LOCK);
        if !lock.exists() {
            if now_ms.saturating_sub(created_ms) < UNLOCKED_GRACE_MS {
                continue;
            }
        } else if is_locked(&lock) {
            continue;
        }
        if referenced.contains(&name) {
            continue;
        }
        if std::fs::remove_dir_all(&dir).is_ok() {
            removed.push(name);
        }
    }
    removed
}

// ---------------------------------------------------------------------------
// Shared font cache
// ---------------------------------------------------------------------------

pub fn fontdata_dir(root: &Path) -> PathBuf {
    root.join(FONTDATA_DIR)
}

pub fn fonts_ready(root: &Path) -> bool {
    root.join(FONTS_MARKER).exists()
        && std::fs::metadata(fontdata_dir(root).join("AllFonts.js"))
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
}

// Moves each generated file over its counterpart in fontdata/. A rename
// replaces in one step, so an instance reading the cache meanwhile gets the
// old file or the new one, never half of one.
fn install_font_cache(staging: &Path, fontdata: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(fontdata)?;
    for entry in std::fs::read_dir(staging)?.flatten() {
        let target = fontdata.join(entry.file_name());
        if entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(&target);
        }
        std::fs::rename(entry.path(), &target)?;
    }
    let _ = std::fs::remove_dir_all(staging);
    Ok(())
}

#[derive(Debug, PartialEq)]
pub enum FontCache {
    Ready,
    Generated,
    GeneratedByOther,
    Failed,
    GaveUp,
}

// Builds the shared font cache if it is missing. One instance generates at a
// time; one that starts meanwhile waits for that run instead of writing the
// same files, and after `timeout` goes on without it. `generate` writes into
// the folder it is handed and reports success.
pub fn ensure_font_cache(
    root: &Path,
    instance_id: &str,
    generate: impl FnOnce(&Path) -> bool,
    timeout: std::time::Duration,
    poll: std::time::Duration,
) -> FontCache {
    if fonts_ready(root) {
        return FontCache::Ready;
    }
    let started = std::time::Instant::now();
    let mut waited = false;
    let lock = loop {
        match take_lock(&root.join(FONTS_LOCK)) {
            Ok(lock) => break lock,
            Err(_) if started.elapsed() >= timeout => return FontCache::GaveUp,
            Err(_) => {
                waited = true;
                std::thread::sleep(poll);
            }
        }
    };
    if fonts_ready(root) {
        return if waited {
            FontCache::GeneratedByOther
        } else {
            FontCache::Ready
        };
    }
    // Holding the lock means nobody else is generating, so a staging folder
    // still here is what a generator that died left behind.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(FONTS_STAGING_PREFIX) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let staging = root.join(format!("{}{}", FONTS_STAGING_PREFIX, instance_id));
    let generated = std::fs::create_dir_all(&staging).is_ok() && generate(&staging);
    let installed = generated
        && install_font_cache(&staging, &fontdata_dir(root)).is_ok()
        && write_replacing(&root.join(FONTS_MARKER), b"generated", instance_id).is_ok();
    let _ = std::fs::remove_dir_all(&staging);
    drop(lock);
    if installed {
        FontCache::Generated
    } else {
        FontCache::Failed
    }
}

fn write_replacing(path: &Path, bytes: &[u8], tag: &str) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", tag));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

// ---------------------------------------------------------------------------
// Recovery images
// ---------------------------------------------------------------------------

// Where the images of a crashed document are. The recovery folder records the
// instance it was opened in; folders written before that existed point at
// the root's media/, where every instance used to keep them.
pub fn recovery_media_source(root: &Path, instance: Option<&str>) -> PathBuf {
    match instance.filter(|id| parse_id(id).is_some()) {
        Some(id) => instance_dir(root, id).join("media"),
        None => root.join("media"),
    }
}

// Copies, never moves: the source can belong to an instance of an older
// version that is still running.
pub fn copy_media(src: &Path, dest: &Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(src) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    std::fs::create_dir_all(dest)?;
    let mut copied = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            std::fs::copy(&path, dest.join(entry.file_name()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

// ---------------------------------------------------------------------------
// Document images
// ---------------------------------------------------------------------------
//
// Every document an instance opens gets the next number, and the editor
// addresses its images as docmedia/<instance>/<number>/media/<name>. sdkjs
// copies an image only when its address is not under the open document's, so
// this is what makes a paste from another document bring that document's
// image instead of whatever this one holds under the same name.

const DOC_SEQ_FILE: &str = "doc.seq";
const PREVIOUS_DOCS_DIR: &str = "previous";

// Earlier documents of an instance whose images stay pasteable.
pub const PREVIOUS_DOCS_KEPT: u64 = 3;

pub fn doc_key(id: &str, seq: u64) -> String {
    format!("{}/{}", id, seq)
}

// What follows the scheme and host of an ascdesktop request: WebKitGTK and
// WKWebView send ascdesktop://<path>, at times with a localhost host, and
// WebView2 http://ascdesktop.localhost/<path>.
pub fn protocol_path(uri: &str) -> &str {
    let path = uri
        .strip_prefix("ascdesktop://")
        .or_else(|| uri.strip_prefix("ascdesktop:///"))
        .or_else(|| uri.strip_prefix("http://ascdesktop.localhost/"))
        .or_else(|| uri.strip_prefix("https://ascdesktop.localhost/"))
        .unwrap_or("");
    path.strip_prefix("localhost/").unwrap_or(path)
}

// The bridge asks for ascdesktop://doc-key. WebKit reads doc-key as the host
// and hands the request over as ascdesktop://doc-key/, empty path made "/".
pub fn is_doc_key_path(path: &str) -> bool {
    path == "doc-key" || path == "doc-key/"
}

// Tells other instances which document the media/ of this one belongs to.
pub fn write_doc_seq(dir: &Path, seq: u64) -> std::io::Result<()> {
    write_replacing(&dir.join(DOC_SEQ_FILE), seq.to_string().as_bytes(), "seq")
}

pub fn read_doc_seq(dir: &Path) -> Option<u64> {
    std::fs::read_to_string(dir.join(DOC_SEQ_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn previous_media_dir(dir: &Path, seq: u64) -> PathBuf {
    dir.join(PREVIOUS_DOCS_DIR).join(seq.to_string()).join("media")
}

fn has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

#[derive(Debug, PartialEq)]
pub enum OutgoingImages {
    None,
    Kept,
    // Could not be moved aside, so they were removed as before; a later paste
    // from that document finds nothing.
    Dropped(String),
}

pub fn begin_document(dir: &Path, old_seq: u64, new_seq: u64) -> Result<OutgoingImages, String> {
    begin_document_with(
        dir,
        old_seq,
        new_seq,
        |from, to| std::fs::rename(from, to),
        |path| std::fs::remove_dir_all(path),
    )
}

// The instance is replacing its document: the outgoing one's images leave
// media/ before the next document's arrive. Order matters to an instance
// reading them meanwhile (read_document_image): they reach previous/<old>/ before
// the number moves on, and the new document only writes after it has.
// Err means media/ still holds some of them, and opening another document on
// top would mix two documents' images under the same names.
pub fn begin_document_with(
    dir: &Path,
    old_seq: u64,
    new_seq: u64,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
    remove: impl Fn(&Path) -> std::io::Result<()>,
) -> Result<OutgoingImages, String> {
    let media = dir.join("media");
    let mut outcome = OutgoingImages::None;
    if has_entries(&media) {
        let dest = previous_media_dir(dir, old_seq);
        let mut last_error = String::new();
        let mut moved = false;
        match dest.parent().map(std::fs::create_dir_all) {
            Some(Ok(())) => {
                // Windows refuses while a file inside is open; that is brief.
                for attempt in 0..3 {
                    if attempt > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    match rename(&media, &dest) {
                        Ok(()) => {
                            moved = true;
                            break;
                        }
                        Err(e) => last_error = e.to_string(),
                    }
                }
            }
            Some(Err(e)) => last_error = e.to_string(),
            None => last_error = "no parent folder".to_string(),
        }
        outcome = if moved {
            OutgoingImages::Kept
        } else {
            let _ = remove(&media);
            OutgoingImages::Dropped(last_error)
        };
    }
    if has_entries(&media) {
        return Err(format!(
            "the previous document's images could not be cleared from {}",
            media.display()
        ));
    }
    write_doc_seq(dir, new_seq).map_err(|e| format!("cannot record the open document: {}", e))?;
    prune_previous(dir, new_seq, PREVIOUS_DOCS_KEPT);
    Ok(outcome)
}

pub fn prune_previous(dir: &Path, current_seq: u64, keep: u64) -> Vec<u64> {
    let mut removed = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir.join(PREVIOUS_DOCS_DIR)) else {
        return removed;
    };
    for entry in entries.flatten() {
        let Some(seq) = entry.file_name().to_str().and_then(|name| name.parse::<u64>().ok()) else {
            continue;
        };
        if seq.saturating_add(keep) < current_seq && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed.push(seq);
        }
    }
    removed
}

#[derive(Debug, PartialEq, Clone)]
pub struct MediaRef {
    pub instance: String,
    pub seq: u64,
    pub name: String,
}

// One path component that stays inside media/.
fn is_plain_name(name: &str) -> bool {
    let separator = |c: char| c == '/' || c == '\\' || c == '\0' || (cfg!(windows) && c == ':');
    !name.is_empty() && name != "." && name != ".." && !name.contains(separator)
}

// `path` is percent-decoded. sdkjs sometimes appends a full image address to
// the document's, so the last docmedia/ is the one that names the image.
pub fn parse_media_path(path: &str) -> Option<MediaRef> {
    let start = path.rfind("docmedia/")? + "docmedia/".len();
    let rest = path[start..].replace("media/media/", "media/");
    let mut parts = rest.splitn(4, '/');
    let instance = parts.next()?;
    let seq = parts.next()?;
    let media = parts.next()?;
    let name = parts.next()?;
    if media != "media" || parse_id(instance).is_none() || !is_plain_name(name) {
        return None;
    }
    if seq.is_empty() || !seq.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(MediaRef {
        instance: instance.to_string(),
        seq: seq.parse().ok()?,
        name: name.to_string(),
    })
}

pub struct OpenDocument<'a> {
    pub root: &'a Path,
    pub id: &'a str,
    pub dir: &'a Path,
    pub seq: u64,
}

// What docmedia/ serves: only the open document's images. Any other address
// is not found, so the screen never shows an image the file will not keep.
pub fn serve_media_path(doc: &OpenDocument, path: &str) -> Option<PathBuf> {
    let image = parse_media_path(path)?;
    (image.instance == doc.id && image.seq == doc.seq).then(|| doc.dir.join("media").join(image.name))
}

// The bytes behind an image address of any instance and document, or None when
// they are gone. `between_reads` lets tests switch documents mid-read.
pub fn read_document_image(root: &Path, image: &MediaRef, between_reads: impl Fn()) -> Option<Vec<u8>> {
    let dir = instance_dir(root, &image.instance);
    if read_doc_seq(&dir) == Some(image.seq) {
        if let Ok(bytes) = std::fs::read(dir.join("media").join(&image.name)) {
            between_reads();
            // A switch in between may have put the next document's file under
            // this name: only a number that did not move proves these bytes.
            if read_doc_seq(&dir) == Some(image.seq) {
                return Some(bytes);
            }
        }
    }
    std::fs::read(previous_media_dir(&dir, image.seq).join(&image.name)).ok()
}

// Brings the image at `url` into the open document's media/ and returns the
// name to reference it by. None when its source is gone: the caller must leave
// the image missing rather than fall back to a same-named one of its own.
pub fn import_media(
    doc: &OpenDocument,
    url: &str,
    stage: impl FnOnce(&Path, &str, &[u8]) -> Option<String>,
) -> Option<String> {
    let image = parse_media_path(url)?;
    if image.instance == doc.id && image.seq == doc.seq {
        let own = doc.dir.join("media").join(&image.name);
        return own.is_file().then(|| image.name);
    }
    let bytes = read_document_image(doc.root, &image, || {})?;
    let media = doc.dir.join("media");
    std::fs::create_dir_all(&media).ok()?;
    stage(&media, &image.name, &bytes)
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn log_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{}{}{}", LOG_PREFIX, id, LOG_SUFFIX))
}

fn log_id(name: &str) -> Option<String> {
    let id = name.strip_prefix(LOG_PREFIX)?.strip_suffix(LOG_SUFFIX)?;
    parse_id(id).map(|_| id.to_string())
}

// Each instance writes its own file: a shared one was emptied by every start
// and then written by every process at once.
pub fn start_log(root: &Path, id: &str, first_line: &str) -> PathBuf {
    let path = log_path(root, id);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{}", first_line);
    }
    let _ = LOG_PATH.set(path.clone());
    path
}

pub fn append_log(line: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{}", line);
    }
}

// Keeps the logs of live instances and the newest `keep` of the dead ones, so
// a crash can still be read after the next start. Any other file in the root
// is left alone.
pub fn prune_logs(root: &Path, keep: usize, is_alive: impl Fn(&str) -> bool) -> Vec<String> {
    let mut dead: Vec<String> = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| log_id(&entry.file_name().to_string_lossy()))
                .filter(|id| !is_alive(id))
                .collect()
        })
        .unwrap_or_default();
    dead.sort_by_key(|id| std::cmp::Reverse(parse_id(id)));
    let mut removed = Vec::new();
    for id in dead.into_iter().skip(keep) {
        if std::fs::remove_file(log_path(root, &id)).is_ok() {
            removed.push(id);
        }
    }
    removed
}

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

pub struct RootChoice {
    pub path: PathBuf,
    // Why the preferred folder was not used, for the log.
    pub note: Option<String>,
}

// %TEMP% is already per user on Windows.
#[cfg(target_os = "windows")]
pub fn temp_root(_identifier: &str) -> Result<RootChoice, String> {
    Ok(RootChoice {
        path: std::env::temp_dir().join("euro-office-lite"),
        note: None,
    })
}

// On Linux and macOS the temp folder is shared by every account on the
// machine, so the root is one folder per user, private to that user. If it
// cannot be had (something else already sits under that name), the app's
// cache folder in the user's home is next, which also keeps the font cache
// across restarts. After that, a folder made new for this start: first in the
// user's cache folder, then in the temp folder. Err when none can be had.
#[cfg(unix)]
pub fn temp_root(identifier: &str) -> Result<RootChoice, String> {
    let uid = unsafe { libc::getuid() };
    #[cfg(target_os = "macos")]
    let shared_tmp = PathBuf::from("/tmp");
    #[cfg(not(target_os = "macos"))]
    let shared_tmp = std::env::temp_dir();

    let mut candidates = vec![shared_tmp.join(format!("euro-office-lite-{}", uid))];
    let mut fresh_in = Vec::new();
    if let Some(cache) = user_cache_dir() {
        let app_cache = cache.join(identifier);
        candidates.push(app_cache.join("tmp"));
        fresh_in.push(app_cache);
    }
    fresh_in.push(shared_tmp);
    let counter = std::sync::atomic::AtomicU32::new(0);
    choose_root(&candidates, &fresh_in, uid, || {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        format!(
            "euro-office-lite-{}-{}-{}-{}",
            uid,
            std::process::id(),
            nanos,
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
    })
}

// Where Tauri's app_cache_dir lives, worked out before Tauri is up.
#[cfg(target_os = "macos")]
fn user_cache_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join("Library/Caches"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn user_cache_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        let xdg = PathBuf::from(xdg);
        if xdg.is_absolute() {
            return Some(xdg);
        }
    }
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join(".cache"))
}

// Names tried per folder before moving on to the next one.
#[cfg(unix)]
const FRESH_TRIES: usize = 10;

// The first candidate that is, or can be made, a private folder of this user;
// failing that, a folder created new under one of `fresh_in`. Only a folder
// this call created itself can be a new one, so nothing that already existed
// under that name is taken, and every path returned has passed the checks.
#[cfg(unix)]
pub fn choose_root(
    candidates: &[PathBuf],
    fresh_in: &[PathBuf],
    uid: u32,
    fresh_name: impl Fn() -> String,
) -> Result<RootChoice, String> {
    let mut reasons = Vec::new();
    for candidate in candidates {
        match ensure_private_dir(candidate, uid) {
            Ok(()) => {
                return Ok(RootChoice {
                    path: candidate.clone(),
                    note: (!reasons.is_empty()).then(|| reasons.join("; ")),
                })
            }
            Err(reason) => reasons.push(format!("{}: {}", candidate.display(), reason)),
        }
    }
    for parent in fresh_in {
        if !parent.exists() && std::fs::create_dir_all(parent).is_err() {
            reasons.push(format!("{}: cannot be created", parent.display()));
            continue;
        }
        let mut last_error = String::new();
        for _ in 0..FRESH_TRIES {
            let path = parent.join(fresh_name());
            let made = create_private_dir(&path)
                .map_err(|e| e.to_string())
                .and_then(|()| check_private_dir(&path, uid));
            match made {
                Ok(()) => {
                    reasons.push(format!("using a new folder in {}", parent.display()));
                    return Ok(RootChoice {
                        path,
                        note: Some(reasons.join("; ")),
                    });
                }
                Err(reason) => last_error = reason,
            }
        }
        reasons.push(format!(
            "{}: no new folder could be made ({})",
            parent.display(),
            last_error
        ));
    }
    Err(reasons.join("; "))
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

// Creates the folder private (0700) if it is missing, then checks what is
// actually there.
#[cfg(unix)]
pub fn ensure_private_dir(path: &Path, uid: u32) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    match create_private_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    check_private_dir(path, uid)
}

// Looks at the folder without following links: a real directory, owned by
// `uid`, with no access for group or others.
#[cfg(unix)]
fn check_private_dir(path: &Path, uid: u32) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_dir() {
        return Err("not a plain folder".to_string());
    }
    if meta.uid() != uid {
        return Err(format!("owned by uid {}", meta.uid()));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(format!("permissions {:o}", meta.mode() & 0o777));
    }
    Ok(())
}

#[cfg(test)]
#[path = "temp_layout_tests.rs"]
mod tests;
