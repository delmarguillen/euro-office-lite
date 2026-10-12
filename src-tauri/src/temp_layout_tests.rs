use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn scratch_root() -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "eo-temp-layout-test-{}-{}",
        std::process::id(),
        unique
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

// What opening a document does to the instance's folder, with x2t's output
// faked: the previous document's files go, then Editor.bin and the media/ x2t
// writes next to it.
fn simulate_open(dir: &Path, doc: &[u8], image: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    for sub in ["media", "insert_tmp", "downloads", "docx_unpacked"] {
        let _ = std::fs::remove_dir_all(dir.join(sub));
    }
    let _ = std::fs::remove_file(dir.join("Editor.bin"));
    std::fs::create_dir_all(dir.join("media")).unwrap();
    std::fs::write(dir.join("media/image1.png"), image).unwrap();
    std::fs::write(dir.join("Editor.bin"), doc).unwrap();
}

// --- one folder per instance ---------------------------------------------------

// Save As writes Editor.bin, waits for the dialog, then converts whatever is
// there. Another instance opening a document meanwhile must not change it.
#[test]
fn save_as_converts_its_own_document_when_another_instance_opens_meanwhile() {
    let root = scratch_root();
    let a = begin_instance(&root, 1_000, 1).unwrap();
    let b = begin_instance(&root, 1_000, 2).unwrap();
    simulate_open(&a.dir, b"doc A", b"image A");

    std::fs::write(a.dir.join("Editor.bin"), b"doc A edited").unwrap();
    simulate_open(&b.dir, b"doc B", b"image B");

    assert_eq!(std::fs::read(a.dir.join("Editor.bin")).unwrap(), b"doc A edited");
    let _ = std::fs::remove_dir_all(&root);
}

// x2t resolves the images of a save from the media/ next to Editor.bin.
#[test]
fn another_instance_opening_keeps_this_instances_images() {
    let root = scratch_root();
    let a = begin_instance(&root, 1_000, 1).unwrap();
    let b = begin_instance(&root, 1_000, 2).unwrap();
    simulate_open(&a.dir, b"doc A", b"image A");
    simulate_open(&b.dir, b"doc B", b"image B");

    assert_eq!(std::fs::read(a.dir.join("media/image1.png")).unwrap(), b"image A");
    let _ = std::fs::remove_dir_all(&root);
}

// x2t names its unpack folder after the extension only, and every open clears
// those folders: an open in one instance would pull the folder out from under
// a conversion running in another.
#[test]
fn another_instance_opening_keeps_this_instances_x2t_scratch() {
    let root = scratch_root();
    let a = begin_instance(&root, 1_000, 1).unwrap();
    let b = begin_instance(&root, 1_000, 2).unwrap();
    simulate_open(&a.dir, b"doc A", b"image A");
    std::fs::create_dir_all(a.dir.join("docx_unpacked")).unwrap();
    simulate_open(&b.dir, b"doc B", b"image B");

    assert!(a.dir.join("docx_unpacked").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn two_starts_in_the_same_millisecond_get_different_folders() {
    let root = scratch_root();
    let a = begin_instance(&root, 42, 1).unwrap();
    let b = begin_instance(&root, 42, 2).unwrap();

    assert_ne!(a.dir, b.dir);
    let _ = std::fs::remove_dir_all(&root);
}

// A normal exit leaves nothing behind; only a killed process does.
#[test]
fn ending_an_instance_removes_its_folder() {
    let root = scratch_root();
    let a = begin_instance(&root, now(), 1).unwrap();
    std::fs::create_dir_all(a.dir.join("media")).unwrap();
    std::fs::write(a.dir.join("media/image1.png"), b"img").unwrap();

    a.end();

    assert!(!a.dir.exists());
    let _ = std::fs::remove_dir_all(&root);
}

// --- removing what dead instances left ------------------------------------------

fn make_folder(root: &Path, id: &str, with_lock_file: bool) {
    let dir = root.join(INSTANCES_DIR).join(id);
    std::fs::create_dir_all(dir.join("media")).unwrap();
    std::fs::write(dir.join("media/image1.png"), b"img").unwrap();
    if with_lock_file {
        std::fs::write(dir.join(INSTANCE_LOCK), b"").unwrap();
    }
}

#[test]
fn the_sweep_keeps_live_young_referenced_and_foreign_folders() {
    let root = scratch_root();
    let n = now();
    let live = format!("{}-11", n - 5 * 60_000);
    let dead = format!("{}-12", n - 5 * 60_000);
    let young_unlocked = format!("{}-13", n - 5 * 60_000);
    let old_unlocked = format!("{}-14", n - UNLOCKED_GRACE_MS - 1);
    let referenced = format!("{}-15", n - 5 * 60_000);
    make_folder(&root, &live, true);
    make_folder(&root, &dead, true);
    make_folder(&root, &young_unlocked, false);
    make_folder(&root, &old_unlocked, false);
    make_folder(&root, &referenced, true);
    std::fs::create_dir_all(root.join(INSTANCES_DIR).join("not-an-id")).unwrap();
    // What older versions kept in the root itself.
    std::fs::create_dir_all(root.join("media")).unwrap();
    std::fs::write(root.join("Editor.bin"), b"older version's document").unwrap();

    let refs: HashSet<String> = [referenced.clone()].into_iter().collect();
    let live_lock = root.join(INSTANCES_DIR).join(&live).join(INSTANCE_LOCK);
    let mut removed = sweep_dead_instances(&root, n, &refs, |path| path == live_lock);
    removed.sort();

    let mut expected = vec![dead.clone(), old_unlocked.clone()];
    expected.sort();
    assert_eq!(removed, expected);
    for kept in [&live, &young_unlocked, &referenced] {
        assert!(root.join(INSTANCES_DIR).join(kept).exists(), "{} must stay", kept);
    }
    assert!(root.join(INSTANCES_DIR).join("not-an-id").exists());
    assert!(root.join("media").exists());
    assert!(root.join("Editor.bin").exists());
    let _ = std::fs::remove_dir_all(&root);
}

// On Unix the lock file exists a moment before it is locked, so a starting
// instance can look like a dead one. Nothing that young is touched.
#[test]
fn a_folder_younger_than_the_minimum_age_is_never_removed() {
    let root = scratch_root();
    let n = now();
    let starting = format!("{}-41", n - 5_000);
    let dead = format!("{}-42", n - MIN_SWEEP_AGE_MS - 1);
    make_folder(&root, &starting, true);
    make_folder(&root, &dead, true);

    let removed = sweep_dead_instances(&root, n, &HashSet::new(), |_| false);

    assert_eq!(removed, vec![dead]);
    assert!(root.join(INSTANCES_DIR).join(&starting).exists());
    let _ = std::fs::remove_dir_all(&root);
}

// With real OS locks: a held lock protects the folder, a released one (what
// the OS does when a process dies, kill included) does not.
#[test]
fn real_locks_tell_a_live_instance_from_a_dead_one() {
    let root = scratch_root();
    let n = now();
    let alive = begin_instance(&root, n - 2 * 60_000, 21).unwrap();
    let gone = begin_instance(&root, n - 2 * 60_000, 22).unwrap();
    let gone_id = gone.id.clone();
    gone.lock.lock().unwrap().take();

    let removed = sweep_dead_instances(&root, n, &HashSet::new(), |path| is_locked(path));

    assert_eq!(removed, vec![gone_id]);
    assert!(instance_alive(&root, &alive.id));
    assert!(alive.dir.exists());
    alive.end();
    let _ = std::fs::remove_dir_all(&root);
}

// The three lock tests below come from recovery.rs along with the primitives.

#[test]
fn an_orphan_lock_file_does_not_read_as_locked() {
    let root = scratch_root();
    let path = root.join(INSTANCE_LOCK);
    std::fs::write(&path, b"").unwrap();

    assert!(!is_locked(&path));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_held_lock_reads_as_locked_and_frees_when_dropped() {
    let root = scratch_root();
    let path = root.join(INSTANCE_LOCK);

    let held = take_lock(&path).unwrap();
    assert!(is_locked(&path));
    drop(held);
    assert!(!is_locked(&path));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_missing_lock_file_is_not_created_by_the_check() {
    let root = scratch_root();
    let path = root.join(INSTANCE_LOCK);

    assert!(!is_locked(&path));
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&root);
}

// --- recovery images -------------------------------------------------------------

// A crashed document's images are in the folder of the instance that had it
// open, and the instance recovering it has to bring them into its own.
#[test]
fn recovery_brings_the_crashed_instances_images_along() {
    let root = scratch_root();
    let crashed = begin_instance(&root, 1_000, 31).unwrap();
    std::fs::create_dir_all(crashed.dir.join("media")).unwrap();
    std::fs::write(crashed.dir.join("media/image1.png"), b"original").unwrap();
    std::fs::write(crashed.dir.join("media/clipboard_1.png"), b"pasted").unwrap();
    let recovering = begin_instance(&root, 2_000, 32).unwrap();

    let src = recovery_media_source(&root, Some(crashed.id.as_str()));
    let copied = copy_media(&src, &recovering.dir.join("media")).unwrap();

    assert_eq!(copied, 2);
    assert_eq!(
        std::fs::read(recovering.dir.join("media/image1.png")).unwrap(),
        b"original"
    );
    assert_eq!(
        std::fs::read(recovering.dir.join("media/clipboard_1.png")).unwrap(),
        b"pasted"
    );
    assert!(
        crashed.dir.join("media/image1.png").exists(),
        "copied, not moved"
    );

    // Recovery folders written before instances existed name none, and their
    // images are where every instance used to keep them. Only ids that
    // begin_instance makes are joined onto the root.
    assert_eq!(recovery_media_source(&root, None), root.join("media"));
    assert_eq!(
        recovery_media_source(&root, Some("../../elsewhere")),
        root.join("media")
    );
    assert_eq!(copy_media(&root.join("gone"), &root.join("other")).unwrap(), 0);
    let _ = std::fs::remove_dir_all(&root);
}

// --- logs ----------------------------------------------------------------------

// One shared log was emptied by every start and then written by every process
// at once.
#[test]
fn each_instance_logs_to_its_own_file() {
    let root = scratch_root();

    assert_ne!(log_path(&root, "1000-1"), log_path(&root, "1000-2"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn starting_a_log_never_empties_another_instances() {
    let root = scratch_root();
    let first = log_path(&root, "1000-1");
    std::fs::write(&first, "[STARTUP] first\n[OPEN] success\n").unwrap();

    let second = start_log(&root, "1000-2", "[STARTUP] second");

    assert!(std::fs::read_to_string(&first).unwrap().contains("[OPEN] success"));
    assert!(std::fs::read_to_string(&second).unwrap().contains("[STARTUP] second"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn logs_of_live_instances_and_the_newest_dead_ones_are_kept() {
    let root = scratch_root();
    for i in 0..15u64 {
        std::fs::write(log_path(&root, &format!("{}-7", 1_000 + i)), b"log").unwrap();
    }
    std::fs::write(root.join("js-debug.log"), b"older version").unwrap();
    std::fs::write(root.join("prepare-dist.log"), b"build").unwrap();
    let live = "1000-7".to_string();

    let removed = prune_logs(&root, LOGS_KEPT, |id| id == live);

    assert_eq!(removed.len(), 4, "15 logs, 1 live, the 10 newest dead ones kept");
    assert!(log_path(&root, &live).exists(), "the oldest is live, so it stays");
    assert!(log_path(&root, "1014-7").exists());
    assert!(!log_path(&root, "1001-7").exists());
    assert!(root.join("js-debug.log").exists());
    assert!(root.join("prepare-dist.log").exists());
    let _ = std::fs::remove_dir_all(&root);
}
