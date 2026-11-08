// Slideshow fullscreen for the main window. The webview has no window write
// permission (core:window:default only grants getters), so sdkjs reaches the
// window through this one command instead.
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, PhysicalSize, Webview, WebviewWindow};

// The window as it was when the slideshow started.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Saved {
    maximized: bool,
    size: PhysicalSize<u32>,
}

// What to do once the window is restored.
#[derive(Clone, Copy, Debug, PartialEq)]
enum End {
    // Close the window again; the usual close flow (and its unsaved-changes
    // confirmation) runs then.
    Close,
    // The Quit menu item, with nothing modified when it was chosen. The
    // document can change during the restore, so it is checked again.
    Quit,
    // force_close: the user already chose to discard the changes.
    Discard,
}

// What runs once the restore is over.
#[derive(Debug, PartialEq)]
enum Finish {
    Close,
    Confirm,
    Exit,
}

fn finish(end: End, modified: bool) -> Finish {
    match end {
        End::Close => Finish::Close,
        End::Discard => Finish::Exit,
        End::Quit => match crate::app_quit::quit_action(modified) {
            crate::app_quit::QuitAction::Confirm => Finish::Confirm,
            crate::app_quit::QuitAction::Exit => Finish::Exit,
        },
    }
}

// A restore in progress, from leaving fullscreen until is_restored or the
// timeout. `id` tells a timeout which restore it belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Restore {
    target: Saved,
    id: u64,
}

#[derive(Default)]
struct Inner {
    // None while no slideshow runs.
    saved: Option<Saved>,
    restoring: Option<Restore>,
    // What waits for the restore to end.
    pending: Option<End>,
    next_id: u64,
}

impl Inner {
    fn start_restore(&mut self, target: Saved) -> u64 {
        self.next_id += 1;
        self.restoring = Some(Restore {
            target,
            id: self.next_id,
        });
        self.next_id
    }
}

#[derive(Default)]
pub struct PresentationState(Mutex<Inner>);

// How long a restore may take before what waits for it runs anyway.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq)]
enum Step {
    Nothing,
    Enter,
    Exit { maximize: bool },
    // The system already left fullscreen: drop the state, leave the window.
    Forget,
}

// `fullscreen` is the window's current state. The system (macOS green button,
// window manager key) can leave fullscreen without sdkjs knowing, so a saved
// state is only trusted while the window is still fullscreen.
fn next_step(saved: Option<Saved>, on: bool, fullscreen: bool) -> Step {
    match (saved, on, fullscreen) {
        (None, true, _) => Step::Enter,
        (Some(_), true, false) => Step::Enter,
        (Some(_), true, true) => Step::Nothing,
        (Some(saved), false, true) => Step::Exit {
            maximize: saved.maximized,
        },
        (Some(_), false, false) => Step::Forget,
        (None, false, _) => Step::Nothing,
    }
}

// The window calls this module makes. WebviewWindow implements it; tests use a
// fake that can raise Resized from inside a window call.
trait PresentationWindow {
    fn is_fullscreen(&self) -> Result<bool, String>;
    fn current(&self) -> Result<Saved, String>;
    fn set_fullscreen(&self, on: bool) -> Result<(), String>;
    fn maximize(&self) -> Result<(), String>;
    fn close(&self);
    // Only for an exit already confirmed: no changes, or a discard.
    fn exit_confirmed(&self);
    fn is_modified(&self) -> bool;
    // Asks the webview to show the unsaved-changes confirmation.
    fn confirm_close(&self);
}

impl PresentationWindow for WebviewWindow {
    fn is_fullscreen(&self) -> Result<bool, String> {
        WebviewWindow::is_fullscreen(self).map_err(|e| e.to_string())
    }

    fn current(&self) -> Result<Saved, String> {
        let maximized = self.is_maximized().map_err(|e| e.to_string())?;
        let size = self.inner_size().map_err(|e| e.to_string())?;
        Ok(Saved { maximized, size })
    }

    fn set_fullscreen(&self, on: bool) -> Result<(), String> {
        WebviewWindow::set_fullscreen(self, on).map_err(|e| e.to_string())
    }

    fn maximize(&self) -> Result<(), String> {
        WebviewWindow::maximize(self).map_err(|e| e.to_string())
    }

    fn close(&self) {
        let _ = WebviewWindow::close(self);
    }

    fn exit_confirmed(&self) {
        crate::app_quit::exit_confirmed(self.app_handle());
    }

    fn is_modified(&self) -> bool {
        let state = self.state::<crate::file_ops::AppState>();
        let modified = *state.modified.lock().unwrap();
        modified
    }

    fn confirm_close(&self) {
        let _ = self.app_handle().emit("confirm-close", ());
    }
}

#[tauri::command]
pub fn set_presentation_fullscreen(
    window: WebviewWindow,
    state: tauri::State<'_, PresentationState>,
    on: bool,
) -> Result<(), String> {
    let restore = apply_fullscreen(&window, &state, on)?;
    end_restore_after_timeout(&window, restore);
    Ok(())
}

// The window is read before the lock and called after it: on Windows a
// window call can send Resized synchronously, and its handler takes the lock.
// Returns the id of the restore it started, if any.
fn apply_fullscreen<W: PresentationWindow>(
    window: &W,
    state: &PresentationState,
    on: bool,
) -> Result<Option<u64>, String> {
    let fullscreen = window.is_fullscreen()?;
    let now = window.current()?;
    let mut inner = state.0.lock().unwrap();
    // A close or exit already waits for this restore.
    if on && inner.pending.is_some() {
        return Ok(None);
    }
    let before = inner.saved;
    match next_step(before, on, fullscreen) {
        Step::Nothing => Ok(None),
        Step::Forget => {
            inner.saved = None;
            Ok(None)
        }
        Step::Enter => {
            inner.saved = Some(now);
            inner.restoring = None;
            drop(inner);
            let result = window.set_fullscreen(true);
            if result.is_err() {
                state.0.lock().unwrap().saved = before;
            }
            result.map(|_| None)
        }
        Step::Exit { maximize } => {
            let target = inner.saved.take().expect("Exit comes from a saved state");
            let id = inner.start_restore(target);
            drop(inner);
            let result = leave_fullscreen(window, maximize);
            if result.is_err() {
                undo_restore(state, target);
            }
            result.map(|_| Some(id))
        }
    }
}

// The window call failed: the slideshow is still on.
fn undo_restore(state: &PresentationState, target: Saved) {
    let mut inner = state.0.lock().unwrap();
    inner.saved = Some(target);
    inner.restoring = None;
    inner.pending = None;
}

fn leave_fullscreen<W: PresentationWindow>(window: &W, maximize: bool) -> Result<(), String> {
    window.set_fullscreen(false)?;
    if maximize {
        window.maximize()?;
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
enum CloseStep {
    Proceed,
    RestoreFirst,
}

// tauri-plugin-window-state saves size and position on CloseRequested unless
// the window is maximized or minimized; fullscreen is not checked. A close
// during the slideshow is held until the window is back to its saved state.
fn close_step(saved: Option<Saved>, fullscreen: bool) -> CloseStep {
    match (saved, fullscreen) {
        (Some(_), true) => CloseStep::RestoreFirst,
        _ => CloseStep::Proceed,
    }
}

#[derive(Debug, PartialEq)]
enum EndStep {
    // Nothing to restore: go on now.
    Now,
    // A restore is running: wait for it, without starting another.
    Wait,
    // The slideshow is on: leave fullscreen, then go on.
    RestoreFirst,
}

// What a close or exit request does in the current state.
fn end_step(restoring: bool, saved: Option<Saved>, fullscreen: bool) -> EndStep {
    if restoring {
        return EndStep::Wait;
    }
    match close_step(saved, fullscreen) {
        CloseStep::RestoreFirst => EndStep::RestoreFirst,
        CloseStep::Proceed => EndStep::Now,
    }
}

// A discard outranks everything: the user confirmed it. A quit outranks a
// close: it checks for changes again too, then ends the app.
fn merge_end(pending: Option<End>, request: End) -> End {
    match (pending, request) {
        (Some(End::Discard), _) => End::Discard,
        (_, End::Discard) => End::Discard,
        (Some(End::Quit), _) => End::Quit,
        (_, request) => request,
    }
}

// macOS reports fullscreen off as soon as set_fullscreen(false) is called, so
// the size is compared too: the restore is over when it is back.
fn is_restored(target: Saved, now: Saved, fullscreen: bool) -> bool {
    if fullscreen {
        return false;
    }
    if target.maximized {
        return now.maximized;
    }
    !now.maximized && now.size == target.size
}

#[derive(Debug, PartialEq)]
enum Requested {
    // Nothing waits: the caller goes on as without a slideshow.
    Now,
    // The request waits for the restore; the caller does nothing else.
    Held { started: Option<u64> },
}

// Called first on the main window's CloseRequested. Returns true when the
// close is held for a restore; the caller then prevents it and does nothing
// else.
pub fn hold_close_for_restore(window: &WebviewWindow, state: &PresentationState) -> bool {
    match request_end(window, state, End::Close) {
        Requested::Now => false,
        Requested::Held { started } => {
            end_restore_after_timeout(window, started);
            true
        }
    }
}

// Every exit that skips CloseRequested comes through these two, so the
// window-state plugin saves the restored window on RunEvent::Exit.

// The Quit menu item, called only when nothing was modified.
pub fn quit_when_restored(window: &WebviewWindow, state: &PresentationState) {
    exit_when_restored(window, state, End::Quit);
}

// force_close, after the user chose to discard the changes.
pub fn discard_when_restored(window: &WebviewWindow, state: &PresentationState) {
    exit_when_restored(window, state, End::Discard);
}

// Requested::Now means nothing waited, so the caller's check still holds.
fn exit_when_restored(window: &WebviewWindow, state: &PresentationState, end: End) {
    match request_end(window, state, end) {
        Requested::Now => window.exit_confirmed(),
        Requested::Held { started } => end_restore_after_timeout(window, started),
    }
}

fn request_end<W: PresentationWindow>(
    window: &W,
    state: &PresentationState,
    request: End,
) -> Requested {
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let mut inner = state.0.lock().unwrap();
    match end_step(inner.restoring.is_some(), inner.saved, fullscreen) {
        EndStep::Now => Requested::Now,
        EndStep::Wait => {
            inner.pending = Some(merge_end(inner.pending, request));
            Requested::Held { started: None }
        }
        EndStep::RestoreFirst => {
            let target = inner
                .saved
                .take()
                .expect("RestoreFirst comes from a saved state");
            let id = inner.start_restore(target);
            // Set before the restore so a Resized sent during it can end it.
            inner.pending = Some(request);
            drop(inner);
            if leave_fullscreen(window, target.maximized).is_err() {
                undo_restore(state, target);
                return Requested::Now;
            }
            Requested::Held { started: Some(id) }
        }
    }
}

// Called on the main window's Resized: ends a restore once the window is back.
pub fn on_resized(window: &WebviewWindow, state: &PresentationState) {
    end_restore_if_restored(window, state);
}

fn end_restore_if_restored<W: PresentationWindow>(window: &W, state: &PresentationState) {
    let Some(restore) = state.0.lock().unwrap().restoring else {
        return;
    };
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let Ok(now) = window.current() else {
        return;
    };
    if !is_restored(restore.target, now, fullscreen) {
        return;
    }
    end_restore(window, state, restore.id);
}

// Ends the restore `id` if it is still the current one and runs what waited.
// The pending request is taken before it runs, so a close that later shows
// the unsaved-changes confirmation leaves nothing armed if it is cancelled.
// modified is read after the lock is released.
fn end_restore<W: PresentationWindow>(window: &W, state: &PresentationState, id: u64) {
    let pending = {
        let mut inner = state.0.lock().unwrap();
        let Some(restore) = inner.restoring else {
            return;
        };
        if restore.id != id {
            return;
        }
        inner.restoring = None;
        inner.pending.take()
    };
    let Some(end) = pending else {
        return;
    };
    match finish(end, window.is_modified()) {
        Finish::Close => window.close(),
        Finish::Confirm => window.confirm_close(),
        Finish::Exit => window.exit_confirmed(),
    }
}

// A window manager that ignores the restore (or a size it rounds) must not
// leave the window open, or the app running, for good.
fn end_restore_after_timeout(window: &WebviewWindow, id: Option<u64>) {
    let Some(id) = id else {
        return;
    };
    let window = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(RESTORE_TIMEOUT);
        let state = window.state::<PresentationState>();
        end_restore(&window, &state, id);
    });
}

// Keyboard focus after native fullscreen (macOS). Leaving fullscreen, tao
// restores the style mask in a block queued on the main queue, and that block
// makes tao's own view first responder. wry moved that view out of the window,
// so AppKit refuses and the window is left with no first responder: keys reach
// nothing until a click. Once the resizes of the exit settle, the webview is
// made first responder again. Any exit counts: slideshow, green button, keys.

// Resized events up to this long after leaving fullscreen belong to the exit.
const EXIT_RESIZES: Duration = Duration::from_secs(2);
// Long enough for tao's queued block to run before the focus.
const FOCUS_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Windowed,
    Fullscreen,
    Leaving { since: Instant },
}

// What a Resized does: the next phase, and whether to focus the webview once
// the resizes settle.
fn phase_step(phase: Phase, fullscreen: bool, now: Instant) -> (Phase, bool) {
    if fullscreen {
        return (Phase::Fullscreen, false);
    }
    match phase {
        Phase::Windowed => (Phase::Windowed, false),
        Phase::Fullscreen => (Phase::Leaving { since: now }, true),
        Phase::Leaving { since } if now.duration_since(since) < EXIT_RESIZES => (phase, true),
        Phase::Leaving { .. } => (Phase::Windowed, false),
    }
}

struct Refocus {
    phase: Phase,
    // The focus most recently asked for; an older one gives way to it.
    latest: u64,
}

pub struct FullscreenFocus(Mutex<Refocus>);

impl Default for FullscreenFocus {
    fn default() -> Self {
        FullscreenFocus(Mutex::new(Refocus {
            phase: Phase::Windowed,
            latest: 0,
        }))
    }
}

impl FullscreenFocus {
    // Returns the ticket of a focus to run after FOCUS_DELAY, if any.
    fn on_resized(&self, fullscreen: bool, now: Instant) -> Option<u64> {
        let mut inner = self.0.lock().unwrap();
        let (phase, focus) = phase_step(inner.phase, fullscreen, now);
        inner.phase = phase;
        if !focus {
            return None;
        }
        inner.latest += 1;
        Some(inner.latest)
    }

    fn is_latest(&self, ticket: u64) -> bool {
        self.0.lock().unwrap().latest == ticket
    }
}

// Called on the main window's Resized, on macOS only.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn refocus_after_fullscreen(window: &WebviewWindow, focus: &FullscreenFocus) {
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let Some(ticket) = focus.on_resized(fullscreen, Instant::now()) else {
        return;
    };
    let window = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FOCUS_DELAY);
        if !window.state::<FullscreenFocus>().is_latest(ticket) {
            return;
        }
        let webview: &Webview = window.as_ref();
        if let Err(e) = webview.set_focus() {
            eprintln!("[presentation] focusing the webview failed: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::TryLockError;

    fn normal(width: u32, height: u32) -> Saved {
        Saved {
            maximized: false,
            size: PhysicalSize::new(width, height),
        }
    }

    fn maximized() -> Saved {
        Saved {
            maximized: true,
            size: PhysicalSize::new(1920, 1040),
        }
    }

    #[test]
    fn entering_records_the_state() {
        assert_eq!(next_step(None, true, false), Step::Enter);
    }

    #[test]
    fn exiting_restores_maximized() {
        assert_eq!(
            next_step(Some(maximized()), false, true),
            Step::Exit { maximize: true }
        );
    }

    #[test]
    fn exiting_restores_normal() {
        assert_eq!(
            next_step(Some(normal(800, 600)), false, true),
            Step::Exit { maximize: false }
        );
    }

    // A second enter must not overwrite the saved state with the fullscreen one.
    #[test]
    fn entering_twice_keeps_the_first_state() {
        assert_eq!(next_step(Some(normal(800, 600)), true, true), Step::Nothing);
    }

    // endReporter can follow SetFullscreen(false).
    #[test]
    fn exiting_without_a_slideshow_does_nothing() {
        assert_eq!(next_step(None, false, false), Step::Nothing);
    }

    // The user went fullscreen by the system before the slideshow.
    #[test]
    fn entering_while_already_fullscreen_enters() {
        assert_eq!(next_step(None, true, true), Step::Enter);
    }

    #[test]
    fn exiting_without_a_slideshow_in_system_fullscreen_does_nothing() {
        assert_eq!(next_step(None, false, true), Step::Nothing);
    }

    #[test]
    fn entering_after_a_system_exit_enters_again() {
        assert_eq!(next_step(Some(maximized()), true, false), Step::Enter);
        assert_eq!(next_step(Some(normal(800, 600)), true, false), Step::Enter);
    }

    #[test]
    fn exiting_after_a_system_exit_leaves_the_window() {
        assert_eq!(next_step(Some(maximized()), false, false), Step::Forget);
        assert_eq!(
            next_step(Some(normal(800, 600)), false, false),
            Step::Forget
        );
    }

    #[test]
    fn closing_in_the_slideshow_restores_first() {
        assert_eq!(
            close_step(Some(normal(800, 600)), true),
            CloseStep::RestoreFirst
        );
    }

    #[test]
    fn closing_without_a_slideshow_proceeds() {
        assert_eq!(close_step(None, false), CloseStep::Proceed);
        assert_eq!(close_step(None, true), CloseStep::Proceed);
    }

    // The system left fullscreen: the window already has its own size.
    #[test]
    fn closing_after_a_system_exit_proceeds() {
        assert_eq!(
            close_step(Some(normal(800, 600)), false),
            CloseStep::Proceed
        );
    }

    #[test]
    fn still_fullscreen_is_not_restored() {
        let target = normal(800, 600);
        assert!(!is_restored(target, target, true));
    }

    #[test]
    fn normal_is_restored_at_its_size() {
        let target = normal(800, 600);
        assert!(is_restored(target, normal(800, 600), false));
    }

    // macOS mid-animation: fullscreen already off, size still the monitor's.
    #[test]
    fn normal_at_another_size_is_not_restored() {
        assert!(!is_restored(normal(800, 600), normal(1920, 1080), false));
    }

    #[test]
    fn normal_target_but_maximized_is_not_restored() {
        let mut now = normal(800, 600);
        now.maximized = true;
        assert!(!is_restored(normal(800, 600), now, false));
    }

    #[test]
    fn a_request_without_a_slideshow_goes_on() {
        assert_eq!(end_step(false, None, false), EndStep::Now);
        assert_eq!(end_step(false, None, true), EndStep::Now);
    }

    #[test]
    fn a_request_in_the_slideshow_restores_first() {
        assert_eq!(
            end_step(false, Some(normal(800, 600)), true),
            EndStep::RestoreFirst
        );
    }

    // saved is already gone once a restore runs.
    #[test]
    fn a_request_during_a_restore_waits() {
        assert_eq!(end_step(true, None, false), EndStep::Wait);
        assert_eq!(end_step(true, None, true), EndStep::Wait);
    }

    #[test]
    fn a_quit_outranks_a_close() {
        assert_eq!(merge_end(None, End::Close), End::Close);
        assert_eq!(merge_end(Some(End::Close), End::Quit), End::Quit);
        assert_eq!(merge_end(Some(End::Quit), End::Close), End::Quit);
    }

    #[test]
    fn a_discard_outranks_a_quit_and_a_close() {
        assert_eq!(merge_end(Some(End::Quit), End::Discard), End::Discard);
        assert_eq!(merge_end(Some(End::Close), End::Discard), End::Discard);
        assert_eq!(merge_end(Some(End::Discard), End::Quit), End::Discard);
        assert_eq!(merge_end(Some(End::Discard), End::Close), End::Discard);
    }

    #[test]
    fn a_quit_asks_if_the_document_changed() {
        assert_eq!(finish(End::Quit, true), Finish::Confirm);
        assert_eq!(finish(End::Quit, false), Finish::Exit);
    }

    #[test]
    fn a_discard_never_asks() {
        assert_eq!(finish(End::Discard, true), Finish::Exit);
        assert_eq!(finish(End::Discard, false), Finish::Exit);
    }

    // main.rs asks then, from CloseRequested.
    #[test]
    fn a_close_closes() {
        assert_eq!(finish(End::Close, true), Finish::Close);
        assert_eq!(finish(End::Close, false), Finish::Close);
    }

    const MONITOR: PhysicalSize<u32> = PhysicalSize::new(1920, 1080);

    // A window that raises Resized from inside set_fullscreen and maximize, as
    // Windows does when SetWindowPlacement and ShowWindow send WM_SIZE. With
    // `slow`, leaving fullscreen keeps the monitor size until `finish_exit`,
    // as the macOS animation does.
    struct FakeWindow<'a> {
        state: &'a PresentationState,
        normal: Saved,
        now: Cell<Saved>,
        fullscreen: Cell<bool>,
        fail_set_fullscreen: bool,
        slow: bool,
        resized_while_locked: Cell<bool>,
        closes: Cell<u32>,
        exits: Cell<u32>,
        confirms: Cell<u32>,
        modified: Cell<bool>,
        set_fullscreen_calls: Cell<u32>,
    }

    impl<'a> FakeWindow<'a> {
        fn new(state: &'a PresentationState, normal: Saved) -> Self {
            FakeWindow {
                state,
                normal,
                now: Cell::new(normal),
                fullscreen: Cell::new(false),
                fail_set_fullscreen: false,
                slow: false,
                resized_while_locked: Cell::new(false),
                closes: Cell::new(0),
                exits: Cell::new(0),
                confirms: Cell::new(0),
                modified: Cell::new(false),
                set_fullscreen_calls: Cell::new(0),
            }
        }

        fn slow(state: &'a PresentationState, normal: Saved) -> Self {
            let mut window = FakeWindow::new(state, normal);
            window.slow = true;
            window
        }

        // try_lock so a held lock fails the test instead of hanging it.
        fn resized(&self) {
            match self.state.0.try_lock() {
                Err(TryLockError::WouldBlock) => self.resized_while_locked.set(true),
                Err(TryLockError::Poisoned(_)) => panic!("presentation lock poisoned"),
                Ok(guard) => {
                    drop(guard);
                    end_restore_if_restored(self, self.state);
                }
            }
        }

        // The end of the macOS exit animation.
        fn finish_exit(&self) {
            self.now.set(self.normal);
            self.resized();
        }

        // Mirrors main.rs: a CloseRequested that is not held closes the window.
        fn close_requested(&self) {
            if request_end(self, self.state, End::Close) == Requested::Now {
                self.close();
            }
        }

        fn current_restore(&self) -> u64 {
            self.state.0.lock().unwrap().restoring.unwrap().id
        }
    }

    impl PresentationWindow for FakeWindow<'_> {
        fn is_fullscreen(&self) -> Result<bool, String> {
            Ok(self.fullscreen.get())
        }

        fn current(&self) -> Result<Saved, String> {
            Ok(self.now.get())
        }

        fn set_fullscreen(&self, on: bool) -> Result<(), String> {
            if self.fail_set_fullscreen {
                return Err("set_fullscreen failed".into());
            }
            self.set_fullscreen_calls
                .set(self.set_fullscreen_calls.get() + 1);
            self.fullscreen.set(on);
            let size = if on || self.slow {
                MONITOR
            } else {
                self.normal.size
            };
            self.now.set(Saved {
                maximized: false,
                size,
            });
            self.resized();
            Ok(())
        }

        fn maximize(&self) -> Result<(), String> {
            self.now.set(Saved {
                maximized: true,
                size: MONITOR,
            });
            self.resized();
            Ok(())
        }

        fn close(&self) {
            self.closes.set(self.closes.get() + 1);
        }

        fn exit_confirmed(&self) {
            self.exits.set(self.exits.get() + 1);
        }

        fn is_modified(&self) -> bool {
            self.modified.get()
        }

        fn confirm_close(&self) {
            self.confirms.set(self.confirms.get() + 1);
        }
    }

    #[test]
    fn entering_does_not_hold_the_lock_during_resized() {
        let state = PresentationState::default();
        let window = FakeWindow::new(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        assert!(!window.resized_while_locked.get());
        assert_eq!(state.0.lock().unwrap().saved, Some(normal(800, 600)));
    }

    #[test]
    fn exiting_does_not_hold_the_lock_during_resized() {
        let state = PresentationState::default();
        let window = FakeWindow::new(&state, maximized());
        apply_fullscreen(&window, &state, true).unwrap();
        apply_fullscreen(&window, &state, false).unwrap();
        assert!(!window.resized_while_locked.get());
        assert!(window.now.get().maximized);
        let inner = state.0.lock().unwrap();
        assert_eq!(inner.saved, None);
        assert_eq!(inner.restoring, None);
    }

    // The Resized sent from inside the restore closes the window right away.
    #[test]
    fn closing_does_not_hold_the_lock_during_resized() {
        let state = PresentationState::default();
        let window = FakeWindow::new(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.close_requested();
        assert!(!window.resized_while_locked.get());
        assert_eq!(window.closes.get(), 1);
        assert_eq!(state.0.lock().unwrap().restoring, None);
    }

    #[test]
    fn closing_maximized_waits_for_the_maximize() {
        let state = PresentationState::default();
        let window = FakeWindow::new(&state, maximized());
        apply_fullscreen(&window, &state, true).unwrap();
        window.close_requested();
        assert!(!window.resized_while_locked.get());
        assert_eq!(window.closes.get(), 1);
    }

    #[test]
    fn a_failed_enter_leaves_no_saved_state() {
        let state = PresentationState::default();
        let mut window = FakeWindow::new(&state, normal(800, 600));
        window.fail_set_fullscreen = true;
        assert!(apply_fullscreen(&window, &state, true).is_err());
        assert_eq!(state.0.lock().unwrap().saved, None);
    }

    #[test]
    fn a_failed_exit_keeps_the_saved_state() {
        let state = PresentationState::default();
        let mut window = FakeWindow::new(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.fail_set_fullscreen = true;
        assert!(apply_fullscreen(&window, &state, false).is_err());
        let inner = state.0.lock().unwrap();
        assert_eq!(inner.saved, Some(normal(800, 600)));
        assert_eq!(inner.restoring, None);
    }

    #[test]
    fn a_failed_close_restore_lets_the_close_proceed() {
        let state = PresentationState::default();
        let mut window = FakeWindow::new(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.fail_set_fullscreen = true;
        assert_eq!(request_end(&window, &state, End::Close), Requested::Now);
        let inner = state.0.lock().unwrap();
        assert_eq!(inner.saved, Some(normal(800, 600)));
        assert_eq!(inner.restoring, None);
        assert_eq!(inner.pending, None);
    }

    #[test]
    fn two_closes_during_a_slow_restore_close_once_restored() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.close_requested();
        window.close_requested();
        assert_eq!(window.closes.get(), 0);
        // The second close does not restart the restore.
        assert_eq!(window.set_fullscreen_calls.get(), 2);
        window.finish_exit();
        assert_eq!(window.closes.get(), 1);
    }

    // The second close does not move the timeout either: it still ends the
    // first restore.
    #[test]
    fn a_second_close_keeps_the_first_timeout() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.close_requested();
        let id = window.current_restore();
        window.close_requested();
        end_restore(&window, &state, id);
        assert_eq!(window.closes.get(), 1);
    }

    // After the restore the close runs the usual flow; a confirmation cancelled
    // then must find nothing left to close later.
    #[test]
    fn a_cancelled_confirmation_leaves_no_close_armed() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.close_requested();
        let id = window.current_restore();
        window.finish_exit();
        assert_eq!(window.closes.get(), 1);
        // The re-sent close is not held: main.rs shows the confirmation.
        assert_eq!(request_end(&window, &state, End::Close), Requested::Now);
        // The user cancels. A late Resized or timeout does nothing.
        window.resized();
        end_restore(&window, &state, id);
        assert_eq!(window.closes.get(), 1);
        assert_eq!(state.0.lock().unwrap().pending, None);
    }

    #[test]
    fn closing_right_after_the_slideshow_waits_for_the_exit() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        apply_fullscreen(&window, &state, false).unwrap();
        // endReporter follows SetFullscreen(false).
        apply_fullscreen(&window, &state, false).unwrap();
        window.close_requested();
        assert_eq!(window.closes.get(), 0);
        window.finish_exit();
        assert_eq!(window.closes.get(), 1);
    }

    #[test]
    fn a_restore_without_a_close_ends_quietly() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        let id = apply_fullscreen(&window, &state, false).unwrap().unwrap();
        end_restore(&window, &state, id);
        assert_eq!(window.closes.get(), 0);
        assert_eq!(state.0.lock().unwrap().restoring, None);
    }

    // A timeout from an earlier restore must not end a later one.
    #[test]
    fn an_old_timeout_does_not_end_a_new_restore() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        let old = apply_fullscreen(&window, &state, false).unwrap().unwrap();
        window.finish_exit();
        apply_fullscreen(&window, &state, true).unwrap();
        window.close_requested();
        end_restore(&window, &state, old);
        assert_eq!(window.closes.get(), 0);
        window.finish_exit();
        assert_eq!(window.closes.get(), 1);
    }

    #[test]
    fn quitting_in_the_slideshow_exits_once_restored() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        assert_ne!(request_end(&window, &state, End::Quit), Requested::Now);
        assert_eq!(window.exits.get(), 0);
        window.finish_exit();
        assert_eq!(window.exits.get(), 1);
        assert_eq!(window.closes.get(), 0);
    }

    // Typing during the macOS exit animation, after an unmodified Quit.
    #[test]
    fn a_quit_asks_when_the_document_changes_during_the_restore() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        assert_ne!(request_end(&window, &state, End::Quit), Requested::Now);
        window.modified.set(true);
        window.finish_exit();
        assert_eq!(window.exits.get(), 0);
        assert_eq!(window.confirms.get(), 1);
        assert_eq!(state.0.lock().unwrap().pending, None);
    }

    #[test]
    fn a_quit_still_unmodified_exits_once_restored() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        request_end(&window, &state, End::Quit);
        window.finish_exit();
        assert_eq!(window.exits.get(), 1);
        assert_eq!(window.confirms.get(), 0);
    }

    #[test]
    fn a_discard_during_a_held_quit_exits_without_asking() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        request_end(&window, &state, End::Quit);
        window.modified.set(true);
        request_end(&window, &state, End::Discard);
        window.finish_exit();
        assert_eq!(window.exits.get(), 1);
        assert_eq!(window.confirms.get(), 0);
    }

    // force_close after the confirmation, while a held close restores.
    #[test]
    fn discarding_during_a_held_close_exits_once_restored() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.close_requested();
        assert_ne!(request_end(&window, &state, End::Discard), Requested::Now);
        window.close_requested();
        window.finish_exit();
        assert_eq!(window.exits.get(), 1);
        assert_eq!(window.closes.get(), 0);
    }

    #[test]
    fn quitting_without_a_slideshow_exits_now() {
        let state = PresentationState::default();
        let window = FakeWindow::new(&state, normal(800, 600));
        assert_eq!(request_end(&window, &state, End::Quit), Requested::Now);
    }

    #[test]
    fn entering_while_a_close_waits_is_ignored() {
        let state = PresentationState::default();
        let window = FakeWindow::slow(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.close_requested();
        apply_fullscreen(&window, &state, true).unwrap();
        assert_eq!(state.0.lock().unwrap().saved, None);
        window.finish_exit();
        assert_eq!(window.closes.get(), 1);
    }

    #[test]
    fn maximized_is_restored_once_maximized() {
        assert!(is_restored(maximized(), maximized(), false));
        assert!(!is_restored(maximized(), normal(1920, 1080), false));
    }

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn resizing_a_window_never_in_fullscreen_does_not_focus() {
        let focus = FullscreenFocus::default();
        let start = Instant::now();
        assert_eq!(focus.on_resized(false, start), None);
        assert_eq!(focus.on_resized(false, after(start, 50)), None);
    }

    #[test]
    fn resizing_in_fullscreen_does_not_focus() {
        let focus = FullscreenFocus::default();
        let start = Instant::now();
        assert_eq!(focus.on_resized(true, start), None);
        assert_eq!(focus.on_resized(true, after(start, 50)), None);
    }

    // Green button or keys: the window reports fullscreen until tao's
    // windowDidExitFullScreen, whose Resized is the first one without it.
    #[test]
    fn a_system_exit_focuses_after_its_last_resize() {
        let focus = FullscreenFocus::default();
        let start = Instant::now();
        focus.on_resized(true, start);
        assert_eq!(focus.on_resized(true, after(start, 100)), None);
        let ticket = focus.on_resized(false, after(start, 700)).unwrap();
        assert!(focus.is_latest(ticket));
    }

    // set_fullscreen(false) turns fullscreen off at once, so every resize of
    // the animation asks; only the last one, after tao's restore, focuses.
    #[test]
    fn a_slideshow_exit_focuses_once_its_resizes_settle() {
        let focus = FullscreenFocus::default();
        let start = Instant::now();
        focus.on_resized(true, start);
        let first = focus.on_resized(false, after(start, 10)).unwrap();
        let middle = focus.on_resized(false, after(start, 300)).unwrap();
        let last = focus.on_resized(false, after(start, 800)).unwrap();
        assert!(!focus.is_latest(first));
        assert!(!focus.is_latest(middle));
        assert!(focus.is_latest(last));
    }

    // A maximized slideshow window is maximized again after the exit.
    #[test]
    fn the_maximize_after_an_exit_still_focuses() {
        let focus = FullscreenFocus::default();
        let start = Instant::now();
        focus.on_resized(true, start);
        focus.on_resized(false, after(start, 10));
        assert!(focus.on_resized(false, after(start, 1200)).is_some());
    }

    #[test]
    fn a_resize_long_after_the_exit_does_not_focus() {
        let focus = FullscreenFocus::default();
        let start = Instant::now();
        focus.on_resized(true, start);
        focus.on_resized(false, after(start, 10));
        assert_eq!(focus.on_resized(false, after(start, 2500)), None);
        assert_eq!(focus.on_resized(false, after(start, 2600)), None);
    }

    #[test]
    fn every_exit_focuses_again() {
        let focus = FullscreenFocus::default();
        let start = Instant::now();
        focus.on_resized(true, start);
        focus.on_resized(false, after(start, 10));
        focus.on_resized(false, after(start, 5000));
        focus.on_resized(true, after(start, 6000));
        assert!(focus.on_resized(false, after(start, 6500)).is_some());
    }
}
