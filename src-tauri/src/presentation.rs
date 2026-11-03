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

fn apply_fullscreen<W: PresentationWindow>(
    window: &W,
    state: &PresentationState,
    on: bool,
) -> Result<(), String> {
    let mut inner = state.0.lock().unwrap();
    let fullscreen = window.is_fullscreen()?;
    match next_step(inner.saved, on, fullscreen) {
        Step::Nothing => Ok(()),
        Step::Forget => {
            inner.saved = None;
            Ok(())
        }
        Step::Enter => {
            let saved = window.current()?;
            window.set_fullscreen(true)?;
            inner.saved = Some(saved);
            Ok(())
        }
        Step::Exit { maximize } => {
            inner.saved = None;
            leave_fullscreen(window, maximize)
        }
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
    let mut inner = state.0.lock().unwrap();
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    if close_step(inner.saved, fullscreen) == CloseStep::Proceed {
        return false;
    }
    let Some(target) = inner.saved.take() else {
        return false;
    };
    if leave_fullscreen(window, target.maximized).is_err() {
        return false;
    }
    inner.closing = Some(target);
    true
}

// Called on the main window's Resized: closes a held window once restored.
pub fn close_if_restored(window: &WebviewWindow, state: &PresentationState) {
    close_when_restored(window, state);
}

fn close_when_restored<W: PresentationWindow>(window: &W, state: &PresentationState) {
    let inner = state.0.lock().unwrap();
    let Some(target) = inner.closing else {
        return;
    };
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let Ok(now) = window.current() else {
        return;
    };
    if !is_restored(target, now, fullscreen) {
        return;
    }
    drop(inner);
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
    fn maximized_is_restored_once_maximized() {
        assert!(is_restored(maximized(), maximized(), false));
        assert!(!is_restored(maximized(), normal(1920, 1080), false));
    }
}
