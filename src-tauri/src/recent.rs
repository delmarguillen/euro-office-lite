use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

// The list lives on the start screen, where it has to stay readable at a
// glance; ten entries fill it without turning into a scrolling history.
const MAX_ENTRIES: usize = 10;
const STORE_FILE: &str = "recent-files.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct RecentEntry {
    pub path: String,
    pub opened_at: u64,
}

#[derive(Serialize, Deserialize)]
pub struct RecentStore {
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub files: Vec<RecentEntry>,
}

fn enabled_default() -> bool {
    true
}

impl Default for RecentStore {
    fn default() -> Self {
        RecentStore {
            enabled: enabled_default(),
            files: Vec::new(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFileView {
    path: String,
    name: String,
    opened_at: u64,
    file_type: i32,
}

#[derive(Serialize)]
pub struct RecentState {
    enabled: bool,
    files: Vec<RecentFileView>,
}

// Windows and macOS ship case insensitive file systems by default, so the same
// document reached through two spellings must collapse into one entry; on Linux
// the two spellings really are two files, and a backslash is a legal name
// character, so nothing is normalized there.
#[cfg(target_os = "windows")]
fn normalize(value: &str) -> String {
    value.replace('\\', "/").to_lowercase()
}

#[cfg(target_os = "macos")]
fn normalize(value: &str) -> String {
    value.to_lowercase()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn normalize(value: &str) -> String {
    value.to_string()
}

fn store_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(STORE_FILE))
}

// A missing, unreadable or corrupt store must never block the start screen:
// fall back to an empty list, which the next recorded opening rewrites.
fn load(app: &AppHandle) -> RecentStore {
    store_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<RecentStore>(&raw).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, store: &RecentStore) {
    let path = match store_path(app) {
        Some(path) => path,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = match serde_json::to_string_pretty(store) {
        Ok(json) => json,
        Err(_) => return,
    };
    // Write through a temporary file so a crash mid-write cannot leave a
    // truncated JSON behind; rename replaces the destination on all three
    // platforms.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json.as_bytes()).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

// Called from the Rust open and save-as paths so every way of reaching a
// document (start screen, editor File menu, reopen after reload, file
// association or command line) feeds the list without the frontend having to
// remember to.
pub fn record(app: &AppHandle, path: &str) {
    let mut store = load(app);
    if !store.enabled {
        return;
    }
    let key = normalize(path);
    store.files.retain(|entry| normalize(&entry.path) != key);
    store.files.insert(
        0,
        RecentEntry {
            path: path.to_string(),
            opened_at: now_secs(),
        },
    );
    store.files.truncate(MAX_ENTRIES);
    save(app, &store);
}

#[tauri::command]
pub async fn recent_files_state(app: AppHandle) -> RecentState {
    let store = load(&app);
    let files = store
        .files
        .iter()
        // A moved or deleted file stays in the store but is not offered: an
        // unmounted drive or an offline share comes back on its own instead of
        // being purged on the one run where it happened to be unreachable.
        .filter(|entry| Path::new(&entry.path).is_file())
        .map(|entry| RecentFileView {
            path: entry.path.clone(),
            name: Path::new(&entry.path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.path.clone()),
            opened_at: entry.opened_at,
            file_type: crate::file_ops::detect_format(&PathBuf::from(&entry.path)),
        })
        .collect();
    RecentState {
        enabled: store.enabled,
        files,
    }
}

#[tauri::command]
pub async fn set_recent_files_enabled(app: AppHandle, enabled: bool) {
    let mut store = load(&app);
    store.enabled = enabled;
    save(&app, &store);
}

#[tauri::command]
pub async fn clear_recent_files(app: AppHandle) {
    let mut store = load(&app);
    store.files.clear();
    save(&app, &store);
}
