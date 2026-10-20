// The launch arguments: the options the app knows and the document to open.
// Free of Tauri so the parsing can be tested on its own.

pub const SYSTEM_TITLE_BAR: &str = "--system-title-bar";
pub const NO_TITLE_BAR: &str = "--no-title-bar";

const KNOWN_OPTIONS: [&str; 2] = [SYSTEM_TITLE_BAR, NO_TITLE_BAR];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LaunchArgs {
    // The first argument that is not a known option, as given: it can still be
    // an unknown option or a missing file, which the caller rejects.
    pub document: Option<String>,
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
            continue;
        }
        if parsed.document.is_none() {
            parsed.document = Some(arg);
        }
    }
    parsed
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
}
