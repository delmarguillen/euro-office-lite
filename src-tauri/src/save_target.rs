// Writing a saved file to where the user asked for it.

use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Start of the error a save returns when it may not write the destination (a
/// read-only file, or a folder it cannot write to), so the frontend can say why
/// instead of showing a generic error.
pub const SAVE_DENIED_ERROR: &str = "save-denied";

/// Start of the error a save returns when another program holds the
/// destination open without sharing write access (Windows only).
#[cfg(windows)]
pub const SAVE_LOCKED_ERROR: &str = "save-locked";

// Windows ERROR_SHARING_VIOLATION.
#[cfg(windows)]
const SHARING_VIOLATION: i32 = 32;

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

// Refuses a destination that exists and is read-only or locked by another
// program, before anything is written. A new file passes: whether its folder
// takes it shows on writing.
pub fn check_writable_destination(dest: &Path) -> Result<(), String> {
    let Ok(metadata) = std::fs::metadata(dest) else {
        return Ok(());
    };
    if metadata.permissions().readonly() {
        return Err(denied(dest, "the file is read-only"));
    }
    if metadata.is_dir() {
        return Ok(());
    }
    check_not_locked(dest)
}

// Only Windows refuses to open a file another program holds open. Elsewhere the
// open finds no lock and fails on a file we may not write but may replace by
// rename, so it is skipped.
#[cfg(not(windows))]
fn check_not_locked(_dest: &Path) -> Result<(), String> {
    Ok(())
}

// Opening for write without truncate or create leaves content and dates as
// they were.
#[cfg(windows)]
fn check_not_locked(dest: &Path) -> Result<(), String> {
    let Err(error) = OpenOptions::new().write(true).open(dest) else {
        return Ok(());
    };
    if is_sharing_violation(&error) {
        return Err(locked(dest, &error.to_string()));
    }
    Err(save_error(dest, error))
}

#[cfg(windows)]
fn is_sharing_violation(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(SHARING_VIOLATION)
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

// Runs `convert` on a file named like `dest` in `staging_dir`, then puts the
// result at `dest` with write_beside_and_rename. x2t writes its output in place
// with no temp file of its own, so pointing it at `dest` could leave the user's
// file truncated. `reshape` runs on the staged file before it is placed. `dest`
// changes only when `convert` succeeded and wrote a non-empty file, and
// `staging_dir` is removed either way.
pub async fn convert_and_place<Convert, Converted>(
    staging_dir: &Path,
    dest: &Path,
    convert: Convert,
    reshape: impl FnOnce(&Path),
) -> Result<(), String>
where
    Convert: FnOnce(PathBuf) -> Converted,
    Converted: std::future::Future<Output = Result<String, String>>,
{
    let staged = staged_path(staging_dir, dest)?;
    let converted = convert(staged.clone()).await;
    let placed = converted
        .and_then(|_| check_not_empty(&staged))
        .and_then(|_| {
            reshape(&staged);
            write_beside_and_rename(&staged, dest)
        });
    let _ = std::fs::remove_dir_all(staging_dir);
    placed
}

// Same name and extension as `dest`: x2t and the text reshaping both go by
// the extension. What a crashed save left in `staging_dir` is cleared first.
fn staged_path(staging_dir: &Path, dest: &Path) -> Result<PathBuf, String> {
    let Some(name) = dest.file_name() else {
        return Err(format!("not a file name: {}", dest.display()));
    };
    let _ = std::fs::remove_dir_all(staging_dir);
    std::fs::create_dir_all(staging_dir).map_err(|e| e.to_string())?;
    Ok(staging_dir.join(name))
}

// x2t can exit 0 without writing anything.
fn check_not_empty(staged: &Path) -> Result<(), String> {
    let size = std::fs::metadata(staged).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err("x2t did not write the document".to_string());
    }
    Ok(())
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

#[cfg(windows)]
fn locked(dest: &Path, reason: &str) -> String {
    format!("{}: {}: {}", SAVE_LOCKED_ERROR, dest.display(), reason)
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
    fn the_check_leaves_an_existing_file_unchanged() {
        let dir = test_dir("check-untouched");
        let dest = dir.join("doc.docx");
        std::fs::write(&dest, b"PK original").unwrap();
        let before = std::fs::metadata(&dest).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        assert!(check_writable_destination(&dest).is_ok());

        assert_eq!(std::fs::read(&dest).unwrap(), b"PK original");
        assert_eq!(std::fs::metadata(&dest).unwrap().modified().unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_sharing_violation_is_a_locked_error() {
        let error = std::io::Error::from_raw_os_error(SHARING_VIOLATION);
        assert!(is_sharing_violation(&error));
        let message = locked(Path::new("doc.docx"), "in use");
        assert!(message.starts_with(SAVE_LOCKED_ERROR), "{}", message);
    }

    #[cfg(windows)]
    #[test]
    fn a_file_open_without_write_sharing_fails_the_check() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = test_dir("check-locked");
        let dest = dir.join("doc.docx");
        std::fs::write(&dest, b"PK").unwrap();
        // FILE_SHARE_READ only, as a program that keeps the file open does.
        let holder = OpenOptions::new().read(true).share_mode(1).open(&dest).unwrap();

        let error = check_writable_destination(&dest).unwrap_err();

        assert!(error.starts_with(SAVE_LOCKED_ERROR), "{}", error);
        drop(holder);
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

        // Mode 020: write bits set, so not read-only, but none for the owner.
        // Before, opening it for write failed; replacing it by rename works.
        #[test]
        fn a_destination_the_owner_may_not_open_passes_the_check() {
            let dir = test_dir("check-no-open");
            let dest = dir.join("doc.docx");
            std::fs::write(&dest, b"PK").unwrap();
            set_mode(&dest, 0o020);
            assert!(!std::fs::metadata(&dest).unwrap().permissions().readonly());

            let result = check_writable_destination(&dest);

            set_mode(&dest, 0o644);
            let _ = std::fs::remove_dir_all(&dir);
            assert!(result.is_ok(), "{:?}", result);
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

    // A fake x2t: `convert_and_place` hands it the path to write, as the save
    // commands hand it to x2t.
    mod convert_and_place {
        use super::*;
        use crate::text_import::{finish_saved_text, TextKind, TextOptions, ENCODING_WINDOWS_1252};

        const OLD: &[u8] = b"PK\x03\x04 old document";
        const NEW: &[u8] = b"PK\x03\x04 new document";

        // A user folder holding `name` with OLD in it, and an empty instance
        // folder.
        fn user_and_instance(label: &str, name: &str) -> (PathBuf, PathBuf, PathBuf) {
            let user = test_dir(&format!("{}-user", label));
            let instance = test_dir(&format!("{}-instance", label));
            let dest = user.join(name);
            std::fs::write(&dest, OLD).unwrap();
            (user, instance, dest)
        }

        fn run(
            instance: &Path,
            dest: &Path,
            x2t: impl FnOnce(&Path) -> Result<String, String>,
            reshape: impl FnOnce(&Path),
        ) -> Result<(), String> {
            let staging = instance.join("save_output");
            let convert = |output: PathBuf| async move { x2t(&output) };
            tauri::async_runtime::block_on(convert_and_place(&staging, dest, convert, reshape))
        }

        fn no_reshape(_: &Path) {}

        fn half_then_crash(output: &Path) -> Result<String, String> {
            std::fs::write(output, &NEW[..6]).unwrap();
            Err("x2t conversion failed (exit code 1)".to_string())
        }

        fn nothing_but_exit_0(_: &Path) -> Result<String, String> {
            Ok("ok".to_string())
        }

        fn writes_new(output: &Path) -> Result<String, String> {
            std::fs::write(output, NEW).unwrap();
            Ok("ok".to_string())
        }

        fn clean_up(dirs: &[&Path]) {
            for dir in dirs {
                let _ = std::fs::remove_dir_all(dir);
            }
        }

        #[test]
        fn a_crash_halfway_leaves_dest_as_it_was() {
            let (user, instance, dest) = user_and_instance("crash", "doc.docx");

            let error = run(&instance, &dest, half_then_crash, no_reshape).unwrap_err();

            assert!(error.contains("exit code 1"), "{}", error);
            assert_eq!(std::fs::read(&dest).unwrap(), OLD);
            assert_eq!(names_in(&user), vec!["doc.docx"]);
            assert!(names_in(&instance).is_empty(), "{:?}", names_in(&instance));
            clean_up(&[&user, &instance]);
        }

        #[test]
        fn exit_0_with_nothing_written_is_an_error_and_dest_is_unchanged() {
            let (user, instance, dest) = user_and_instance("empty", "doc.xlsx");

            let error = run(&instance, &dest, nothing_but_exit_0, no_reshape).unwrap_err();

            assert!(error.contains("did not write"), "{}", error);
            assert_eq!(std::fs::read(&dest).unwrap(), OLD);
            assert_eq!(names_in(&user), vec!["doc.xlsx"]);
            assert!(names_in(&instance).is_empty(), "{:?}", names_in(&instance));
            clean_up(&[&user, &instance]);
        }

        #[test]
        fn x2t_never_writes_in_the_user_folder() {
            let (user, instance, dest) = user_and_instance("where", "doc.pptx");
            let mut written_to = PathBuf::new();

            let x2t = |output: &Path| {
                written_to = output.to_path_buf();
                writes_new(output)
            };
            run(&instance, &dest, x2t, no_reshape).unwrap();

            assert!(written_to.starts_with(&instance), "{:?}", written_to);
            assert_eq!(written_to.file_name(), dest.file_name());
            clean_up(&[&user, &instance]);
        }

        #[test]
        fn a_good_run_replaces_dest_and_leaves_no_temp_file() {
            let (user, instance, dest) = user_and_instance("good", "doc.odt");

            run(&instance, &dest, writes_new, no_reshape).unwrap();

            assert_eq!(std::fs::read(&dest).unwrap(), NEW);
            assert_eq!(names_in(&user), vec!["doc.odt"]);
            assert!(names_in(&instance).is_empty(), "{:?}", names_in(&instance));
            clean_up(&[&user, &instance]);
        }

        // What a save killed by a power cut left in the staging folder, x2t's
        // asc… scratch folder among it, goes on the next save.
        #[test]
        fn leftovers_of_a_killed_save_are_cleared() {
            let (user, instance, dest) = user_and_instance("leftover", "doc.pdf");
            std::fs::create_dir_all(instance.join("save_output").join("ascwfVgtc")).unwrap();

            run(&instance, &dest, writes_new, no_reshape).unwrap();

            assert!(names_in(&instance).is_empty(), "{:?}", names_in(&instance));
            clean_up(&[&user, &instance]);
        }

        #[cfg(unix)]
        #[test]
        fn a_symlinked_dest_stays_a_symlink_to_the_new_content() {
            let (user, instance, real) = user_and_instance("link", "real.docx");
            let link = user.join("doc.docx");
            std::os::unix::fs::symlink(&real, &link).unwrap();

            run(&instance, &link, writes_new, no_reshape).unwrap();

            let link_type = std::fs::symlink_metadata(&link).unwrap().file_type();
            assert!(link_type.is_symlink());
            assert_eq!(std::fs::read_link(&link).unwrap(), real);
            assert_eq!(std::fs::read(&real).unwrap(), NEW);
            assert_eq!(names_in(&user), vec!["doc.docx", "real.docx"]);
            clean_up(&[&user, &instance]);
        }

        // x2t writes TXT as UTF-8 with a BOM; the original was windows-1252
        // with CRLF. Re-encoding happens on the staged file, so `dest` goes
        // from the old bytes to the finished ones in one rename.
        #[test]
        fn a_txt_is_re_encoded_before_it_is_placed() {
            let (user, instance, dest) = user_and_instance("txt", "notes.txt");
            let x2t = |output: &Path| {
                std::fs::write(output, "\u{FEFF}caf\u{e9}\n".as_bytes()).unwrap();
                Ok("ok".to_string())
            };
            let options = TextOptions {
                encoding: ENCODING_WINDOWS_1252,
                delimiter: None,
                utf8_bom: false,
                crlf: true,
                trailing_breaks: 1,
            };
            let mut dest_while_reshaping = Vec::new();
            let reshape = |staged: &Path| {
                dest_while_reshaping = std::fs::read(&dest).unwrap();
                finish_saved_text(staged, TextKind::Txt, options).unwrap();
            };

            run(&instance, &dest, x2t, reshape).unwrap();

            assert_eq!(dest_while_reshaping, OLD);
            assert_eq!(std::fs::read(&dest).unwrap(), b"caf\xE9\r\n");
            assert_eq!(names_in(&user), vec!["notes.txt"]);
            assert!(names_in(&instance).is_empty(), "{:?}", names_in(&instance));
            clean_up(&[&user, &instance]);
        }
    }
}
