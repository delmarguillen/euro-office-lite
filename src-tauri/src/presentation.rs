// Slideshow fullscreen for the main window. The webview has no window write
// permission (core:window:default only grants getters), so sdkjs reaches the
// window through this one command instead.
use std::sync::Mutex;
use std::time::Duration;
use tauri::{PhysicalSize, WebviewWindow};

// The window as it was when the slideshow started.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Saved {
    maximized: bool,
    size: PhysicalSize<u32>,
}

#[derive(Default)]
struct Inner {
    // None while no slideshow runs.
    saved: Option<Saved>,
    // Set when a close arrived during the slideshow: the window is being
    // restored to this and closes once it matches.
    closing: Option<Saved>,
}

#[derive(Default)]
pub struct PresentationState(Mutex<Inner>);

// How long a close waits for the restore before closing anyway.
const CLOSE_RESTORE_TIMEOUT: Duration = Duration::from_secs(2);

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
}

#[tauri::command]
pub fn set_presentation_fullscreen(
    window: WebviewWindow,
    state: tauri::State<'_, PresentationState>,
    on: bool,
) -> Result<(), String> {
    apply_fullscreen(&window, &state, on)
}

// The window is read before the lock and called after it: on Windows a
// window call can send Resized synchronously, and its handler takes the lock.
fn apply_fullscreen<W: PresentationWindow>(
    window: &W,
    state: &PresentationState,
    on: bool,
) -> Result<(), String> {
    let fullscreen = window.is_fullscreen()?;
    let now = window.current()?;
    let mut inner = state.0.lock().unwrap();
    let before = inner.saved;
    match next_step(before, on, fullscreen) {
        Step::Nothing => Ok(()),
        Step::Forget => {
            inner.saved = None;
            Ok(())
        }
        Step::Enter => {
            inner.saved = Some(now);
            drop(inner);
            let result = window.set_fullscreen(true);
            undo_saved_on_error(state, &result, before);
            result
        }
        Step::Exit { maximize } => {
            inner.saved = None;
            drop(inner);
            let result = leave_fullscreen(window, maximize);
            undo_saved_on_error(state, &result, before);
            result
        }
    }
}

// The window call failed: put back the state it was taken with.
fn undo_saved_on_error(
    state: &PresentationState,
    result: &Result<(), String>,
    before: Option<Saved>,
) {
    if result.is_err() {
        state.0.lock().unwrap().saved = before;
    }
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

// Called first on the main window's CloseRequested. Returns true when the
// close was held to restore the window; the caller then does nothing else.
pub fn hold_close_for_restore(window: &WebviewWindow, state: &PresentationState) -> bool {
    if !hold_close(window, state) {
        return false;
    }
    close_after_timeout(window.clone());
    true
}

fn hold_close<W: PresentationWindow>(window: &W, state: &PresentationState) -> bool {
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let Some(target) = take_saved_for_close(state, fullscreen) else {
        return false;
    };
    // closing is set before the restore so a Resized sent during it can close.
    if leave_fullscreen(window, target.maximized).is_err() {
        let mut inner = state.0.lock().unwrap();
        inner.closing = None;
        inner.saved = Some(target);
        return false;
    }
    true
}

// Moves the saved state to closing when the close has to wait for a restore.
fn take_saved_for_close(state: &PresentationState, fullscreen: bool) -> Option<Saved> {
    let mut inner = state.0.lock().unwrap();
    if close_step(inner.saved, fullscreen) == CloseStep::Proceed {
        return None;
    }
    let target = inner.saved.take()?;
    inner.closing = Some(target);
    Some(target)
}

// Called on the main window's Resized: closes a held window once restored.
pub fn close_if_restored(window: &WebviewWindow, state: &PresentationState) {
    close_when_restored(window, state);
}

fn close_when_restored<W: PresentationWindow>(window: &W, state: &PresentationState) {
    let Some(target) = state.0.lock().unwrap().closing else {
        return;
    };
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let Ok(now) = window.current() else {
        return;
    };
    if !is_restored(target, now, fullscreen) {
        return;
    }
    finish_close(window, state);
}

fn finish_close<W: PresentationWindow>(window: &W, state: &PresentationState) {
    let held = state.0.lock().unwrap().closing.take();
    if held.is_some() {
        window.close();
    }
}

// A window manager that ignores the restore (or a size it rounds) must not
// leave the window open for good.
fn close_after_timeout(window: WebviewWindow) {
    std::thread::spawn(move || {
        std::thread::sleep(CLOSE_RESTORE_TIMEOUT);
        use tauri::Manager;
        let state = window.state::<PresentationState>();
        finish_close(&window, &state);
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

    const MONITOR: PhysicalSize<u32> = PhysicalSize::new(1920, 1080);

    // A window that raises Resized from inside set_fullscreen and maximize, as
    // Windows does when SetWindowPlacement and ShowWindow send WM_SIZE.
    struct FakeWindow<'a> {
        state: &'a PresentationState,
        normal: Saved,
        now: Cell<Saved>,
        fullscreen: Cell<bool>,
        fail_set_fullscreen: bool,
        resized_while_locked: Cell<bool>,
        closed: Cell<bool>,
    }

    impl<'a> FakeWindow<'a> {
        fn new(state: &'a PresentationState, normal: Saved) -> Self {
            FakeWindow {
                state,
                normal,
                now: Cell::new(normal),
                fullscreen: Cell::new(false),
                fail_set_fullscreen: false,
                resized_while_locked: Cell::new(false),
                closed: Cell::new(false),
            }
        }

        // try_lock so a held lock fails the test instead of hanging it.
        fn resized(&self) {
            match self.state.0.try_lock() {
                Err(TryLockError::WouldBlock) => self.resized_while_locked.set(true),
                Err(TryLockError::Poisoned(_)) => panic!("presentation lock poisoned"),
                Ok(guard) => {
                    drop(guard);
                    close_when_restored(self, self.state);
                }
            }
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
            self.fullscreen.set(on);
            let size = if on { MONITOR } else { self.normal.size };
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
            self.closed.set(true);
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
        assert_eq!(state.0.lock().unwrap().saved, None);
    }

    // The Resized sent from inside the restore closes the window right away.
    #[test]
    fn closing_does_not_hold_the_lock_during_resized() {
        let state = PresentationState::default();
        let window = FakeWindow::new(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        assert!(hold_close(&window, &state));
        assert!(!window.resized_while_locked.get());
        assert!(window.closed.get());
        assert_eq!(state.0.lock().unwrap().closing, None);
    }

    #[test]
    fn closing_maximized_waits_for_the_maximize() {
        let state = PresentationState::default();
        let window = FakeWindow::new(&state, maximized());
        apply_fullscreen(&window, &state, true).unwrap();
        assert!(hold_close(&window, &state));
        assert!(!window.resized_while_locked.get());
        assert!(window.closed.get());
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
        assert_eq!(state.0.lock().unwrap().saved, Some(normal(800, 600)));
    }

    #[test]
    fn a_failed_close_restore_lets_the_close_proceed() {
        let state = PresentationState::default();
        let mut window = FakeWindow::new(&state, normal(800, 600));
        apply_fullscreen(&window, &state, true).unwrap();
        window.fail_set_fullscreen = true;
        assert!(!hold_close(&window, &state));
        let inner = state.0.lock().unwrap();
        assert_eq!(inner.saved, Some(normal(800, 600)));
        assert_eq!(inner.closing, None);
    }

    #[test]
    fn maximized_is_restored_once_maximized() {
        assert!(is_restored(maximized(), maximized(), false));
        assert!(!is_restored(maximized(), normal(1920, 1080), false));
    }
}
