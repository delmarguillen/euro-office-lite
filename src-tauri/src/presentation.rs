// Slideshow fullscreen for the main window. The webview has no window write
// permission (core:window:default only grants getters), so sdkjs reaches the
// window through this one command instead.
use std::sync::Mutex;
use tauri::WebviewWindow;

// Whether the window was maximized when the slideshow started; None while no
// slideshow runs.
#[derive(Default)]
pub struct PresentationState(Mutex<Option<bool>>);

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
fn next_step(saved: Option<bool>, on: bool, fullscreen: bool) -> Step {
    match (saved, on, fullscreen) {
        (None, true, _) => Step::Enter,
        (Some(_), true, false) => Step::Enter,
        (Some(_), true, true) => Step::Nothing,
        (Some(was_maximized), false, true) => Step::Exit {
            maximize: was_maximized,
        },
        (Some(_), false, false) => Step::Forget,
        (None, false, _) => Step::Nothing,
    }
}

#[tauri::command]
pub fn set_presentation_fullscreen(
    window: WebviewWindow,
    state: tauri::State<'_, PresentationState>,
    on: bool,
) -> Result<(), String> {
    let mut saved = state.0.lock().unwrap();
    let fullscreen = window.is_fullscreen().map_err(|e| e.to_string())?;
    match next_step(*saved, on, fullscreen) {
        Step::Nothing => Ok(()),
        Step::Forget => {
            *saved = None;
            Ok(())
        }
        Step::Enter => {
            let was_maximized = window.is_maximized().map_err(|e| e.to_string())?;
            window.set_fullscreen(true).map_err(|e| e.to_string())?;
            *saved = Some(was_maximized);
            Ok(())
        }
        Step::Exit { maximize } => {
            *saved = None;
            window.set_fullscreen(false).map_err(|e| e.to_string())?;
            if maximize {
                window.maximize().map_err(|e| e.to_string())?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entering_records_the_state() {
        assert_eq!(next_step(None, true, false), Step::Enter);
    }

    #[test]
    fn exiting_restores_maximized() {
        assert_eq!(
            next_step(Some(true), false, true),
            Step::Exit { maximize: true }
        );
    }

    #[test]
    fn exiting_restores_normal() {
        assert_eq!(
            next_step(Some(false), false, true),
            Step::Exit { maximize: false }
        );
    }

    // A second enter must not overwrite the saved state with the fullscreen one.
    #[test]
    fn entering_twice_keeps_the_first_state() {
        assert_eq!(next_step(Some(false), true, true), Step::Nothing);
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
        assert_eq!(next_step(Some(true), true, false), Step::Enter);
        assert_eq!(next_step(Some(false), true, false), Step::Enter);
    }

    #[test]
    fn exiting_after_a_system_exit_leaves_the_window() {
        assert_eq!(next_step(Some(true), false, false), Step::Forget);
        assert_eq!(next_step(Some(false), false, false), Step::Forget);
    }
}
