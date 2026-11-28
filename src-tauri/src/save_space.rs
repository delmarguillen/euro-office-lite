// Whether the disk of the instance folder has room for a save. x2t exits 0 on
// a full disk and leaves what it was writing truncated, so the room is checked
// before it runs and the disk again once it has.

use crate::converter::Volume;
use crate::save_target::SAVE_NO_SPACE_ERROR;
use std::path::Path;

const MIB: u64 = 1024 * 1024;

// x2t unpacks the document next to Editor.bin and builds the saved file from
// that. Measured with x2t 10.0.0.169 on Linux: at most 7.5 times the size of
// Editor.bin and its media, for a heavily formatted text saved as ODT.
const WORK_FACTOR: u64 = 10;

// An empty document takes 0.3 MB; the rest is the free space the disk keeps.
const WORK_BASE: u64 = 4 * MIB;

// A PDF export also writes a script cache of about 8 MB every time.
const PDF_WORK_BASE: u64 = 16 * MIB;

// A disk that filled up while x2t wrote is left with less than one block.
const FULL_DISK_FLOOR: u64 = MIB;

const PDF_FORMAT: i32 = 513;

// What a save may use of the disk it is staged on.
pub struct SaveSpace<'a> {
    // Bytes the save needs free before x2t runs.
    pub needed: u64,
    // The file system of a folder, None when that cannot be told.
    pub volume: &'a (dyn Fn(&Path) -> Option<Volume> + Sync),
}

impl SaveSpace<'_> {
    // Err when the disk of `dir` has less free than the save needs. A disk
    // whose free space cannot be told passes.
    pub fn check_room(&self, dir: &Path) -> Result<(), String> {
        let Some(disk) = (self.volume)(dir) else {
            return Ok(());
        };
        if disk.free >= self.needed {
            return Ok(());
        }
        let megabytes = self.needed.div_ceil(1_000_000);
        Err(no_space_error(dir, &format!("the save needs about {} MB free", megabytes)))
    }

    // Err when x2t left the disk of `dir` full: what it wrote there may be
    // truncated.
    pub fn check_not_filled(&self, dir: &Path) -> Result<(), String> {
        let Some(disk) = (self.volume)(dir) else {
            return Ok(());
        };
        if disk.free >= FULL_DISK_FLOOR {
            return Ok(());
        }
        Err(no_space_error(dir, "the disk filled up during the save"))
    }
}

// The free space a save of the document in `temp_dir` to `format_to` asks for.
pub fn needed_for_save(temp_dir: &Path, format_to: i32) -> u64 {
    let document = file_len(&temp_dir.join("Editor.bin")) + files_len(&temp_dir.join("media"));
    let base = match format_to {
        PDF_FORMAT => PDF_WORK_BASE,
        _ => WORK_BASE,
    };
    document.saturating_mul(WORK_FACTOR).saturating_add(base)
}

pub fn is_out_of_space(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(error.kind(), ErrorKind::StorageFull | ErrorKind::QuotaExceeded)
}

pub fn no_space_error(path: &Path, reason: &str) -> String {
    format!("{}: {}: {}", SAVE_NO_SPACE_ERROR, path.display(), reason)
}

// 0 for a file that is not there.
fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0)
}

// The files directly in `dir`, 0 for a folder that is not there.
fn files_len(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries.flatten().map(|entry| file_len(&entry.path())).sum()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    // A disk with `free` bytes, whatever the folder.
    pub fn disk_with(free: u64) -> impl Fn(&Path) -> Option<Volume> + Sync {
        move |_| Some(Volume { device: 1, free })
    }

    pub fn unknown_disk(_: &Path) -> Option<Volume> {
        None
    }

    fn temp_dir_with(label: &str, editor_bin: usize, media: &[usize]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("eo-save-space-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("media")).unwrap();
        std::fs::write(dir.join("Editor.bin"), vec![0u8; editor_bin]).unwrap();
        for (index, len) in media.iter().enumerate() {
            std::fs::write(dir.join("media").join(format!("image{}.png", index)), vec![0u8; *len]).unwrap();
        }
        dir
    }

    #[test]
    fn a_save_asks_for_ten_times_the_document_and_its_media_plus_the_base() {
        let dir = temp_dir_with("needed", 1000, &[200, 300]);

        assert_eq!(needed_for_save(&dir, 65), 15_000 + 4 * MIB);
        assert_eq!(needed_for_save(&dir, PDF_FORMAT), 15_000 + 16 * MIB);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_document_asks_for_the_base_only() {
        let dir = std::env::temp_dir().join(format!("eo-save-space-none-{}", std::process::id()));

        assert_eq!(needed_for_save(&dir, 257), 4 * MIB);
    }

    #[test]
    fn a_disk_with_less_than_the_save_needs_is_a_no_space_error() {
        let space = SaveSpace { needed: 5 * MIB, volume: &disk_with(5 * MIB - 1) };

        let error = space.check_room(Path::new("/tmp/instance/save_output/0")).unwrap_err();

        assert_eq!(
            error,
            "save-no-space: /tmp/instance/save_output/0: the save needs about 6 MB free"
        );
    }

    #[test]
    fn a_disk_with_exactly_what_the_save_needs_passes() {
        let space = SaveSpace { needed: 5 * MIB, volume: &disk_with(5 * MIB) };

        assert_eq!(space.check_room(Path::new("/tmp/instance")), Ok(()));
    }

    #[test]
    fn a_disk_whose_free_space_cannot_be_told_passes_both_checks() {
        let space = SaveSpace { needed: u64::MAX, volume: &unknown_disk };

        assert_eq!(space.check_room(Path::new("/tmp/instance")), Ok(()));
        assert_eq!(space.check_not_filled(Path::new("/tmp/instance")), Ok(()));
    }

    #[test]
    fn a_disk_left_with_less_than_the_floor_is_a_no_space_error() {
        let space = SaveSpace { needed: 0, volume: &disk_with(FULL_DISK_FLOOR - 1) };

        let error = space.check_not_filled(Path::new("/tmp/instance")).unwrap_err();

        assert_eq!(error, "save-no-space: /tmp/instance: the disk filled up during the save");
    }

    #[test]
    fn a_disk_left_with_the_floor_passes() {
        let space = SaveSpace { needed: 0, volume: &disk_with(FULL_DISK_FLOOR) };

        assert_eq!(space.check_not_filled(Path::new("/tmp/instance")), Ok(()));
    }

    #[test]
    fn a_full_disk_and_a_full_quota_are_out_of_space() {
        use std::io::{Error, ErrorKind};

        assert!(is_out_of_space(&Error::from(ErrorKind::StorageFull)));
        assert!(is_out_of_space(&Error::from(ErrorKind::QuotaExceeded)));
        assert!(!is_out_of_space(&Error::from(ErrorKind::PermissionDenied)));
    }
}
