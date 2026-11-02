// Remembers the main window's size, position and maximized state between runs.
use tauri_plugin_window_state::StateFlags;

pub const MAIN_WINDOW: &str = "main";

// Minimized is not a flag the plugin has, and fullscreen is left out: a window
// reopened fullscreen or hidden is one the user cannot find.
// VISIBLE is left out so the plugin never shows the window: main shows it.
pub fn remembered_flags() -> StateFlags {
    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED
}

// On Wayland the size the plugin saves includes the window decoration, so each
// restore would grow the window; Wayland also ignores the position. The size is
// still saved there, and a later run on X11 applies it once.
pub fn restored_flags(on_wayland: bool) -> StateFlags {
    if on_wayland {
        return StateFlags::MAXIMIZED;
    }
    remembered_flags()
}

// main is restored from setup, once its display backend is known, so the
// plugin skips its own restore of it.
pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_window_state::Builder::new()
        .with_state_flags(remembered_flags())
        .skip_initial_state(MAIN_WINDOW)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_size_position_and_maximized() {
        let flags = remembered_flags();
        assert!(flags.contains(StateFlags::SIZE));
        assert!(flags.contains(StateFlags::POSITION));
        assert!(flags.contains(StateFlags::MAXIMIZED));
    }

    #[test]
    fn never_restores_fullscreen_visibility_or_decorations() {
        let flags = remembered_flags();
        assert!(!flags.contains(StateFlags::FULLSCREEN));
        assert!(!flags.contains(StateFlags::VISIBLE));
        assert!(!flags.contains(StateFlags::DECORATIONS));
    }

    #[test]
    fn wayland_restores_only_maximized() {
        assert_eq!(restored_flags(true).bits(), StateFlags::MAXIMIZED.bits());
    }

    #[test]
    fn elsewhere_restores_everything_remembered() {
        assert_eq!(restored_flags(false).bits(), remembered_flags().bits());
    }
}
