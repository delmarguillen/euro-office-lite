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

#[cfg(target_os = "macos")]
pub fn temp_root() -> PathBuf {
    PathBuf::from("/tmp/euro-office-lite")
}

#[cfg(not(target_os = "macos"))]
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("euro-office-lite")
}

#[cfg(test)]
#[path = "temp_layout_tests.rs"]
mod tests;
