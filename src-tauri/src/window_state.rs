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

// Below this the editors' toolbars stop fitting; a smaller saved size is
// raised to it, unless the monitor itself is smaller.
const MIN_WIDTH: u32 = 640;
const MIN_HEIGHT: u32 = 480;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn contains(&self, x: i32, y: i32) -> bool {
        let right = self.x as i64 + self.width as i64;
        let bottom = self.y as i64 + self.height as i64;
        x >= self.x && (x as i64) < right && y >= self.y && (y as i64) < bottom
    }
}

// The monitor holding the window's centre, else the first one: a window whose
// monitor is gone moves to one that exists.
fn monitor_for(window: Rect, monitors: &[Rect]) -> Option<Rect> {
    let first = monitors.first().copied();
    let centre_x = window.x.saturating_add((window.width / 2) as i32);
    let centre_y = window.y.saturating_add((window.height / 2) as i32);
    monitors.iter().copied().find(|m| m.contains(centre_x, centre_y)).or(first)
}

fn fit(value: u32, minimum: u32, available: u32) -> u32 {
    value.max(minimum).min(available)
}

fn clamp_into(monitor: Rect, x: i32, y: i32, width: u32, height: u32) -> (i32, i32) {
    let max_x = monitor.x.saturating_add(monitor.width.saturating_sub(width) as i32);
    let max_y = monitor.y.saturating_add(monitor.height.saturating_sub(height) as i32);
    (x.clamp(monitor.x, max_x), y.clamp(monitor.y, max_y))
}

// The plugin keeps a saved position if any corner lands on a monitor and the
// saved size as is. This moves and resizes the window so all of it, title bar
// included, sits on one monitor. None when nothing changes.
pub fn fit_to_monitors(window: Rect, monitors: &[Rect]) -> Option<Rect> {
    let monitor = monitor_for(window, monitors)?;
    let width = fit(window.width, MIN_WIDTH, monitor.width);
    let height = fit(window.height, MIN_HEIGHT, monitor.height);
    let (x, y) = clamp_into(monitor, window.x, window.y, width, height);
    let fitted = Rect { x, y, width, height };
    if fitted == window {
        return None;
    }
    Some(fitted)
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

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn upper_monitor_removed_moves_the_whole_window_to_the_one_left() {
        // Saved on a monitor above the remaining one; only its bottom strip
        // overlapped the lower monitor.
        let lower = rect(0, 0, 1920, 1040);
        let window = rect(100, -900, 1200, 950);
        let fitted = fit_to_monitors(window, &[lower]).unwrap();
        assert_eq!(fitted, rect(100, 0, 1200, 950));
    }

    #[test]
    fn tiny_size_is_raised_to_the_minimum() {
        let monitor = rect(0, 0, 1920, 1040);
        let fitted = fit_to_monitors(rect(200, 200, 10, 10), &[monitor]).unwrap();
        assert_eq!(fitted, rect(200, 200, MIN_WIDTH, MIN_HEIGHT));
    }

    #[test]
    fn size_larger_than_the_monitor_is_lowered_to_it() {
        let monitor = rect(0, 0, 1920, 1040);
        let fitted = fit_to_monitors(rect(50, 50, 5000, 3000), &[monitor]).unwrap();
        assert_eq!(fitted, monitor);
    }

    #[test]
    fn well_placed_window_is_left_alone() {
        let monitors = [rect(0, 0, 1920, 1040), rect(1920, 0, 2560, 1400)];
        assert_eq!(fit_to_monitors(rect(2000, 100, 1200, 800), &monitors), None);
    }

    #[test]
    fn no_monitors_changes_nothing() {
        assert_eq!(fit_to_monitors(rect(0, 0, 800, 600), &[]), None);
    }

    #[test]
    fn monitor_smaller_than_the_minimum_wins() {
        let monitor = rect(0, 0, 600, 400);
        let fitted = fit_to_monitors(rect(10, 10, 100, 100), &[monitor]).unwrap();
        assert_eq!(fitted, monitor);
    }

    #[test]
    fn window_on_a_second_monitor_stays_there() {
        let monitors = [rect(0, 0, 1920, 1040), rect(1920, 0, 1280, 1000)];
        let fitted = fit_to_monitors(rect(2700, 500, 900, 700), &monitors).unwrap();
        assert_eq!(fitted, rect(2300, 300, 900, 700));
    }
}
