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

// How far a window opened from another one is moved, right and down, so it
// does not open exactly on top of it. In logical pixels.
pub const NEW_WINDOW_OFFSET: f64 = 32.0;

// A rectangle in physical pixels, as Tauri gives positions, sizes and monitors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    fn contains_point(&self, x: i32, y: i32) -> bool {
        let inside_x = x >= self.x && i64::from(x) < self.right();
        let inside_y = y >= self.y && i64::from(y) < self.bottom();
        inside_x && inside_y
    }
}

// The index of the monitor the window's top-left corner is on, if any.
pub fn monitor_of(window: Area, monitors: &[Area]) -> Option<usize> {
    monitors
        .iter()
        .position(|monitor| monitor.contains_point(window.x, window.y))
}

// The window's new top-left corner, moved `offset` physical pixels right and
// down. None when the moved window would cross the monitor's right or bottom
// edge: it then stays where it was restored.
pub fn offset_position(window: Area, monitor: Area, offset: i32) -> Option<(i32, i32)> {
    let x = window.x.checked_add(offset)?;
    let y = window.y.checked_add(offset)?;
    let moved = Area { x, y, ..window };
    if moved.right() > monitor.right() {
        return None;
    }
    if moved.bottom() > monitor.bottom() {
        return None;
    }
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: i32, y: i32, width: u32, height: u32) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    const FULL_HD: Area = Area {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn a_window_with_room_is_moved_right_and_down() {
        let window = area(100, 80, 1200, 800);
        assert_eq!(offset_position(window, FULL_HD, 32), Some((132, 112)));
    }

    #[test]
    fn a_window_that_would_cross_the_right_edge_stays() {
        let window = area(700, 80, 1200, 800);
        assert_eq!(offset_position(window, FULL_HD, 32), None);
    }

    #[test]
    fn a_window_that_would_cross_the_bottom_edge_stays() {
        let window = area(100, 250, 1200, 800);
        assert_eq!(offset_position(window, FULL_HD, 32), None);
    }

    #[test]
    fn a_window_that_ends_exactly_on_the_edge_is_moved() {
        let window = area(688, 248, 1200, 800);
        assert_eq!(offset_position(window, FULL_HD, 32), Some((720, 280)));
    }

    #[test]
    fn a_monitor_at_negative_coordinates_bounds_the_window() {
        let left = area(-1920, 0, 1920, 1080);
        let window = area(-1800, 100, 1200, 800);
        assert_eq!(offset_position(window, left, 32), Some((-1768, 132)));
        let near_its_right_edge = area(-1220, 100, 1200, 800);
        assert_eq!(offset_position(near_its_right_edge, left, 32), None);
    }

    #[test]
    fn a_second_monitor_to_the_right_bounds_the_window() {
        let right = area(1920, 0, 2560, 1440);
        let window = area(3000, 200, 1200, 800);
        assert_eq!(offset_position(window, right, 32), Some((3032, 232)));
        let near_its_right_edge = area(3300, 200, 1200, 800);
        assert_eq!(offset_position(near_its_right_edge, right, 32), None);
    }

    #[test]
    fn the_monitor_is_the_one_under_the_top_left_corner() {
        let left = area(-1920, 0, 1920, 1080);
        let right = area(1920, 0, 2560, 1440);
        let monitors = [left, FULL_HD, right];
        assert_eq!(monitor_of(area(-1800, 100, 1200, 800), &monitors), Some(0));
        assert_eq!(monitor_of(area(100, 100, 1200, 800), &monitors), Some(1));
        assert_eq!(monitor_of(area(3000, 200, 1200, 800), &monitors), Some(2));
        assert_eq!(monitor_of(area(1920, 0, 1200, 800), &monitors), Some(2));
    }

    #[test]
    fn a_window_on_no_monitor_has_no_monitor() {
        let monitors = [FULL_HD];
        assert_eq!(monitor_of(area(5000, 100, 1200, 800), &monitors), None);
        assert_eq!(monitor_of(area(100, -50, 1200, 800), &monitors), None);
    }

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
