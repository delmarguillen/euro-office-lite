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
}

fn next_step(saved: Option<bool>, on: bool) -> Step {
    match (saved, on) {
        (None, true) => Step::Enter,
        (Some(was_maximized), false) => Step::Exit {
            maximize: was_maximized,
        },
        _ => Step::Nothing,
    }
}

#[tauri::command]
pub fn set_presentation_fullscreen(
    window: WebviewWindow,
    state: tauri::State<'_, PresentationState>,
    on: bool,
) -> Result<(), String> {
    let mut saved = state.0.lock().unwrap();
    match next_step(*saved, on) {
        Step::Nothing => Ok(()),
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
        assert_eq!(next_step(None, true), Step::Enter);
    }

    #[test]
    fn exiting_restores_maximized() {
        assert_eq!(next_step(Some(true), false), Step::Exit { maximize: true });
    }

    #[test]
    fn exiting_restores_normal() {
        assert_eq!(
            next_step(Some(false), false),
            Step::Exit { maximize: false }
        );
    }

    // A second enter must not overwrite the saved state with the fullscreen one.
    #[test]
    fn entering_twice_keeps_the_first_state() {
        assert_eq!(next_step(Some(false), true), Step::Nothing);
    }

    // endReporter can follow SetFullscreen(false).
    #[test]
    fn exiting_without_a_slideshow_does_nothing() {
        assert_eq!(next_step(None, false), Step::Nothing);
    }
}
