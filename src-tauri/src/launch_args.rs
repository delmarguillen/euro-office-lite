// The launch arguments: the options the app knows and the document to open.
// Free of Tauri so the parsing can be tested on its own.

pub const SYSTEM_TITLE_BAR: &str = "--system-title-bar";
pub const NO_TITLE_BAR: &str = "--no-title-bar";

// The same choice for a launcher that cannot add options: "system" or "none".
pub const TITLE_BAR_ENV: &str = "EURO_OFFICE_LITE_TITLE_BAR";

const KNOWN_OPTIONS: [&str; 2] = [SYSTEM_TITLE_BAR, NO_TITLE_BAR];

// Where the window's title bar comes from. Only Linux Wayland acts on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleBar {
    // The GTK header bar tao adds on Wayland.
    Default,
    // No GTK header bar: GTK follows the compositor, which draws its own bar
    // where it prefers server-side decorations.
    System,
    // No decorations at all, for tiling compositors.
    Undecorated,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LaunchArgs {
    // The first argument that is not a known option, as given: it can still be
    // an unknown option or a missing file, which the caller rejects.
    pub document: Option<String>,
    pub system_title_bar: bool,
    pub no_title_bar: bool,
}

// `args` leaves out the program name. Known options are taken wherever they
// are; only the first document is kept, as a launch opens a single one.
pub fn parse_launch_args<I>(args: I) -> LaunchArgs
where
    I: IntoIterator<Item = String>,
{
    let mut parsed = LaunchArgs::default();
    for arg in args {
        if KNOWN_OPTIONS.contains(&arg.as_str()) {
            parsed.system_title_bar |= arg == SYSTEM_TITLE_BAR;
            parsed.no_title_bar |= arg == NO_TITLE_BAR;
            continue;
        }
        if parsed.document.is_none() {
            parsed.document = Some(arg);
        }
    }
    parsed
}

// The command line wins over the environment, and --no-title-bar over
// --system-title-bar. The notes are lines for the startup log.
pub fn resolve_title_bar(args: &LaunchArgs, env: Option<&str>) -> (TitleBar, Vec<String>) {
    let mut notes = Vec::new();
    let from_args = match (args.system_title_bar, args.no_title_bar) {
        (true, true) => {
            notes.push(format!(
                "both {SYSTEM_TITLE_BAR} and {NO_TITLE_BAR} given, using {NO_TITLE_BAR}"
            ));
            Some(TitleBar::Undecorated)
        }
        (false, true) => Some(TitleBar::Undecorated),
        (true, false) => Some(TitleBar::System),
        (false, false) => None,
    };
    // An empty value is the usual way to unset a variable in a launcher.
    let from_env = match env.map(str::trim) {
        None | Some("") => None,
        Some("system") => Some(TitleBar::System),
        Some("none") => Some(TitleBar::Undecorated),
        Some(other) => {
            notes.push(format!(
                "ignoring {TITLE_BAR_ENV}={other:?}: expected \"system\" or \"none\""
            ));
            None
        }
    };
    let mode = match (from_args, from_env) {
        (Some(from_args), Some(from_env)) => {
            if from_args != from_env {
                notes.push(format!("the command line option overrides {TITLE_BAR_ENV}"));
            }
            from_args
        }
        (Some(mode), None) | (None, Some(mode)) => mode,
        (None, None) => TitleBar::Default,
    };
    (mode, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> LaunchArgs {
        parse_launch_args(args.iter().map(|arg| arg.to_string()))
    }

    fn document(args: &[&str]) -> Option<String> {
        parse(args).document
    }

    #[test]
    fn no_arguments_means_no_document() {
        assert_eq!(document(&[]), None);
    }

    #[test]
    fn a_lone_path_is_the_document() {
        assert_eq!(document(&["a.docx"]), Some("a.docx".to_string()));
    }

    #[test]
    fn option_before_the_path_is_not_the_document() {
        assert_eq!(
            document(&[SYSTEM_TITLE_BAR, "a.docx"]),
            Some("a.docx".to_string())
        );
        assert_eq!(
            document(&[NO_TITLE_BAR, "a.docx"]),
            Some("a.docx".to_string())
        );
    }

    #[test]
    fn option_after_the_path_is_not_the_document() {
        assert_eq!(
            document(&["a.docx", SYSTEM_TITLE_BAR]),
            Some("a.docx".to_string())
        );
        assert_eq!(
            document(&["a.docx", NO_TITLE_BAR]),
            Some("a.docx".to_string())
        );
    }

    #[test]
    fn options_alone_mean_no_document() {
        assert_eq!(document(&[SYSTEM_TITLE_BAR]), None);
        assert_eq!(document(&[NO_TITLE_BAR, SYSTEM_TITLE_BAR]), None);
    }

    #[test]
    fn only_the_first_of_several_paths_is_kept() {
        assert_eq!(
            document(&[SYSTEM_TITLE_BAR, "a.docx", "b.docx"]),
            Some("a.docx".to_string())
        );
    }

    // An unknown option is still the candidate, as before, so the path check
    // keeps rejecting it instead of a later argument being opened.
    #[test]
    fn unknown_option_stays_the_candidate() {
        assert_eq!(
            document(&["--unknown", "a.docx"]),
            Some("--unknown".to_string())
        );
        assert_eq!(
            document(&[SYSTEM_TITLE_BAR, "-psn_0_12345", "a.docx"]),
            Some("-psn_0_12345".to_string())
        );
    }

    #[test]
    fn options_are_recorded_wherever_they_are() {
        let parsed = parse(&["a.docx", SYSTEM_TITLE_BAR]);
        assert!(parsed.system_title_bar && !parsed.no_title_bar);
        let parsed = parse(&[NO_TITLE_BAR, "a.docx"]);
        assert!(!parsed.system_title_bar && parsed.no_title_bar);
        let parsed = parse(&[SYSTEM_TITLE_BAR, SYSTEM_TITLE_BAR]);
        assert!(parsed.system_title_bar && !parsed.no_title_bar);
    }

    // Matching is exact: anything else is not one of our options.
    #[test]
    fn near_misses_are_not_options() {
        let parsed = parse(&["--system-title-bar=yes", "--SYSTEM-TITLE-BAR"]);
        assert!(!parsed.system_title_bar && !parsed.no_title_bar);
        assert_eq!(parsed.document, Some("--system-title-bar=yes".to_string()));
    }

    fn resolve(args: &[&str], env: Option<&str>) -> (TitleBar, Vec<String>) {
        resolve_title_bar(&parse(args), env)
    }

    #[test]
    fn nothing_asked_keeps_the_default_quietly() {
        assert_eq!(resolve(&["a.docx"], None), (TitleBar::Default, vec![]));
        assert_eq!(resolve(&[], Some("")), (TitleBar::Default, vec![]));
        assert_eq!(resolve(&[], Some("  ")), (TitleBar::Default, vec![]));
    }

    #[test]
    fn each_option_picks_its_mode() {
        assert_eq!(
            resolve(&[SYSTEM_TITLE_BAR], None),
            (TitleBar::System, vec![])
        );
        assert_eq!(
            resolve(&[NO_TITLE_BAR], None),
            (TitleBar::Undecorated, vec![])
        );
    }

    #[test]
    fn no_title_bar_wins_over_system_title_bar_and_says_so() {
        for args in [
            [SYSTEM_TITLE_BAR, NO_TITLE_BAR],
            [NO_TITLE_BAR, SYSTEM_TITLE_BAR],
        ] {
            let (mode, notes) = resolve(&args, None);
            assert_eq!(mode, TitleBar::Undecorated);
            assert_eq!(notes.len(), 1);
            assert!(notes[0].contains("using --no-title-bar"));
        }
    }

    #[test]
    fn environment_picks_the_mode_without_options() {
        assert_eq!(resolve(&[], Some("system")), (TitleBar::System, vec![]));
        assert_eq!(
            resolve(&["a.docx"], Some("none")),
            (TitleBar::Undecorated, vec![])
        );
    }

    #[test]
    fn command_line_wins_over_environment() {
        let (mode, notes) = resolve(&[SYSTEM_TITLE_BAR], Some("none"));
        assert_eq!(mode, TitleBar::System);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("overrides"));
        assert_eq!(
            resolve(&[NO_TITLE_BAR], Some("none")),
            (TitleBar::Undecorated, vec![])
        );
    }

    #[test]
    fn unknown_environment_value_is_ignored_with_a_note() {
        let (mode, notes) = resolve(&[], Some("server"));
        assert_eq!(mode, TitleBar::Default);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains(TITLE_BAR_ENV) && notes[0].contains("server"));
        // The command line still applies.
        let (mode, notes) = resolve(&[NO_TITLE_BAR], Some("System"));
        assert_eq!(mode, TitleBar::Undecorated);
        assert_eq!(notes.len(), 1);
    }
}
