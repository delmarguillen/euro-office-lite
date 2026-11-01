// Remembers the main window's size, position and maximized state between runs.
use tauri_plugin_window_state::StateFlags;

// Minimized is not a flag the plugin has, and fullscreen is left out: a window
// reopened fullscreen or hidden is one the user cannot find.
// VISIBLE is left out so the plugin never shows the window: main shows it.
pub fn remembered_flags() -> StateFlags {
    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED
}

pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_window_state::Builder::new()
        .with_state_flags(remembered_flags())
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
}
