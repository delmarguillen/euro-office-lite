// Whether the shared font cache is usable yet. Building it takes seconds (about
// 14 s on a Raspberry Pi), so it runs on its own thread and the window paints
// meanwhile; the frontend asks for this status and waits for "fonts-ready"
// before it opens the editor (bridge.js, _eoWaitForFonts).

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use crate::temp_layout::FontGeneration;

pub const FONTS_READY_EVENT: &str = "fonts-ready";

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FontsStatus {
    Pending,
    Ready,
    Failed,
}

// Only the cache generation this start built or accepted holds a usable
// AllFonts.js. While Pending there is none yet, and after Failed there is
// none. Either way the editor gets the bundled list.
pub fn uses_generated_list(status: FontsStatus) -> bool {
    status == FontsStatus::Ready
}

pub struct FontsState {
    status: Mutex<FontsStatus>,
    settled: Condvar,
    // The cache generation this process reads (temp_layout.rs). Held until the
    // process ends, so no other instance removes it meanwhile.
    generation: OnceLock<FontGeneration>,
}

impl FontsState {
    pub fn pending() -> FontsState {
        FontsState {
            status: Mutex::new(FontsStatus::Pending),
            settled: Condvar::new(),
            generation: OnceLock::new(),
        }
    }

    pub fn status(&self) -> FontsStatus {
        *self.lock()
    }

    // Only the first generation is kept: the editor and the PDF export of one
    // process read the same one.
    pub fn adopt(&self, generation: FontGeneration) {
        let _ = self.generation.set(generation);
    }

    pub fn generation_dir(&self) -> Option<PathBuf> {
        self.generation.get().map(|generation| generation.dir().to_path_buf())
    }

    // The AllFonts.js the editor gets, None while it gets the bundled one.
    pub fn generated_list(&self) -> Option<PathBuf> {
        if !uses_generated_list(self.status()) {
            return None;
        }
        self.generation_dir().map(|dir| dir.join("AllFonts.js"))
    }

    pub fn finish(&self, status: FontsStatus) {
        *self.lock() = status;
        self.settled.notify_all();
    }

    // The status once it is no longer Pending, or Pending after `timeout`.
    pub fn wait_settled(&self, timeout: Duration) -> FontsStatus {
        let guard = self.lock();
        let waited = self
            .settled
            .wait_timeout_while(guard, timeout, |status| *status == FontsStatus::Pending);
        match waited {
            Ok((status, _)) => *status,
            Err(poisoned) => *poisoned.into_inner().0,
        }
    }

    // A panic elsewhere while holding the lock leaves a plain value behind:
    // still readable, and a font problem must never stop the app.
    fn lock(&self) -> MutexGuard<'_, FontsStatus> {
        self.status.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// A warm cache is Ready at once, with no thread. Otherwise `generate` runs on
// a thread of its own and `on_settled` hears how it ended.
pub fn start(
    fonts: Arc<FontsState>,
    cache_ready: bool,
    generate: impl FnOnce() -> bool + Send + 'static,
    on_settled: impl FnOnce(FontsStatus) + Send + 'static,
) -> Option<std::thread::JoinHandle<()>> {
    if cache_ready {
        fonts.finish(FontsStatus::Ready);
        on_settled(FontsStatus::Ready);
        return None;
    }
    let spawned = std::thread::Builder::new()
        .name("font-cache".to_string())
        .spawn(move || {
            let status = if generate() {
                FontsStatus::Ready
            } else {
                FontsStatus::Failed
            };
            fonts.finish(status);
            on_settled(status);
        });
    // Without a thread there is no cache; the editor opens with the bundled
    // font list, as after a failed build.
    spawned.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    fn record() -> (Arc<Mutex<Vec<FontsStatus>>>, impl FnOnce(FontsStatus) + Send + 'static) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        (seen, move |status| sink.lock().unwrap().push(status))
    }

    #[test]
    fn only_a_ready_cache_gives_the_generated_list() {
        assert!(uses_generated_list(FontsStatus::Ready));
        assert!(!uses_generated_list(FontsStatus::Pending));
        assert!(!uses_generated_list(FontsStatus::Failed));
    }

    #[test]
    fn the_font_list_comes_from_the_cache_only_once_it_is_ready() {
        let root = std::env::temp_dir().join(format!("eo-font-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let outcome = crate::temp_layout::ensure_font_cache(
            &root,
            "1-1",
            |dir: &std::path::Path| {
                std::fs::write(dir.join("AllFonts.js"), "the list").unwrap();
                std::fs::write(dir.join("font_selection.bin"), "selection").unwrap();
                true
            },
            Duration::from_secs(5),
            Duration::from_millis(10),
        );
        let crate::temp_layout::FontCache::Generated(generation) = outcome else {
            panic!("not built: {:?}", outcome);
        };
        let fonts = FontsState::pending();
        fonts.adopt(generation);

        assert_eq!(fonts.generated_list(), None);
        fonts.finish(FontsStatus::Failed);
        assert_eq!(fonts.generated_list(), None);
        fonts.finish(FontsStatus::Ready);
        let list = fonts.generated_list().expect("ready with a generation");
        assert_eq!(std::fs::read_to_string(list).unwrap(), "the list");
        drop(fonts);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_warm_cache_is_ready_without_a_thread_or_a_build() {
        let fonts = Arc::new(FontsState::pending());
        let built = Arc::new(AtomicBool::new(false));
        let flag = built.clone();
        let (seen, on_settled) = record();

        let build = move || {
            flag.store(true, Ordering::SeqCst);
            true
        };

        let thread = start(fonts.clone(), true, build, on_settled);

        assert!(thread.is_none());
        assert_eq!(fonts.status(), FontsStatus::Ready);
        assert!(!built.load(Ordering::SeqCst));
        assert_eq!(*seen.lock().unwrap(), vec![FontsStatus::Ready]);
    }

    #[test]
    fn a_cold_cache_is_pending_until_the_build_ends_then_ready() {
        let fonts = Arc::new(FontsState::pending());
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let (seen, on_settled) = record();

        let thread = start(fonts.clone(), false, move || gate.recv().is_ok(), on_settled)
            .expect("a cold cache is built on a thread");

        assert_eq!(fonts.status(), FontsStatus::Pending);
        assert!(seen.lock().unwrap().is_empty());
        release.send(()).unwrap();
        thread.join().unwrap();
        assert_eq!(fonts.status(), FontsStatus::Ready);
        assert_eq!(*seen.lock().unwrap(), vec![FontsStatus::Ready]);
    }

    #[test]
    fn a_failed_build_is_failed() {
        let fonts = Arc::new(FontsState::pending());
        let (seen, on_settled) = record();

        start(fonts.clone(), false, || false, on_settled).unwrap().join().unwrap();

        assert_eq!(fonts.status(), FontsStatus::Failed);
        assert_eq!(*seen.lock().unwrap(), vec![FontsStatus::Failed]);
    }

    #[test]
    fn a_waiter_wakes_when_the_build_ends() {
        let fonts = Arc::new(FontsState::pending());
        let finisher = fonts.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            finisher.finish(FontsStatus::Ready);
        });

        let started = Instant::now();
        let status = fonts.wait_settled(Duration::from_secs(10));

        assert_eq!(status, FontsStatus::Ready);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_waiter_gives_up_after_the_timeout() {
        let fonts = FontsState::pending();

        let started = Instant::now();
        let status = fonts.wait_settled(Duration::from_millis(100));

        assert_eq!(status, FontsStatus::Pending);
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn a_settled_status_is_returned_without_waiting() {
        let fonts = FontsState::pending();
        fonts.finish(FontsStatus::Failed);

        let started = Instant::now();
        let status = fonts.wait_settled(Duration::from_secs(10));

        assert_eq!(status, FontsStatus::Failed);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_status_reaches_the_frontend_in_lowercase() {
        assert_eq!(serde_json::to_string(&FontsStatus::Pending).unwrap(), "\"pending\"");
        assert_eq!(serde_json::to_string(&FontsStatus::Ready).unwrap(), "\"ready\"");
        assert_eq!(serde_json::to_string(&FontsStatus::Failed).unwrap(), "\"failed\"");
    }
}
