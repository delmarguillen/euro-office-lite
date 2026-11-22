// Whether to set WEBKIT_DISABLE_DMABUF_RENDERER before WebKitGTK starts.
//
// From WebKitGTK 2.44 on, any value other than "0" turns off hardware
// acceleration in the web view, not only the DMABUF transport
// (Source/WebKit/UIProcess/gtk/HardwareAccelerationManager.cpp). It keeps the
// canvas text clean with NVIDIA's proprietary driver (issue #27); on Intel it
// made scrolling slower (issue #65). So it is set only for that driver.
//
// The decision is by driver, not by GPU vendor: an NVIDIA card on nouveau
// renders like Intel or AMD.
//
// The user can override the driver rule in Configuración (Linux only): "on"
// keeps DMABUF ("0"), "off" turns it off ("1"), "auto" follows the driver. The
// choice is a small file in the app's config folder, read here because the
// window does not exist yet. The module compiles on every platform so the
// commands can be registered everywhere; only Linux reads or writes the file.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

pub const ENV_NAME: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

// The kernel lists every loaded module here.
const SYS_MODULE_DIR: &str = "/sys/module";
// Created by NVIDIA's proprietary kernel module, closed or open source.
const NVIDIA_SYS_MODULE: &str = "/sys/module/nvidia";
// Created by the same module. Flatpak does not share /sys/module with the
// sandbox, but it mounts its own /proc, where this file is still visible.
const NVIDIA_PROC_VERSION: &str = "/proc/driver/nvidia/version";

// Created in the app's config folder, next to recent-files.json.
const SETTING_FILE: &str = "dmabuf-renderer.json";

// The variable as the user set it, before apply() overwrites it. The settings
// screen reads it to say that the environment value wins over the setting.
static USER_VALUE: OnceLock<Option<String>> = OnceLock::new();

// The choice in Configuración. Auto follows the driver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Setting {
    #[default]
    Auto,
    On,
    Off,
}

// What the decision looks at, read from the system by `observe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    // The value the user set, if any. WebKitGTK reads "0" as "keep DMABUF".
    pub user_value: Option<String>,
    pub setting: Setting,
    pub nvidia_proprietary_loaded: bool,
    pub module_list_readable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    // The user set the variable; it is left as it is.
    User,
    // The choice in Configuración decided it.
    Setting,
    // NVIDIA's proprietary module is loaded: the flag is set.
    NvidiaProprietary,
    // The loaded modules cannot be listed: the flag is set to be safe.
    UnknownDriver,
    // Another driver (Intel, AMD, nouveau...): the flag is not set.
    NotNeeded,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::User => "user",
            Reason::Setting => "setting",
            Reason::NvidiaProprietary => "nvidia-proprietary",
            Reason::UnknownDriver => "unknown-driver",
            Reason::NotNeeded => "not-needed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    // The value to set, or None to leave the variable as it is.
    pub value: Option<&'static str>,
    pub reason: Reason,
}

pub fn decide(observed: &Observed) -> Decision {
    if observed.user_value.is_some() {
        return Decision {
            value: None,
            reason: Reason::User,
        };
    }
    match observed.setting {
        Setting::On => {
            return Decision {
                value: Some("0"),
                reason: Reason::Setting,
            }
        }
        Setting::Off => {
            return Decision {
                value: Some("1"),
                reason: Reason::Setting,
            }
        }
        Setting::Auto => {}
    }
    if observed.nvidia_proprietary_loaded {
        return Decision {
            value: Some("1"),
            reason: Reason::NvidiaProprietary,
        };
    }
    if !observed.module_list_readable {
        return Decision {
            value: Some("1"),
            reason: Reason::UnknownDriver,
        };
    }
    Decision {
        value: None,
        reason: Reason::NotNeeded,
    }
}

// The setting read from the file, and the problem when the default was used
// instead: "unreadable" or "corrupt". A missing file is not a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub setting: Setting,
    pub problem: Option<&'static str>,
}

#[derive(Serialize, Deserialize)]
struct StoredSetting {
    dmabuf: Setting,
}

fn automatic(problem: Option<&'static str>) -> Loaded {
    Loaded {
        setting: Setting::Auto,
        problem,
    }
}

fn load_setting_from(path: &Path) -> Loaded {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return automatic(None),
        Err(_) => return automatic(Some("unreadable")),
    };
    match serde_json::from_str::<StoredSetting>(&raw) {
        Ok(stored) => Loaded {
            setting: stored.dmabuf,
            problem: None,
        },
        Err(_) => automatic(Some("corrupt")),
    }
}

fn save_setting_to(path: &Path, setting: Setting) -> Result<(), String> {
    let parent = path.parent().ok_or("no-folder")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&StoredSetting { dmabuf: setting })
        .map_err(|error| error.to_string())?;
    // Write through a temporary file so a crash mid-write cannot leave a
    // truncated file behind; rename replaces the destination.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes()).map_err(|error| error.to_string())?;
    std::fs::rename(&tmp, path).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        error.to_string()
    })
}

// The folder Tauri's app_config_dir() gives on Linux: $XDG_CONFIG_HOME when it
// is an absolute path, else ~/.config, then the bundle identifier. Inside the
// Flatpak both are the sandbox's own folders, which the sandbox lets the app
// write.
fn linux_config_dir(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
    identifier: &str,
) -> Option<PathBuf> {
    let xdg = xdg_config_home.map(PathBuf::from);
    if let Some(xdg) = xdg.filter(|xdg| xdg.is_absolute()) {
        return Some(xdg.join(identifier));
    }
    let home = home.filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join(".config").join(identifier))
}

fn settings_path(identifier: &str) -> Option<PathBuf> {
    linux_config_dir(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
        identifier,
    )
    .map(|dir| dir.join(SETTING_FILE))
}

fn settings_path_for(app: &AppHandle) -> Option<PathBuf> {
    settings_path(&app.config().identifier)
}

fn observe(setting: Setting) -> Observed {
    let user_value = std::env::var_os(ENV_NAME).map(|value| value.to_string_lossy().into_owned());
    let nvidia_in_sys = Path::new(NVIDIA_SYS_MODULE).exists();
    let nvidia_in_proc = Path::new(NVIDIA_PROC_VERSION).exists();
    let module_list_readable = std::fs::read_dir(SYS_MODULE_DIR).is_ok();
    Observed {
        user_value,
        setting,
        nvidia_proprietary_loaded: nvidia_in_sys || nvidia_in_proc,
        module_list_readable,
    }
}

// What apply() decided, and the problem with the settings file if there was one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    pub decision: Decision,
    pub setting_problem: Option<&'static str>,
}

// Must run before anything starts WebKitGTK, while the process has one thread.
pub fn apply(identifier: &str) -> Applied {
    let loaded = match settings_path(identifier) {
        Some(path) => load_setting_from(&path),
        None => automatic(Some("no-folder")),
    };
    let observed = observe(loaded.setting);
    let _ = USER_VALUE.set(observed.user_value.clone());
    let decision = decide(&observed);
    if let Some(value) = decision.value {
        std::env::set_var(ENV_NAME, value);
    }
    Applied {
        decision,
        setting_problem: loaded.problem,
    }
}

// The end of the "[STARTUP] Linux ..." line: the value WebKitGTK sees and why.
pub fn log_field(value: Option<&str>, reason: Reason) -> String {
    format!(
        "{}={} reason={}",
        ENV_NAME,
        value.unwrap_or("unset"),
        reason.as_str()
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingState {
    // Only Linux has the setting; the settings screen hides it elsewhere.
    supported: bool,
    // The saved choice, as the next start will read it.
    setting: Setting,
    // The variable was already set when the app started, so it wins.
    env_set: bool,
}

#[tauri::command]
pub fn dmabuf_setting_state(app: AppHandle) -> SettingState {
    let supported = cfg!(target_os = "linux");
    let setting = match settings_path_for(&app) {
        Some(path) if supported => load_setting_from(&path).setting,
        _ => Setting::Auto,
    };
    SettingState {
        supported,
        setting,
        env_set: matches!(USER_VALUE.get(), Some(Some(_))),
    }
}

#[tauri::command]
pub fn set_dmabuf_setting(app: AppHandle, setting: Setting) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("not-supported".to_string());
    }
    let path = settings_path_for(&app).ok_or("no-folder")?;
    save_setting_to(&path, setting)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(user_value: Option<&str>, nvidia: bool, readable: bool) -> Observed {
        Observed {
            user_value: user_value.map(str::to_string),
            setting: Setting::Auto,
            nvidia_proprietary_loaded: nvidia,
            module_list_readable: readable,
        }
    }

    fn with_setting(setting: Setting, user_value: Option<&str>, nvidia: bool) -> Observed {
        Observed {
            setting,
            ..observed(user_value, nvidia, true)
        }
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("eol-dmabuf-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn user_value_zero_is_left_alone_even_with_nvidia() {
        let decision = decide(&observed(Some("0"), true, true));
        assert_eq!(
            decision,
            Decision {
                value: None,
                reason: Reason::User
            }
        );
    }

    #[test]
    fn user_value_one_is_left_alone_without_nvidia() {
        let decision = decide(&observed(Some("1"), false, true));
        assert_eq!(
            decision,
            Decision {
                value: None,
                reason: Reason::User
            }
        );
    }

    #[test]
    fn user_value_wins_when_modules_are_unreadable() {
        let decision = decide(&observed(Some("1"), false, false));
        assert_eq!(
            decision,
            Decision {
                value: None,
                reason: Reason::User
            }
        );
    }

    #[test]
    fn nvidia_proprietary_sets_the_flag() {
        let decision = decide(&observed(None, true, true));
        assert_eq!(
            decision,
            Decision {
                value: Some("1"),
                reason: Reason::NvidiaProprietary
            }
        );
    }

    #[test]
    fn nvidia_seen_only_in_proc_sets_the_flag() {
        // Flatpak: /sys/module is not shared, /proc/driver/nvidia is.
        let decision = decide(&observed(None, true, false));
        assert_eq!(
            decision,
            Decision {
                value: Some("1"),
                reason: Reason::NvidiaProprietary
            }
        );
    }

    #[test]
    fn other_drivers_leave_the_flag_unset() {
        // Intel, AMD and nouveau all look like this.
        let decision = decide(&observed(None, false, true));
        assert_eq!(
            decision,
            Decision {
                value: None,
                reason: Reason::NotNeeded
            }
        );
    }

    #[test]
    fn unreadable_module_list_sets_the_flag() {
        let decision = decide(&observed(None, false, false));
        assert_eq!(
            decision,
            Decision {
                value: Some("1"),
                reason: Reason::UnknownDriver
            }
        );
    }

    #[test]
    fn setting_on_keeps_dmabuf_with_nvidia() {
        let decision = decide(&with_setting(Setting::On, None, true));
        assert_eq!(
            decision,
            Decision {
                value: Some("0"),
                reason: Reason::Setting
            }
        );
    }

    #[test]
    fn setting_on_keeps_dmabuf_without_nvidia() {
        let decision = decide(&with_setting(Setting::On, None, false));
        assert_eq!(
            decision,
            Decision {
                value: Some("0"),
                reason: Reason::Setting
            }
        );
    }

    #[test]
    fn setting_off_disables_dmabuf_without_nvidia() {
        let decision = decide(&with_setting(Setting::Off, None, false));
        assert_eq!(
            decision,
            Decision {
                value: Some("1"),
                reason: Reason::Setting
            }
        );
    }

    #[test]
    fn setting_off_disables_dmabuf_with_nvidia() {
        let decision = decide(&with_setting(Setting::Off, None, true));
        assert_eq!(
            decision,
            Decision {
                value: Some("1"),
                reason: Reason::Setting
            }
        );
    }

    #[test]
    fn user_value_beats_the_setting() {
        let on = decide(&with_setting(Setting::On, Some("1"), false));
        let off = decide(&with_setting(Setting::Off, Some("0"), true));
        assert_eq!(
            on,
            Decision {
                value: None,
                reason: Reason::User
            }
        );
        assert_eq!(
            off,
            Decision {
                value: None,
                reason: Reason::User
            }
        );
    }

    #[test]
    fn automatic_setting_follows_the_driver() {
        let decision = decide(&with_setting(Setting::Auto, None, true));
        assert_eq!(
            decision,
            Decision {
                value: Some("1"),
                reason: Reason::NvidiaProprietary
            }
        );
    }

    #[test]
    fn missing_file_is_automatic_without_a_problem() {
        let dir = scratch_dir("missing");
        let loaded = load_setting_from(&dir.join(SETTING_FILE));
        assert_eq!(
            loaded,
            Loaded {
                setting: Setting::Auto,
                problem: None
            }
        );
    }

    #[test]
    fn each_saved_value_is_read_back() {
        let dir = scratch_dir("round-trip");
        let path = dir.join(SETTING_FILE);
        for setting in [Setting::Auto, Setting::On, Setting::Off] {
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
    fn empty_file_is_corrupt_and_automatic() {
        let dir = scratch_dir("empty");
        let path = dir.join(SETTING_FILE);
        std::fs::write(&path, "").unwrap();
        assert_eq!(
            load_setting_from(&path),
            Loaded {
                setting: Setting::Auto,
                problem: Some("corrupt")
            }
        );
    }

    #[test]
    fn unknown_value_is_corrupt_and_automatic() {
        let dir = scratch_dir("unknown");
        let path = dir.join(SETTING_FILE);
        std::fs::write(&path, r#"{"dmabuf": "maybe"}"#).unwrap();
        assert_eq!(
            load_setting_from(&path),
            Loaded {
                setting: Setting::Auto,
                problem: Some("corrupt")
            }
        );
    }

    #[test]
    fn missing_key_is_corrupt_and_automatic() {
        let dir = scratch_dir("missing-key");
        let path = dir.join(SETTING_FILE);
        std::fs::write(&path, r#"{"other": "on"}"#).unwrap();
        assert_eq!(
            load_setting_from(&path),
            Loaded {
                setting: Setting::Auto,
                problem: Some("corrupt")
            }
        );
    }

    #[test]
    fn a_folder_where_the_file_should_be_is_unreadable() {
        let dir = scratch_dir("unreadable");
        let path = dir.join(SETTING_FILE);
        std::fs::create_dir_all(&path).unwrap();
        assert_eq!(
            load_setting_from(&path),
            Loaded {
                setting: Setting::Auto,
                problem: Some("unreadable")
            }
        );
    }

    #[test]
    fn config_folder_follows_xdg_when_absolute() {
        let dir = linux_config_dir(
            Some(OsString::from("/custom/config")),
            Some(OsString::from("/home/u")),
            "org.euro-office.lite",
        );
        assert_eq!(
            dir,
            Some(PathBuf::from("/custom/config/org.euro-office.lite"))
        );
    }

    #[test]
    fn config_folder_ignores_a_relative_xdg_value() {
        let dir = linux_config_dir(
            Some(OsString::from("relative/config")),
            Some(OsString::from("/home/u")),
            "org.euro-office.lite",
        );
        assert_eq!(
            dir,
            Some(PathBuf::from("/home/u/.config/org.euro-office.lite"))
        );
    }

    #[test]
    fn config_folder_needs_a_home_without_xdg() {
        assert_eq!(linux_config_dir(None, Some(OsString::from("")), "id"), None);
        assert_eq!(linux_config_dir(None, None, "id"), None);
    }

    #[test]
    fn reasons_have_their_log_names() {
        assert_eq!(Reason::User.as_str(), "user");
        assert_eq!(Reason::Setting.as_str(), "setting");
        assert_eq!(Reason::NvidiaProprietary.as_str(), "nvidia-proprietary");
        assert_eq!(Reason::UnknownDriver.as_str(), "unknown-driver");
        assert_eq!(Reason::NotNeeded.as_str(), "not-needed");
    }

    #[test]
    fn log_field_shows_value_and_reason() {
        assert_eq!(
            log_field(Some("1"), Reason::NvidiaProprietary),
            "WEBKIT_DISABLE_DMABUF_RENDERER=1 reason=nvidia-proprietary"
        );
        assert_eq!(
            log_field(Some("0"), Reason::User),
            "WEBKIT_DISABLE_DMABUF_RENDERER=0 reason=user"
        );
        assert_eq!(
            log_field(Some("0"), Reason::Setting),
            "WEBKIT_DISABLE_DMABUF_RENDERER=0 reason=setting"
        );
        assert_eq!(
            log_field(None, Reason::NotNeeded),
            "WEBKIT_DISABLE_DMABUF_RENDERER=unset reason=not-needed"
        );
    }
}
