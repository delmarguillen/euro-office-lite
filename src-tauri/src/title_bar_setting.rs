// The "Title bar" choice in Configuración: the app's own bar (the GTK header
// bar tao adds on Wayland, the default), the system's, or none. Only Linux
// with GDK's Wayland backend has it, as tao adds its bar nowhere else.
//
// The choice is a small file in the app's config folder, next to
// dmabuf-renderer.json. It is read before the window is built, because GTK
// settles the decorations when it realizes the window. The command line and
// EURO_OFFICE_LITE_TITLE_BAR win over it (launch_args.rs). The module compiles
// on every platform so the commands can be registered everywhere; only Linux
// reads or writes the file.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::launch_args::{TitleBar, TitleBarSource};

// Created in the app's config folder, next to dmabuf-renderer.json.
const SETTING_FILE: &str = "title-bar.json";

// A colon-separated list of desktop names, such as "ubuntu:GNOME".
const DESKTOP_ENV: &str = "XDG_CURRENT_DESKTOP";

// The choice in Configuración. "system" and "none" are the words
// EURO_OFFICE_LITE_TITLE_BAR takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Setting {
    #[default]
    App,
    System,
    #[serde(rename = "none")]
    Undecorated,
}

impl Setting {
    pub fn mode(self) -> TitleBar {
        match self {
            Setting::App => TitleBar::Default,
            Setting::System => TitleBar::System,
            Setting::Undecorated => TitleBar::Undecorated,
        }
    }
}

// The setting read from the file, and the problem when the default was used
// instead: "unreadable", "corrupt" or "no-folder". A missing file is not a
// problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub setting: Setting,
    pub problem: Option<&'static str>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSetting {
    title_bar: Setting,
}

fn app_bar(problem: Option<&'static str>) -> Loaded {
    Loaded {
        setting: Setting::App,
        problem,
    }
}

fn load_setting_from(path: &Path) -> Loaded {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return app_bar(None),
        Err(_) => return app_bar(Some("unreadable")),
    };
    let Ok(stored) = serde_json::from_str::<StoredSetting>(&raw) else {
        return app_bar(Some("corrupt"));
    };
    Loaded {
        setting: stored.title_bar,
        problem: None,
    }
}

fn save_setting_to(path: &Path, setting: Setting) -> Result<(), String> {
    let stored = StoredSetting { title_bar: setting };
    let json = serde_json::to_string_pretty(&stored).map_err(|error| error.to_string())?;
    crate::dmabuf_default::write_setting_file(path, &json)
}

fn settings_path(identifier: &str) -> Option<PathBuf> {
    let dir = crate::dmabuf_default::config_dir(identifier)?;
    Some(dir.join(SETTING_FILE))
}

// The saved choice, read at startup while the window does not exist yet.
pub fn load(identifier: &str) -> Loaded {
    if !cfg!(target_os = "linux") {
        return app_bar(None);
    }
    let Some(path) = settings_path(identifier) else {
        return app_bar(Some("no-folder"));
    };
    load_setting_from(&path)
}

// What this start found, kept for the settings screen.
pub struct Startup {
    // GDK's backend is Wayland.
    pub wayland: bool,
    // What decided this start's mode.
    pub source: TitleBarSource,
}

fn supported(startup: &Startup) -> bool {
    cfg!(target_os = "linux") && startup.wayland
}

// With the system's bar on GNOME the window may not move when dragged by it
// (issue #47), so the settings screen says so next to that choice.
pub fn gnome_drag_note(desktop: Option<&str>, setting: Setting) -> bool {
    let Some(desktop) = desktop else {
        return false;
    };
    setting == Setting::System && desktop.contains("GNOME")
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingState {
    // Only Linux on Wayland has the setting; the settings screen hides it
    // elsewhere.
    supported: bool,
    // The saved choice, as the next start will read it.
    setting: Setting,
    // The command line or the environment variable chose this start's mode,
    // and will again the next time: they win over the setting.
    overridden: bool,
    // Show the note about dragging the window on GNOME.
    gnome_note: bool,
}

fn setting_state(setting: Setting, startup: &Startup, desktop: Option<&str>) -> SettingState {
    SettingState {
        supported: supported(startup),
        setting,
        overridden: startup.source.overrides_setting(),
        gnome_note: gnome_drag_note(desktop, setting),
    }
}

fn desktop_name() -> Option<String> {
    std::env::var(DESKTOP_ENV).ok()
}

fn saved_setting(app: &AppHandle, startup: &Startup) -> Setting {
    if !supported(startup) {
        return Setting::App;
    }
    let Some(path) = settings_path(&app.config().identifier) else {
        return Setting::App;
    };
    load_setting_from(&path).setting
}

#[tauri::command]
pub fn title_bar_setting_state(app: AppHandle, startup: State<'_, Startup>) -> SettingState {
    let setting = saved_setting(&app, &startup);
    setting_state(setting, &startup, desktop_name().as_deref())
}

// Answers the state after the change, so the notes follow the new choice.
#[tauri::command]
pub fn set_title_bar_setting(
    app: AppHandle,
    startup: State<'_, Startup>,
    setting: Setting,
) -> Result<SettingState, String> {
    if !supported(&startup) {
        return Err("not-supported".to_string());
    }
    let path = settings_path(&app.config().identifier).ok_or("no-folder")?;
    save_setting_to(&path, setting)?;
    Ok(setting_state(setting, &startup, desktop_name().as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eol-title-bar-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn started(wayland: bool, source: TitleBarSource) -> Startup {
        Startup { wayland, source }
    }

    #[test]
    fn each_choice_maps_to_its_mode() {
        assert_eq!(Setting::App.mode(), TitleBar::Default);
        assert_eq!(Setting::System.mode(), TitleBar::System);
        assert_eq!(Setting::Undecorated.mode(), TitleBar::Undecorated);
    }

    #[test]
    fn missing_file_is_the_app_bar_without_a_problem() {
        let dir = scratch_dir("missing");
        assert_eq!(load_setting_from(&dir.join(SETTING_FILE)), app_bar(None));
    }

    #[test]
    fn each_saved_value_is_read_back() {
        let dir = scratch_dir("round-trip");
        let path = dir.join(SETTING_FILE);
        for setting in [Setting::System, Setting::Undecorated, Setting::App] {
            save_setting_to(&path, setting).unwrap();
            assert_eq!(
                load_setting_from(&path),
                Loaded {
                    setting,
                    problem: None
                }
            );
        }
    }

    #[test]
    fn saving_creates_the_folder_and_leaves_no_temporary_file() {
        let dir = scratch_dir("new-folder");
        let path = dir.join("config").join(SETTING_FILE);
        save_setting_to(&path, Setting::System).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.join("config"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(SETTING_FILE)]);
    }

    #[test]
    fn the_file_holds_the_words_the_environment_variable_takes() {
        let dir = scratch_dir("words");
        let path = dir.join(SETTING_FILE);
        std::fs::write(&path, r#"{"titleBar": "none"}"#).unwrap();
        assert_eq!(load_setting_from(&path).setting, Setting::Undecorated);
        std::fs::write(&path, r#"{"titleBar": "system"}"#).unwrap();
        assert_eq!(load_setting_from(&path).setting, Setting::System);
        std::fs::write(&path, r#"{"titleBar": "app"}"#).unwrap();
        assert_eq!(load_setting_from(&path).setting, Setting::App);
    }

    #[test]
    fn empty_file_is_corrupt_and_the_app_bar() {
        let dir = scratch_dir("empty");
        let path = dir.join(SETTING_FILE);
        std::fs::write(&path, "").unwrap();
        assert_eq!(load_setting_from(&path), app_bar(Some("corrupt")));
    }

    #[test]
    fn unknown_value_is_corrupt_and_the_app_bar() {
        let dir = scratch_dir("unknown");
        let path = dir.join(SETTING_FILE);
        std::fs::write(&path, r#"{"titleBar": "server"}"#).unwrap();
        assert_eq!(load_setting_from(&path), app_bar(Some("corrupt")));
    }

    #[test]
    fn missing_key_is_corrupt_and_the_app_bar() {
        let dir = scratch_dir("missing-key");
        let path = dir.join(SETTING_FILE);
        std::fs::write(&path, r#"{"other": "system"}"#).unwrap();
        assert_eq!(load_setting_from(&path), app_bar(Some("corrupt")));
    }

    #[test]
    fn a_folder_where_the_file_should_be_is_unreadable() {
        let dir = scratch_dir("unreadable");
        let path = dir.join(SETTING_FILE);
        std::fs::create_dir_all(&path).unwrap();
        assert_eq!(load_setting_from(&path), app_bar(Some("unreadable")));
    }

    #[test]
    fn gnome_note_needs_gnome_and_the_system_bar() {
        assert!(gnome_drag_note(Some("GNOME"), Setting::System));
        assert!(gnome_drag_note(Some("ubuntu:GNOME"), Setting::System));
        assert!(!gnome_drag_note(Some("ubuntu:GNOME"), Setting::App));
        assert!(!gnome_drag_note(Some("ubuntu:GNOME"), Setting::Undecorated));
    }

    #[test]
    fn gnome_note_stays_off_on_other_desktops() {
        for desktop in ["KDE", "niri", "labwc:wlroots", "sway", ""] {
            assert!(!gnome_drag_note(Some(desktop), Setting::System), "{desktop}");
        }
        assert!(!gnome_drag_note(None, Setting::System));
    }

    #[test]
    fn the_setting_is_only_offered_on_wayland() {
        let on_wayland = setting_state(
            Setting::App,
            &started(true, TitleBarSource::Default),
            Some("KDE"),
        );
        let on_x11 = setting_state(
            Setting::App,
            &started(false, TitleBarSource::Default),
            Some("KDE"),
        );
        assert_eq!(on_wayland.supported, cfg!(target_os = "linux"));
        assert!(!on_x11.supported);
    }

    #[test]
    fn the_state_says_when_the_start_overrode_the_setting() {
        let overridden = |source| {
            setting_state(Setting::System, &started(true, source), Some("KDE")).overridden
        };
        assert!(overridden(TitleBarSource::CommandLine));
        assert!(overridden(TitleBarSource::Environment));
        assert!(!overridden(TitleBarSource::Setting));
        assert!(!overridden(TitleBarSource::Default));
    }

    #[test]
    fn the_state_carries_the_saved_choice_and_the_gnome_note() {
        let startup = started(true, TitleBarSource::Setting);
        let on_gnome = setting_state(Setting::System, &startup, Some("ubuntu:GNOME"));
        let on_kde = setting_state(Setting::System, &startup, Some("KDE"));
        assert_eq!(on_gnome.setting, Setting::System);
        assert!(on_gnome.gnome_note);
        assert!(!on_kde.gnome_note);
    }

    #[test]
    fn the_state_reaches_the_page_in_camel_case() {
        let startup = started(true, TitleBarSource::Environment);
        let state = setting_state(Setting::Undecorated, &startup, Some("GNOME"));
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!({
                "supported": cfg!(target_os = "linux"),
                "setting": "none",
                "overridden": true,
                "gnomeNote": false,
            })
        );
    }
}
