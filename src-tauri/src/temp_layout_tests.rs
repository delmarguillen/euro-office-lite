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

// --- shared font cache -----------------------------------------------------------

fn write_fonts(dir: &Path, content: &[u8]) {
    std::fs::write(dir.join("AllFonts.js"), content).unwrap();
    std::fs::write(dir.join("font_selection.bin"), content).unwrap();
}

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const POLL: std::time::Duration = std::time::Duration::from_millis(20);

// Another instance may be serving AllFonts.js while this one regenerates it,
// so the generator works elsewhere and the result replaces the cache whole.
#[test]
fn the_generator_never_writes_into_the_live_cache() {
    let root = scratch_root();
    let live = fontdata_dir(&root);
    std::fs::create_dir_all(&live).unwrap();
    write_fonts(&live, b"old");
    let reader = File::open(live.join("AllFonts.js")).unwrap();

    let outcome = ensure_font_cache(
        &root,
        "1-1",
        |dir: &Path| {
            assert_ne!(dir, live.as_path(), "the generator got the live cache");
            write_fonts(dir, b"new");
            true
        },
        WAIT,
        POLL,
    );
    drop(reader);

    assert_eq!(outcome, FontCache::Generated);
    assert_eq!(std::fs::read(live.join("AllFonts.js")).unwrap(), b"new");
    assert_eq!(
        ensure_font_cache(&root, "1-2", |_: &Path| panic!("built twice"), WAIT, POLL),
        FontCache::Ready
    );
    let _ = std::fs::remove_dir_all(&root);
}

// A run that fails must not leave half a cache behind for the next start to
// take as good.
#[test]
fn a_failed_generation_leaves_the_cache_as_it_was() {
    let root = scratch_root();
    let live = fontdata_dir(&root);
    std::fs::create_dir_all(&live).unwrap();
    write_fonts(&live, b"old");

    let outcome = ensure_font_cache(
        &root,
        "1-1",
        |dir: &Path| {
            std::fs::write(dir.join("AllFonts.js"), b"half").unwrap();
            false
        },
        WAIT,
        POLL,
    );

    assert_eq!(outcome, FontCache::Failed);
    assert_eq!(std::fs::read(live.join("AllFonts.js")).unwrap(), b"old");
    assert!(!fonts_ready(&root));
    let _ = std::fs::remove_dir_all(&root);
}

// A second start while the first is generating waits for that run instead of
// writing the same files at the same time.
#[test]
fn a_start_during_generation_waits_and_does_not_generate_again() {
    let root = scratch_root();
    let held = take_lock(&root.join(FONTS_LOCK)).unwrap();
    let other_root = root.clone();
    let other = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let live = fontdata_dir(&other_root);
        std::fs::create_dir_all(&live).unwrap();
        write_fonts(&live, b"theirs");
        std::fs::write(other_root.join(FONTS_MARKER), b"generated").unwrap();
        drop(held);
    });

    let outcome = ensure_font_cache(&root, "1-2", |_: &Path| panic!("built twice"), WAIT, POLL);
    other.join().unwrap();

    assert_eq!(outcome, FontCache::GeneratedByOther);
    assert_eq!(std::fs::read(fontdata_dir(&root).join("AllFonts.js")).unwrap(), b"theirs");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_generator_that_never_finishes_is_not_waited_on_forever() {
    let root = scratch_root();
    let _held = take_lock(&root.join(FONTS_LOCK)).unwrap();

    let outcome = ensure_font_cache(
        &root,
        "1-2",
        |_: &Path| panic!("built while another instance holds the lock"),
        std::time::Duration::from_millis(200),
        POLL,
    );

    assert_eq!(outcome, FontCache::GaveUp);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_dead_generators_staging_folder_is_cleared() {
    let root = scratch_root();
    let leftover = root.join(format!("{}1-9", FONTS_STAGING_PREFIX));
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("AllFonts.js"), b"half").unwrap();

    ensure_font_cache(
        &root,
        "2-1",
        |dir: &Path| {
            write_fonts(dir, b"new");
            true
        },
        WAIT,
        POLL,
    );

    assert!(!leftover.exists());
    let _ = std::fs::remove_dir_all(&root);
}

// --- the root on Linux and macOS ---------------------------------------------------
//
// Only compiled on Unix. A folder owned by another user cannot be made here
// without root, so that case is left to the test machines.

#[cfg(unix)]
mod private_root {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn uid() -> u32 {
        unsafe { libc::getuid() }
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_missing_root_is_created_private() {
        let base = scratch_root();
        let root = base.join("root");

        assert_eq!(ensure_private_dir(&root, uid()), Ok(()));
        assert_eq!(mode(&root), 0o700);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_root_open_to_others_is_not_used() {
        let base = scratch_root();
        let root = base.join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(ensure_private_dir(&root, uid()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_link_in_place_of_the_root_is_not_used() {
        let base = scratch_root();
        let real = base.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = base.join("root");
        std::os::unix::fs::symlink(&real, &root).unwrap();

        assert!(ensure_private_dir(&root, uid()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_file_in_place_of_the_root_is_not_used() {
        let base = scratch_root();
        let root = base.join("root");
        std::fs::write(&root, b"").unwrap();

        assert!(ensure_private_dir(&root, uid()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    // Same check with a uid that is not ours stands in for a folder another
    // user created.
    #[test]
    fn a_root_owned_by_someone_else_is_not_used() {
        let base = scratch_root();
        let root = base.join("root");

        assert!(ensure_private_dir(&root, uid().wrapping_add(1)).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_next_candidate_is_used_when_the_first_is_not_private() {
        let base = scratch_root();
        let first = base.join("first");
        std::fs::create_dir(&first).unwrap();
        std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o755)).unwrap();
        let second = base.join("cache").join("app").join("tmp");

        let choice = choose_root(&[first.clone(), second.clone()], &[], uid(), || {
            "fresh".to_string()
        })
        .unwrap();

        assert_eq!(choice.path, second);
        assert_eq!(mode(&second), 0o700);
        assert!(choice.note.is_some(), "the skipped folder is reported");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_fresh_folder_is_made_when_no_candidate_is_private() {
        let base = scratch_root();
        let first = base.join("first");
        std::fs::write(&first, b"").unwrap();
        let cache = base.join("cache");

        let choice = choose_root(&[first], &[cache.clone(), base.clone()], uid(), || {
            "fresh".to_string()
        })
        .unwrap();

        assert_eq!(choice.path, cache.join("fresh"), "the user's cache folder comes first");
        assert_eq!(mode(&choice.path), 0o700);
        assert!(choice.note.unwrap().contains("new folder"));
        let _ = std::fs::remove_dir_all(&base);
    }

    // A name that is already taken is never used as the new folder, whoever
    // made it, and when nothing else is left there is no folder at all
    // rather than an unchecked one.
    #[test]
    fn no_folder_that_existed_before_is_taken_as_a_new_one() {
        let base = scratch_root();
        let first = base.join("first");
        std::fs::write(&first, b"").unwrap();
        let taken = base.join("taken");
        std::fs::create_dir(&taken).unwrap();
        std::fs::set_permissions(&taken, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mine = base.join("mine");
        std::fs::create_dir(&mine).unwrap();
        std::fs::set_permissions(&mine, std::fs::Permissions::from_mode(0o700)).unwrap();

        let result = choose_root(&[first], &[base.clone()], uid(), || "taken".to_string());
        assert!(result.is_err());

        let result = choose_root(&[], &[base.clone()], uid(), || "mine".to_string());
        assert!(result.is_err(), "even a private folder of ours is not new");
        assert_eq!(mode(&taken), 0o755, "left as it was");
        let _ = std::fs::remove_dir_all(&base);
    }
}
