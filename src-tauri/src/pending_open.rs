// Files waiting for the page to open them: the launch argument, and on macOS
// the files the Finder hands over after launch (double click, Open With).
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// Tells the page that the queue has something new. It carries no data: the
// page takes each file with take_pending_open_file.
pub const PENDING_OPEN_EVENT: &str = "pending-open-file";

#[derive(Debug, PartialEq, Eq)]
pub struct PendingFile {
    pub path: PathBuf,
    // A template to start a new document from, not to edit (starts_new_document).
    pub new_document: bool,
}

#[derive(Default)]
pub struct PendingOpens {
    queue: Mutex<VecDeque<PendingFile>>,
}

impl PendingOpens {
    pub fn with_launch_file(file: Option<PendingFile>) -> Self {
        let pending = Self::default();
        if let Some(file) = file {
            pending.push(file);
        }
        pending
    }

    pub fn push(&self, file: PendingFile) {
        self.queue.lock().unwrap().push_back(file);
    }

    // Each file comes out once, in the order it went in.
    pub fn take(&self) -> Option<PendingFile> {
        self.queue.lock().unwrap().pop_front()
    }
}

// A template handed to the app starts a new document, as in Office, unless one
// of the app's own windows asked to edit it: File > Open and Recent with a
// document open start a process with --new-window. --from-template, which
// new_document_in_new_window adds, is honoured for a template and nothing
// else, so it never opens an ordinary document without its checks.
pub fn starts_new_document(path: &Path, new_window: bool, from_template: bool) -> bool {
    if !crate::file_dialogs::is_template_source(path) {
        return false;
    }
    if from_template {
        return true;
    }
    !new_window
}

// What a launch argument becomes in the queue.
pub fn launch_file(path: PathBuf, args: &crate::launch_args::LaunchArgs) -> PendingFile {
    let new_document = starts_new_document(&path, args.new_window, args.from_template);
    PendingFile { path, new_document }
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
                // The Finder hands over what the user opened there.
                let new_document = starts_new_document(&path, false, false);
                pending.push(PendingFile { path, new_document });
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
    use crate::launch_args::{FROM_TEMPLATE as FROM_TEMPLATE_ARG, NEW_WINDOW as NEW_WINDOW_ARG};

    fn url(text: &str) -> tauri::Url {
        tauri::Url::parse(text).unwrap()
    }

    fn edited(path: &str) -> PendingFile {
        PendingFile { path: PathBuf::from(path), new_document: false }
    }

    fn taken_path(pending: &PendingOpens) -> Option<PathBuf> {
        pending.take().map(|file| file.path)
    }

    fn launched(args: &[&str]) -> PendingFile {
        let parsed = crate::launch_args::parse_launch_args(args.iter().map(|arg| arg.to_string()));
        let path = PathBuf::from(parsed.document.clone().unwrap());
        launch_file(path, &parsed)
    }

    #[test]
    fn queue_hands_out_paths_in_order_and_once() {
        let pending = PendingOpens::with_launch_file(Some(edited("/a/first.docx")));
        pending.push(edited("/a/second.xlsx"));
        pending.push(edited("/a/third.pptx"));

        assert_eq!(taken_path(&pending), Some(PathBuf::from("/a/first.docx")));
        assert_eq!(taken_path(&pending), Some(PathBuf::from("/a/second.xlsx")));
        assert_eq!(taken_path(&pending), Some(PathBuf::from("/a/third.pptx")));
        assert_eq!(pending.take(), None);
        assert_eq!(pending.take(), None);
    }

    #[test]
    fn queue_without_launch_file_starts_empty() {
        let pending = PendingOpens::with_launch_file(None);
        assert_eq!(pending.take(), None);
    }

    // A double click or a launch argument starts a new document from a
    // template; File > Open and Recent, which open it in another window with
    // --new-window, edit it.
    #[test]
    fn a_template_from_outside_starts_a_new_document_and_one_opened_in_the_app_is_edited() {
        for name in ["/t/Letter.dotx", "/t/Budget.XLTX", "/t/Deck.potx", "/t/Form.dotm", "/t/Odf.ott"] {
            assert!(launched(&[name]).new_document, "{name}");
            assert!(!launched(&[NEW_WINDOW_ARG, name]).new_document, "{name}");
            assert!(launched(&[NEW_WINDOW_ARG, FROM_TEMPLATE_ARG, name]).new_document, "{name}");
        }
    }

    // --from-template never turns an ordinary document into a new one, so it is
    // no way around the checks an open makes.
    #[test]
    fn a_document_is_always_edited_whatever_the_options() {
        for name in ["/d/Report.docx", "/d/Book.xlsx", "/d/Talk.pptx", "/d/Notes.md", "/d/Scan.pdf"] {
            assert!(!launched(&[name]).new_document, "{name}");
            assert!(!launched(&[NEW_WINDOW_ARG, name]).new_document, "{name}");
            assert!(!launched(&[NEW_WINDOW_ARG, FROM_TEMPLATE_ARG, name]).new_document, "{name}");
            assert!(!launched(&[FROM_TEMPLATE_ARG, name]).new_document, "{name}");
        }
    }

    #[test]
    fn a_template_the_finder_hands_over_starts_a_new_document() {
        let pending = PendingOpens::default();
        let urls = vec![url("file:///Users/me/Letter.dotx"), url("file:///Users/me/Report.docx")];
        queue_opened_urls(&pending, urls, |_| {});
        assert_eq!(pending.take().map(|file| file.new_document), Some(true));
        assert_eq!(pending.take().map(|file| file.new_document), Some(false));
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
        let pending = PendingOpens::with_launch_file(Some(edited("/launch.docx")));
        let urls = vec![
            url("file:///Users/me/a.docx"),
            url("https://example.com/secret/b.docx"),
            url("file:///Users/me/c.xlsx"),
        ];
        let mut lines = Vec::new();

        let queued = queue_opened_urls(&pending, urls, |line| lines.push(line.to_string()));

        assert_eq!(queued, 2);
        assert_eq!(taken_path(&pending), Some(PathBuf::from("/launch.docx")));
        assert_eq!(taken_path(&pending), Some(PathBuf::from("/Users/me/a.docx")));
        assert_eq!(taken_path(&pending), Some(PathBuf::from("/Users/me/c.xlsx")));
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
