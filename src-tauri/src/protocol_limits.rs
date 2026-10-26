// What each route of the ascdesktop protocol accepts. Every rule is a plain
// function so it can be tested without a running app.

use std::path::{Component, Path, PathBuf};

// A path under the resource directory: only plain names, so no root, drive
// prefix (\\?\ included) or ".." can move the join outside it.
pub fn resource_relative(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    let mut relative = PathBuf::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return None;
        };
        relative.push(name);
    }
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_resource_path_is_accepted() {
        assert_eq!(
            resource_relative("sdkjs/word/sdk-all.js"),
            Some(PathBuf::from("sdkjs/word/sdk-all.js"))
        );
        assert_eq!(resource_relative("index.html"), Some(PathBuf::from("index.html")));
    }

    #[test]
    fn a_current_dir_component_is_refused() {
        assert_eq!(resource_relative("./fonts/a.ttf"), None);
    }

    #[test]
    fn a_parent_component_is_refused() {
        assert_eq!(resource_relative("../secret"), None);
        assert_eq!(resource_relative("sdkjs/../../etc/passwd"), None);
    }

    #[test]
    fn an_absolute_path_is_refused() {
        assert_eq!(resource_relative("/etc/passwd"), None);
    }

    #[test]
    fn an_empty_path_is_refused() {
        assert_eq!(resource_relative(""), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_or_verbatim_path_is_refused() {
        assert_eq!(resource_relative("C:\\Windows\\win.ini"), None);
        assert_eq!(resource_relative("C:win.ini"), None);
        assert_eq!(resource_relative("\\\\?\\C:\\Windows\\win.ini"), None);
        assert_eq!(resource_relative("\\\\server\\share\\x"), None);
    }
}
