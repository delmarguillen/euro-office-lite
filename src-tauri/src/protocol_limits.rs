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

// The image types the insert dialog offers and copy-to-media copies.
pub const IMAGE_EXTENSIONS: [&str; 11] = [
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "tif", "tiff", "emf", "wmf",
];

pub fn has_image_extension(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

// An absolute path copy-to-media may read: one the user picked in the image
// dialog, and an image.
pub fn may_copy_absolute(path: &Path, picked: &[PathBuf]) -> bool {
    if !picked.iter().any(|choice| choice == path) {
        return false;
    }
    has_image_extension(path)
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

    #[test]
    fn a_picked_image_may_be_copied() {
        let picked = vec![PathBuf::from("/home/u/Pictures/a.PNG"), PathBuf::from("/home/u/b.emf")];
        assert!(may_copy_absolute(Path::new("/home/u/Pictures/a.PNG"), &picked));
        assert!(may_copy_absolute(Path::new("/home/u/b.emf"), &picked));
    }

    #[test]
    fn a_path_that_was_not_picked_is_refused() {
        let picked = vec![PathBuf::from("/home/u/Pictures/a.png")];
        assert!(!may_copy_absolute(Path::new("/home/u/Pictures/b.png"), &picked));
        assert!(!may_copy_absolute(Path::new("/home/u/Pictures/a.png"), &[]));
    }

    #[test]
    fn a_picked_file_that_is_not_an_image_is_refused() {
        let picked = vec![PathBuf::from("/home/u/.ssh/id_rsa"), PathBuf::from("/home/u/notes.txt")];
        assert!(!may_copy_absolute(Path::new("/home/u/.ssh/id_rsa"), &picked));
        assert!(!may_copy_absolute(Path::new("/home/u/notes.txt"), &picked));
    }

    #[test]
    fn image_extensions_ignore_case() {
        assert!(has_image_extension(Path::new("x.JPeG")));
        assert!(has_image_extension(Path::new("x.tiff")));
        assert!(!has_image_extension(Path::new("x.ico")));
        assert!(!has_image_extension(Path::new("png")));
    }
}
