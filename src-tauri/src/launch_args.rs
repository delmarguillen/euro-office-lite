// The launch arguments: the options the app knows and the document to open.
// Free of Tauri so the parsing can be tested on its own.

pub const SYSTEM_TITLE_BAR: &str = "--system-title-bar";
pub const NO_TITLE_BAR: &str = "--no-title-bar";
// Added by open_in_new_window: this process is a second window, so it opens
// offset from the restored position instead of on top of the first one.
pub const NEW_WINDOW: &str = "--new-window";
// Added with NEW_WINDOW when the file is a template to start a new document
// from (new_document_in_new_window). Only a template honours it.
pub const FROM_TEMPLATE: &str = "--from-template";

// The same choice for a launcher that cannot add options: "system" or "none".
pub const TITLE_BAR_ENV: &str = "EURO_OFFICE_LITE_TITLE_BAR";

const KNOWN_OPTIONS: [&str; 4] = [SYSTEM_TITLE_BAR, NO_TITLE_BAR, NEW_WINDOW, FROM_TEMPLATE];

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
    pub new_window: bool,
    pub from_template: bool,
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
            parsed.new_window |= arg == NEW_WINDOW;
            parsed.from_template |= arg == FROM_TEMPLATE;
            continue;
        }
        if parsed.document.is_none() {
            parsed.document = Some(arg);
        }
    }
    parsed
}

// The title bar options as given, to pass on to another process of the app.
pub fn title_bar_options(args: &LaunchArgs) -> Vec<&'static str> {
    let mut options = Vec::new();
    if args.system_title_bar {
        options.push(SYSTEM_TITLE_BAR);
    }
    if args.no_title_bar {
        options.push(NO_TITLE_BAR);
    }
    options
}

// What decided the title bar mode, for the startup log and the settings screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleBarSource {
    CommandLine,
    Environment,
    // The choice saved in Configuración (title_bar_setting.rs).
    Setting,
    Default,
}

impl TitleBarSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TitleBarSource::CommandLine => "command-line",
            TitleBarSource::Environment => "environment",
            TitleBarSource::Setting => "setting",
            TitleBarSource::Default => "default",
        }
    }

    // The saved setting was not looked at: this start was told its mode.
    pub fn overrides_setting(self) -> bool {
        matches!(
            self,
            TitleBarSource::CommandLine | TitleBarSource::Environment
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedTitleBar {
    pub mode: TitleBar,
    pub source: TitleBarSource,
    // Lines for the startup log.
    pub notes: Vec<String>,
}

// The command line wins over the environment, the environment over the saved
// setting, and --no-title-bar over --system-title-bar.
pub fn resolve_title_bar(
    args: &LaunchArgs,
    env: Option<&str>,
    setting: TitleBar,
) -> ResolvedTitleBar {
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
    let (mode, source) = match (from_args, from_env) {
        (Some(from_args), Some(from_env)) => {
            if from_args != from_env {
                notes.push(format!("the command line option overrides {TITLE_BAR_ENV}"));
            }
            (from_args, TitleBarSource::CommandLine)
        }
        (Some(mode), None) => (mode, TitleBarSource::CommandLine),
        (None, Some(mode)) => (mode, TitleBarSource::Environment),
        (None, None) if setting != TitleBar::Default => (setting, TitleBarSource::Setting),
        (None, None) => (TitleBar::Default, TitleBarSource::Default),
    };
    ResolvedTitleBar {
        mode,
        source,
        notes,
    }
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
    fn title_bar_options_are_passed_on_as_given() {
        assert!(title_bar_options(&parse(&["a.docx"])).is_empty());
        assert_eq!(
            title_bar_options(&parse(&["a.docx", SYSTEM_TITLE_BAR])),
            vec![SYSTEM_TITLE_BAR]
        );
        assert_eq!(
            title_bar_options(&parse(&[NO_TITLE_BAR, SYSTEM_TITLE_BAR])),
            vec![SYSTEM_TITLE_BAR, NO_TITLE_BAR]
        );
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

    #[test]
    fn new_window_is_an_option_not_the_document() {
        let parsed = parse(&[NEW_WINDOW, "a.docx"]);
        assert!(parsed.new_window);
        assert_eq!(parsed.document, Some("a.docx".to_string()));
        let parsed = parse(&["a.docx", NEW_WINDOW]);
        assert!(parsed.new_window);
        assert_eq!(parsed.document, Some("a.docx".to_string()));
        assert_eq!(document(&[NEW_WINDOW]), None);
        assert!(!parse(&["a.docx"]).new_window);
    }

    #[test]
    fn from_template_is_an_option_not_the_document() {
        let parsed = parse(&[NEW_WINDOW, FROM_TEMPLATE, "a.dotx"]);
        assert!(parsed.new_window && parsed.from_template);
        assert_eq!(parsed.document, Some("a.dotx".to_string()));
        assert!(title_bar_options(&parsed).is_empty());
        assert!(!parse(&[NEW_WINDOW, "a.dotx"]).from_template);
    }

    #[test]
    fn new_window_is_not_a_title_bar_option() {
        let parsed = parse(&[NEW_WINDOW, SYSTEM_TITLE_BAR, "a.docx"]);
        assert_eq!(title_bar_options(&parsed), vec![SYSTEM_TITLE_BAR]);
        let resolved = resolve_title_bar(&parsed, None, TitleBar::Default);
        assert_eq!(resolved.mode, TitleBar::System);
    }

    // Matching is exact: anything else is not one of our options.
    #[test]
    fn near_misses_are_not_options() {
        let parsed = parse(&["--system-title-bar=yes", "--SYSTEM-TITLE-BAR"]);
        assert!(!parsed.system_title_bar && !parsed.no_title_bar);
        assert_eq!(parsed.document, Some("--system-title-bar=yes".to_string()));
    }

    // Without a saved setting, as before there was one.
    fn resolve(args: &[&str], env: Option<&str>) -> (TitleBar, Vec<String>) {
        let resolved = resolve_title_bar(&parse(args), env, TitleBar::Default);
        (resolved.mode, resolved.notes)
    }

    fn resolve_with_setting(
        args: &[&str],
        env: Option<&str>,
        setting: TitleBar,
    ) -> (TitleBar, TitleBarSource) {
        let resolved = resolve_title_bar(&parse(args), env, setting);
        (resolved.mode, resolved.source)
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

    #[test]
    fn saved_setting_picks_the_mode_when_nothing_else_asks() {
        assert_eq!(
            resolve_with_setting(&["a.docx"], None, TitleBar::System),
            (TitleBar::System, TitleBarSource::Setting)
        );
        assert_eq!(
            resolve_with_setting(&[], Some(""), TitleBar::Undecorated),
            (TitleBar::Undecorated, TitleBarSource::Setting)
        );
    }

    #[test]
    fn saved_app_title_bar_is_the_default() {
        assert_eq!(
            resolve_with_setting(&[], None, TitleBar::Default),
            (TitleBar::Default, TitleBarSource::Default)
        );
    }

    #[test]
    fn environment_wins_over_the_saved_setting() {
        assert_eq!(
            resolve_with_setting(&[], Some("none"), TitleBar::System),
            (TitleBar::Undecorated, TitleBarSource::Environment)
        );
        assert_eq!(
            resolve_with_setting(&[], Some("system"), TitleBar::Undecorated),
            (TitleBar::System, TitleBarSource::Environment)
        );
    }

    #[test]
    fn command_line_wins_over_environment_and_the_saved_setting() {
        assert_eq!(
            resolve_with_setting(&[SYSTEM_TITLE_BAR], None, TitleBar::Undecorated),
            (TitleBar::System, TitleBarSource::CommandLine)
        );
        assert_eq!(
            resolve_with_setting(&[NO_TITLE_BAR], Some("system"), TitleBar::System),
            (TitleBar::Undecorated, TitleBarSource::CommandLine)
        );
    }

    #[test]
    fn ignored_environment_value_leaves_the_saved_setting_to_decide() {
        let resolved = resolve_title_bar(&parse(&[]), Some("server"), TitleBar::System);
        assert_eq!(resolved.mode, TitleBar::System);
        assert_eq!(resolved.source, TitleBarSource::Setting);
        assert_eq!(resolved.notes.len(), 1);
    }

    #[test]
    fn only_the_command_line_and_the_environment_override_the_setting() {
        assert!(TitleBarSource::CommandLine.overrides_setting());
        assert!(TitleBarSource::Environment.overrides_setting());
        assert!(!TitleBarSource::Setting.overrides_setting());
        assert!(!TitleBarSource::Default.overrides_setting());
    }

    #[test]
    fn sources_have_their_log_names() {
        assert_eq!(TitleBarSource::CommandLine.as_str(), "command-line");
        assert_eq!(TitleBarSource::Environment.as_str(), "environment");
        assert_eq!(TitleBarSource::Setting.as_str(), "setting");
        assert_eq!(TitleBarSource::Default.as_str(), "default");
    }
}
