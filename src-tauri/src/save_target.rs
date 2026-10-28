// Writing a saved file to where the user asked for it.

use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Start of the error a save returns when it may not write the destination (a
/// read-only file, or a folder it cannot write to), so the frontend can say why
/// instead of showing a generic error.
pub const SAVE_DENIED_ERROR: &str = "save-denied";

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

// Refuses a destination that exists and is read-only, before anything is
// written. A new file passes: whether its folder takes it shows on writing.
pub fn check_writable_destination(dest: &Path) -> Result<(), String> {
    let Ok(metadata) = std::fs::metadata(dest) else {
        return Ok(());
    };
    if metadata.permissions().readonly() {
        return Err(denied(dest, "the file is read-only"));
    }
    Ok(())
}

// Puts a copy of `from` at `dest` in one step: the copy is written next to
// `dest` and renamed over it, so `dest` is either the old file or the whole new
// one. An existing `dest` keeps its permissions. On any failure `dest` is left
// as it was and the copy is removed.
pub fn write_beside_and_rename(from: &Path, dest: &Path) -> Result<(), String> {
    check_writable_destination(dest)?;
    let target = resolved_target(dest);
    let tmp = temp_path_beside(&target);
    let file = create_temp(&tmp).map_err(|e| save_error(dest, e))?;
    let result = fill_temp(file, from, &target).and_then(|_| std::fs::rename(&tmp, &target));
    let Err(error) = result else {
        return Ok(());
    };
    let _ = std::fs::remove_file(&tmp);
    Err(save_error(dest, error))
}

// A rename over a symlink would turn it into a regular file, so the file it
// points to is the one replaced.
fn resolved_target(dest: &Path) -> PathBuf {
    let is_link = std::fs::symlink_metadata(dest)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    if !is_link {
        return dest.to_path_buf();
    }
    std::fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf())
}

// Hidden, and unique to this process and call, in the destination's own folder
// so the rename never crosses file systems.
fn temp_path_beside(target: &Path) -> PathBuf {
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".{}.{}-{}.tmp", name, std::process::id(), n))
}

// create_new never opens a file that is already there.
fn create_temp(tmp: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(tmp)
}

// The handle is dropped on return, before the rename.
fn fill_temp(mut file: File, from: &Path, target: &Path) -> std::io::Result<()> {
    let mut source = File::open(from)?;
    std::io::copy(&mut source, &mut file)?;
    keep_permissions(&file, target)?;
    file.sync_all()
}

// A new destination keeps the defaults the temp file was created with.
fn keep_permissions(file: &File, target: &Path) -> std::io::Result<()> {
    let Ok(metadata) = std::fs::metadata(target) else {
        return Ok(());
    };
    file.set_permissions(metadata.permissions())
}

fn save_error(dest: &Path, error: std::io::Error) -> String {
    let refused = matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem
    );
    if refused {
        return denied(dest, &error.to_string());
    }
    format!("could not write {}: {}", dest.display(), error)
}

fn denied(dest: &Path, reason: &str) -> String {
    format!("{}: {}: {}", SAVE_DENIED_ERROR, dest.display(), reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    static DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn test_dir(label: &str) -> PathBuf {
        let n = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-save-target-{}-{}-{}",
            label,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // The new content, somewhere other than the destination's folder.
    fn new_content(label: &str) -> (PathBuf, PathBuf) {
        let source_dir = test_dir(&format!("{}-src", label));
        let from = source_dir.join("output.pdf");
        std::fs::write(&from, b"%PDF-1.7 new").unwrap();
        (source_dir, from)
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_new_destination_is_written() {
        let (source_dir, from) = new_content("new");
        let dir = test_dir("new");
        let dest = dir.join("doc.pdf");

        write_beside_and_rename(&from, &dest).unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 new");
        assert_eq!(names_in(&dir), vec!["doc.pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&source_dir);
    }

    #[test]
    fn an_existing_destination_is_replaced_and_the_source_kept() {
        let (source_dir, from) = new_content("replace");
        let dir = test_dir("replace");
        let dest = dir.join("doc.pdf");
        std::fs::write(&dest, b"%PDF-1.7 old").unwrap();

        write_beside_and_rename(&from, &dest).unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 new");
        assert_eq!(std::fs::read(&from).unwrap(), b"%PDF-1.7 new");
        assert_eq!(names_in(&dir), vec!["doc.pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&source_dir);
    }

    #[test]
    fn a_read_only_destination_is_refused_untouched() {
        let (source_dir, from) = new_content("ro");
        let dir = test_dir("ro");
        let dest = dir.join("doc.pdf");
        std::fs::write(&dest, b"%PDF-1.7 old").unwrap();
        let mut permissions = std::fs::metadata(&dest).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&dest, permissions).unwrap();

        let error = write_beside_and_rename(&from, &dest).unwrap_err();

        assert!(error.starts_with(SAVE_DENIED_ERROR), "{}", error);
        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 old");
        assert!(std::fs::metadata(&dest).unwrap().permissions().readonly());
        assert_eq!(names_in(&dir), vec!["doc.pdf"]);
        let mut permissions = std::fs::metadata(&dest).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&dest, permissions).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&source_dir);
    }

    #[test]
    fn a_writable_destination_passes_the_check() {
        let dir = test_dir("check");
        let dest = dir.join("doc.docx");
        assert!(check_writable_destination(&dest).is_ok());
        std::fs::write(&dest, b"PK").unwrap();
        assert!(check_writable_destination(&dest).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_read_only_destination_fails_the_check() {
        let dir = test_dir("check-ro");
        let dest = dir.join("doc.docx");
        std::fs::write(&dest, b"PK").unwrap();
        let mut permissions = std::fs::metadata(&dest).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&dest, permissions).unwrap();

        let error = check_writable_destination(&dest).unwrap_err();

        assert!(error.starts_with(SAVE_DENIED_ERROR), "{}", error);
        let mut permissions = std::fs::metadata(&dest).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&dest, permissions).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_source_leaves_no_temp_file_behind() {
        let dir = test_dir("no-source");
        let dest = dir.join("doc.pdf");
        std::fs::write(&dest, b"%PDF-1.7 old").unwrap();

        let result = write_beside_and_rename(&dir.join("missing.pdf"), &dest);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 old");
        assert_eq!(names_in(&dir), vec!["doc.pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_rename_leaves_no_temp_file_behind() {
        let (source_dir, from) = new_content("rename");
        let dir = test_dir("rename");
        // A rename cannot put a file over a non-empty directory.
        let dest = dir.join("doc.pdf");
        std::fs::create_dir_all(dest.join("inside")).unwrap();

        let result = write_beside_and_rename(&from, &dest);

        assert!(result.is_err());
        assert!(dest.join("inside").is_dir());
        assert_eq!(names_in(&dir), vec!["doc.pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&source_dir);
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn set_mode(path: &Path, mode: u32) {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }

        fn mode_of(path: &Path) -> u32 {
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777
        }

        #[test]
        fn a_replaced_destination_keeps_its_mode() {
            let (source_dir, from) = new_content("mode");
            set_mode(&from, 0o664);
            let dir = test_dir("mode");
            let dest = dir.join("doc.pdf");
            std::fs::write(&dest, b"%PDF-1.7 old").unwrap();
            set_mode(&dest, 0o640);

            write_beside_and_rename(&from, &dest).unwrap();

            assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 new");
            assert_eq!(mode_of(&dest), 0o640);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&source_dir);
        }

        #[test]
        fn a_mode_444_destination_is_refused_untouched() {
            let (source_dir, from) = new_content("444");
            let dir = test_dir("444");
            let dest = dir.join("doc.pdf");
            std::fs::write(&dest, b"%PDF-1.7 old").unwrap();
            set_mode(&dest, 0o444);

            let error = write_beside_and_rename(&from, &dest).unwrap_err();

            assert!(error.starts_with(SAVE_DENIED_ERROR), "{}", error);
            assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 old");
            assert_eq!(mode_of(&dest), 0o444);
            assert_eq!(names_in(&dir), vec!["doc.pdf"]);
            set_mode(&dest, 0o644);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&source_dir);
        }

        // Root writes anywhere, so there is nothing to check as root.
        fn running_as_root(own_dir: &Path) -> bool {
            let metadata = std::fs::metadata(own_dir).unwrap();
            std::os::unix::fs::MetadataExt::uid(&metadata) == 0
        }

        #[test]
        fn a_folder_it_cannot_write_is_refused_untouched() {
            let (source_dir, from) = new_content("555");
            let dir = test_dir("555");
            if running_as_root(&dir) {
                return;
            }
            let dest = dir.join("doc.pdf");
            std::fs::write(&dest, b"%PDF-1.7 old").unwrap();
            set_mode(&dest, 0o644);
            set_mode(&dir, 0o555);

            let result = write_beside_and_rename(&from, &dest);

            set_mode(&dir, 0o755);
            let error = result.unwrap_err();
            assert!(error.starts_with(SAVE_DENIED_ERROR), "{}", error);
            assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 old");
            assert_eq!(mode_of(&dest), 0o644);
            assert_eq!(names_in(&dir), vec!["doc.pdf"]);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&source_dir);
        }

        #[test]
        fn a_symlinked_destination_stays_a_symlink() {
            let (source_dir, from) = new_content("link");
            let dir = test_dir("link");
            let real = dir.join("real.pdf");
            std::fs::write(&real, b"%PDF-1.7 old").unwrap();
            let link = dir.join("doc.pdf");
            std::os::unix::fs::symlink(&real, &link).unwrap();

            write_beside_and_rename(&from, &link).unwrap();

            assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read(&real).unwrap(), b"%PDF-1.7 new");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&source_dir);
        }
    }
}
