// Files waiting for the page to open them: the launch argument, and on macOS
// the files the Finder hands over after launch (double click, Open With).
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

// Tells the page that the queue has something new. It carries no data: the
// page takes each file with take_pending_open_file.
pub const PENDING_OPEN_EVENT: &str = "pending-open-file";

#[derive(Default)]
pub struct PendingOpens {
    queue: Mutex<VecDeque<PathBuf>>,
}

impl PendingOpens {
    pub fn with_launch_file(path: Option<PathBuf>) -> Self {
        let pending = Self::default();
        if let Some(path) = path {
            pending.push(path);
        }
        pending
    }

    pub fn push(&self, path: PathBuf) {
        self.queue.lock().unwrap().push_back(path);
    }

    // Each path comes out once, in the order it went in.
    pub fn take(&self) -> Option<PathBuf> {
        self.queue.lock().unwrap().pop_front()
    }
}

// Only a local file URL with an absolute path is opened. The Err names the
// reason for the log, without the path.
pub fn opened_file_path(url: &tauri::Url) -> Result<PathBuf, &'static str> {
    if url.scheme() != "file" {
        return Err("not a file URL");
    }
    let Ok(path) = url.to_file_path() else {
        return Err("no local path");
    };
    absolute_only(path)
}

// A file URL always parses to an absolute path; this keeps the rule explicit.
fn absolute_only(path: PathBuf) -> Result<PathBuf, &'static str> {
    if !path.is_absolute() {
        return Err("relative path");
    }
    Ok(path)
}

// Queues the files among `urls` and logs the rest. Returns how many were queued.
pub fn queue_opened_urls(
    pending: &PendingOpens,
    urls: Vec<tauri::Url>,
    mut log: impl FnMut(&str),
) -> usize {
    let mut queued = 0;
    for url in urls {
        match opened_file_path(&url) {
            Ok(path) => {
                log(&format!(
                    "[OPEN] handed over by the system: {}",
                    file_name(&path)
                ));
                pending.push(path);
                queued += 1;
            }
            Err(reason) => {
                log(&format!(
                    "[OPEN] ignored a {} URL: {}",
                    url.scheme(),
                    reason
                ));
            }
        }
    }
    queued
}

fn file_name(path: &std::path::Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
}

// RunEvent::Opened only exists on macOS, iOS and Android; this is what it
// calls there. The page may not be listening yet: the event can be lost, and
// the request the page makes on DOMContentLoaded collects the queue instead.
#[cfg_attr(
    not(any(target_os = "macos", target_os = "ios", target_os = "android")),
    allow(dead_code)
)]
pub fn on_opened<R: tauri::Runtime>(app: &tauri::AppHandle<R>, urls: Vec<tauri::Url>) {
    use tauri::{Emitter, Manager};
    let state = app.state::<crate::file_ops::AppState>();
    let log = |msg: &str| crate::file_ops::log_event(&state, msg);
    let queued = queue_opened_urls(&state.pending_open, urls, log);
    if queued == 0 {
        return;
    }
    let sent = app.emit_to(crate::window_state::MAIN_WINDOW, PENDING_OPEN_EVENT, ());
    if let Err(e) = sent {
        crate::file_ops::log_event(&state, &format!("[OPEN] could not tell the page: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> tauri::Url {
        tauri::Url::parse(text).unwrap()
    }

    #[test]
    fn queue_hands_out_paths_in_order_and_once() {
        let pending = PendingOpens::with_launch_file(Some(PathBuf::from("/a/first.docx")));
        pending.push(PathBuf::from("/a/second.xlsx"));
        pending.push(PathBuf::from("/a/third.pptx"));

        assert_eq!(pending.take(), Some(PathBuf::from("/a/first.docx")));
        assert_eq!(pending.take(), Some(PathBuf::from("/a/second.xlsx")));
        assert_eq!(pending.take(), Some(PathBuf::from("/a/third.pptx")));
        assert_eq!(pending.take(), None);
        assert_eq!(pending.take(), None);
    }

    #[test]
    fn queue_without_launch_file_starts_empty() {
        let pending = PendingOpens::with_launch_file(None);
        assert_eq!(pending.take(), None);
    }

    #[test]
    fn file_url_gives_its_absolute_path() {
        let path = opened_file_path(&url("file:///Users/me/Documents/My%20doc.docx"));
        assert_eq!(path, Ok(PathBuf::from("/Users/me/Documents/My doc.docx")));
    }

    #[test]
    fn other_schemes_are_refused() {
        assert_eq!(
            opened_file_path(&url("https://example.com/doc.docx")),
            Err("not a file URL")
        );
        assert_eq!(
            opened_file_path(&url("euro-office://open?doc=1")),
            Err("not a file URL")
        );
    }

    #[test]
    fn file_url_on_another_host_is_refused() {
        assert_eq!(
            opened_file_path(&url("file://server/share/doc.docx")),
            Err("no local path")
        );
    }

    #[test]
    fn relative_path_is_refused() {
        assert_eq!(
            absolute_only(PathBuf::from("doc.docx")),
            Err("relative path")
        );
        assert_eq!(
            absolute_only(PathBuf::from("/doc.docx")),
            Ok(PathBuf::from("/doc.docx"))
        );
    }

    #[test]
    fn opened_urls_queue_the_files_and_log_the_rest() {
        let pending = PendingOpens::with_launch_file(Some(PathBuf::from("/launch.docx")));
        let urls = vec![
            url("file:///Users/me/a.docx"),
            url("https://example.com/secret/b.docx"),
            url("file:///Users/me/c.xlsx"),
        ];
        let mut lines = Vec::new();

        let queued = queue_opened_urls(&pending, urls, |line| lines.push(line.to_string()));

        assert_eq!(queued, 2);
        assert_eq!(pending.take(), Some(PathBuf::from("/launch.docx")));
        assert_eq!(pending.take(), Some(PathBuf::from("/Users/me/a.docx")));
        assert_eq!(pending.take(), Some(PathBuf::from("/Users/me/c.xlsx")));
        assert_eq!(pending.take(), None);
        assert_eq!(
            lines,
            vec![
                "[OPEN] handed over by the system: a.docx",
                "[OPEN] ignored a https URL: not a file URL",
                "[OPEN] handed over by the system: c.xlsx",
            ]
        );
    }

    #[test]
    fn no_file_urls_queue_nothing() {
        let pending = PendingOpens::default();
        let queued = queue_opened_urls(&pending, vec![url("mailto:a@b.c")], |_| {});
        assert_eq!(queued, 0);
        assert_eq!(pending.take(), None);
    }
}
