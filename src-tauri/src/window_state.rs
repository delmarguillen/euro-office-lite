// Remembers the main window's size, position and maximized state between runs.
use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
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

// What the plugin saved for one window in its state file. x and y are the
// outer position; width and height are the inner size. Physical pixels.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct SavedWindow {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

// The window `label` saved in the plugin's state file, if any. A window saved
// with no size is one the plugin does not restore.
pub fn saved_window(state_file: &str, label: &str) -> Option<SavedWindow> {
    let mut windows: HashMap<String, SavedWindow> = serde_json::from_str(state_file).ok()?;
    let saved = windows.remove(label)?;
    if saved.width == 0 || saved.height == 0 {
        return None;
    }
    Some(saved)
}

// The window `label` saved in the plugin's state file at `state_path`. None
// when the file is missing or unreadable.
pub fn read_saved_window(state_path: &Path, label: &str) -> Option<SavedWindow> {
    let state_file = std::fs::read_to_string(state_path).ok()?;
    saved_window(&state_file, label)
}

// The outer rectangle of a saved window: its inner size plus the frame, the
// difference between the window's outer and inner size.
pub fn saved_outer_area(saved: SavedWindow, frame_width: u32, frame_height: u32) -> Area {
    Area {
        x: saved.x,
        y: saved.y,
        width: saved.width.saturating_add(frame_width),
        height: saved.height.saturating_add(frame_height),
    }
}

// A monitor's rectangle and scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub area: Area,
    pub scale_factor: f64,
}

// Where a window opened from another one is moved, given the geometry it is
// restored to. None when it stays: maximized, on no monitor, or with no room.
pub fn new_window_position(
    window: Area,
    maximized: bool,
    monitors: &[Monitor],
) -> Option<(i32, i32)> {
    if maximized {
        return None;
    }
    let areas: Vec<Area> = monitors.iter().map(|monitor| monitor.area).collect();
    let monitor = monitors[monitor_of(window, &areas)?];
    let offset = (NEW_WINDOW_OFFSET * monitor.scale_factor).round() as i32;
    offset_position(window, monitor.area, offset)
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

    // Inner sizes as the plugin saves them; 28 px is a macOS title bar at 1x.
    const TITLE_BAR: u32 = 28;

    fn full_hd_at(scale_factor: f64) -> Monitor {
        Monitor {
            area: FULL_HD,
            scale_factor,
        }
    }

    fn position_from_saved(state_file: &str) -> Option<(i32, i32)> {
        let saved = saved_window(state_file, MAIN_WINDOW).expect("main is saved");
        let window = saved_outer_area(saved, 0, TITLE_BAR);
        new_window_position(window, saved.maximized, &[full_hd_at(1.0)])
    }

    #[test]
    fn c1_a_saved_window_with_room_is_moved_from_its_saved_position() {
        let state = r#"{"main":{"width":1000,"height":700,"x":100,"y":100,"prev_x":320,"prev_y":68,"maximized":false,"visible":true,"decorated":true,"fullscreen":false}}"#;
        assert_eq!(position_from_saved(state), Some((132, 132)));
    }

    #[test]
    fn c2_a_saved_window_near_the_edge_stays_at_its_saved_position() {
        let state = r#"{"main":{"width":1000,"height":740,"x":900,"y":340,"prev_x":100,"prev_y":100,"maximized":false,"visible":true,"decorated":true,"fullscreen":false}}"#;
        assert_eq!(position_from_saved(state), None);
    }

    #[test]
    fn c3_a_saved_maximized_window_is_not_moved() {
        let state = r#"{"main":{"width":1920,"height":970,"x":0,"y":25,"prev_x":100,"prev_y":100,"maximized":true,"visible":true,"decorated":true,"fullscreen":false}}"#;
        assert_eq!(position_from_saved(state), None);
    }

    #[test]
    fn the_offset_follows_the_monitor_scale_factor() {
        let saved = SavedWindow {
            x: 100,
            y: 100,
            width: 1000,
            height: 700,
            maximized: false,
        };
        let window = saved_outer_area(saved, 0, TITLE_BAR);
        let position = new_window_position(window, false, &[full_hd_at(2.0)]);
        assert_eq!(position, Some((164, 164)));
    }

    #[test]
    fn the_frame_counts_against_the_bottom_edge() {
        let saved = SavedWindow {
            x: 100,
            y: 300,
            width: 1000,
            height: 740,
            maximized: false,
        };
        let monitors = [full_hd_at(1.0)];
        let without_frame = saved_outer_area(saved, 0, 0);
        let with_title_bar = saved_outer_area(saved, 0, TITLE_BAR);
        assert_eq!(
            new_window_position(without_frame, false, &monitors),
            Some((132, 332))
        );
        assert_eq!(new_window_position(with_title_bar, false, &monitors), None);
    }

    #[test]
    fn a_saved_window_on_no_monitor_is_not_moved() {
        let saved = SavedWindow {
            x: 5000,
            y: 100,
            width: 1000,
            height: 700,
            maximized: false,
        };
        let window = saved_outer_area(saved, 0, TITLE_BAR);
        assert_eq!(new_window_position(window, false, &[full_hd_at(1.0)]), None);
    }

    #[test]
    fn a_missing_state_file_has_no_saved_window() {
        let missing = std::env::temp_dir().join("eol-no-such-window-state.json");
        assert_eq!(read_saved_window(&missing, MAIN_WINDOW), None);
    }

    #[test]
    fn a_state_file_is_read_from_disk() {
        let path =
            std::env::temp_dir().join(format!("eol-window-state-{}.json", std::process::id()));
        let state = r#"{"main":{"width":1000,"height":700,"x":100,"y":100,"prev_x":320,"prev_y":68,"maximized":false,"visible":true,"decorated":true,"fullscreen":false}}"#;
        std::fs::write(&path, state).unwrap();
        let saved = read_saved_window(&path, MAIN_WINDOW);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(saved.map(|saved| (saved.x, saved.y)), Some((100, 100)));
    }

    #[test]
    fn a_state_file_without_main_has_no_saved_window() {
        let state = r#"{"other":{"width":1000,"height":700,"x":100,"y":100,"prev_x":0,"prev_y":0,"maximized":false,"visible":true,"decorated":true,"fullscreen":false}}"#;
        assert_eq!(saved_window(state, MAIN_WINDOW), None);
    }

    #[test]
    fn an_unreadable_state_file_has_no_saved_window() {
        assert_eq!(saved_window("", MAIN_WINDOW), None);
        assert_eq!(saved_window("{not json", MAIN_WINDOW), None);
    }

    #[test]
    fn a_window_saved_with_no_size_has_no_saved_window() {
        let state = r#"{"main":{"width":0,"height":0,"x":0,"y":0,"prev_x":0,"prev_y":0,"maximized":false,"visible":true,"decorated":true,"fullscreen":false}}"#;
        assert_eq!(saved_window(state, MAIN_WINDOW), None);
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
