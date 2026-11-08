// Whether to set WEBKIT_DISABLE_DMABUF_RENDERER=1 before WebKitGTK starts.
//
// From WebKitGTK 2.44 on, any value other than "0" turns off hardware
// acceleration in the web view, not only the DMABUF transport
// (Source/WebKit/UIProcess/gtk/HardwareAccelerationManager.cpp). It keeps the
// canvas text clean with NVIDIA's proprietary driver (issue #27); on Intel it
// made scrolling slower (issue #65). So it is set only for that driver.
//
// The decision is by driver, not by GPU vendor: an NVIDIA card on nouveau
// renders like Intel or AMD.

use std::path::Path;

pub const ENV_NAME: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

// The kernel lists every loaded module here.
const SYS_MODULE_DIR: &str = "/sys/module";
// Created by NVIDIA's proprietary kernel module, closed or open source.
const NVIDIA_SYS_MODULE: &str = "/sys/module/nvidia";
// Created by the same module. Flatpak does not share /sys/module with the
// sandbox, but it mounts its own /proc, where this file is still visible.
const NVIDIA_PROC_VERSION: &str = "/proc/driver/nvidia/version";

// What the decision looks at, read from the system by `observe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    // The value the user set, if any. WebKitGTK reads "0" as "keep DMABUF".
    pub user_value: Option<String>,
    pub nvidia_proprietary_loaded: bool,
    pub module_list_readable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    // The user set the variable; it is left as it is.
    User,
    // NVIDIA's proprietary module is loaded: the flag is set.
    NvidiaProprietary,
    // The loaded modules cannot be listed: the flag is set to be safe.
    UnknownDriver,
    // Another driver (Intel, AMD, nouveau...): the flag is not set.
    NotNeeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub set_flag: bool,
    pub reason: Reason,
}

pub fn decide(observed: &Observed) -> Decision {
    if observed.user_value.is_some() {
        return Decision {
            set_flag: false,
            reason: Reason::User,
        };
    }
    if observed.nvidia_proprietary_loaded {
        return Decision {
            set_flag: true,
            reason: Reason::NvidiaProprietary,
        };
    }
    if !observed.module_list_readable {
        return Decision {
            set_flag: true,
            reason: Reason::UnknownDriver,
        };
    }
    Decision {
        set_flag: false,
        reason: Reason::NotNeeded,
    }
}

pub fn observe() -> Observed {
    let user_value = std::env::var_os(ENV_NAME).map(|value| value.to_string_lossy().into_owned());
    let nvidia_in_sys = Path::new(NVIDIA_SYS_MODULE).exists();
    let nvidia_in_proc = Path::new(NVIDIA_PROC_VERSION).exists();
    let module_list_readable = std::fs::read_dir(SYS_MODULE_DIR).is_ok();
    Observed {
        user_value,
        nvidia_proprietary_loaded: nvidia_in_sys || nvidia_in_proc,
        module_list_readable,
    }
}

// Must run before anything starts WebKitGTK, while the process has one thread.
pub fn apply() -> Decision {
    let decision = decide(&observe());
    if decision.set_flag {
        std::env::set_var(ENV_NAME, "1");
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(user_value: Option<&str>, nvidia: bool, readable: bool) -> Observed {
        Observed {
            user_value: user_value.map(str::to_string),
            nvidia_proprietary_loaded: nvidia,
            module_list_readable: readable,
        }
    }

    #[test]
    fn user_value_zero_is_left_alone_even_with_nvidia() {
        let decision = decide(&observed(Some("0"), true, true));
        assert_eq!(
            decision,
            Decision {
                set_flag: false,
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
                set_flag: false,
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
                set_flag: false,
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
                set_flag: true,
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
                set_flag: true,
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
                set_flag: false,
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
                set_flag: true,
                reason: Reason::UnknownDriver
            }
        );
    }
}
