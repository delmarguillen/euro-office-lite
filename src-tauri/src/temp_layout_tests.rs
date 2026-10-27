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
        std::fs::write(other_root.join(FONTS_MARKER), expected_marker()).unwrap();
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

fn root_with_marker(marker: &[u8]) -> PathBuf {
    let root = scratch_root();
    let live = fontdata_dir(&root);
    std::fs::create_dir_all(&live).unwrap();
    write_fonts(&live, b"cache");
    std::fs::write(root.join(FONTS_MARKER), marker).unwrap();
    root
}

#[test]
fn a_cache_built_by_another_version_is_not_ready() {
    let root = root_with_marker(b"0.17.41-alpha");

    assert!(!fonts_ready_for(&root, "0.17.42-alpha"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_cache_from_before_markers_held_the_version_is_not_ready() {
    let root = root_with_marker(b"generated");

    assert!(!fonts_ready_for(&root, "0.17.42-alpha"));
    let _ = std::fs::remove_dir_all(&root);
}

// A cell of its own: the app-wide one can be set only once per process.
#[test]
fn the_version_set_at_startup_is_the_one_the_marker_records() {
    let cell = OnceLock::new();
    assert_eq!(stored_version(&cell), "unset");

    store_version(&cell, "0.17.42-alpha".to_string());
    store_version(&cell, "0.17.43-alpha".to_string());

    assert_eq!(stored_version(&cell), "0.17.42-alpha");
}

#[test]
fn a_cache_built_by_this_version_is_ready() {
    let root = root_with_marker(b"0.17.42-alpha");

    assert!(fonts_ready_for(&root, "0.17.42-alpha"));
    let _ = std::fs::remove_dir_all(&root);
}

// After an update the old cache is rebuilt once, and the new marker makes it
// ready for the next start.
#[test]
fn a_cache_from_an_older_version_is_rebuilt_once() {
    let root = root_with_marker(b"generated");

    let outcome = ensure_font_cache(
        &root,
        "1-1",
        |dir: &Path| {
            write_fonts(dir, b"new");
            true
        },
        WAIT,
        POLL,
    );

    assert_eq!(outcome, FontCache::Generated);
    let marker = std::fs::read(root.join(FONTS_MARKER)).unwrap();
    assert_eq!(marker, expected_marker().as_bytes());
    assert_eq!(
        ensure_font_cache(&root, "1-2", |_: &Path| panic!("built twice"), WAIT, POLL),
        FontCache::Ready
    );
    let _ = std::fs::remove_dir_all(&root);
}

// The app can die after the new files land but before the marker is written:
// the cache must not pass for ready, and the next start builds it again.
#[test]
fn new_files_next_to_the_previous_versions_marker_are_rebuilt() {
    let root = root_with_marker(b"0.17.41-alpha");
    write_fonts(&fontdata_dir(&root), b"new, marker not written yet");

    assert!(!fonts_ready_for(&root, "0.17.42-alpha"));
    assert!(!fonts_ready(&root));
    let outcome = ensure_font_cache(
        &root,
        "1-1",
        |dir: &Path| {
            write_fonts(dir, b"rebuilt");
            true
        },
        WAIT,
        POLL,
    );

    assert_eq!(outcome, FontCache::Generated);
    let allfonts = std::fs::read(fontdata_dir(&root).join("AllFonts.js")).unwrap();
    assert_eq!(allfonts, b"rebuilt");
    let _ = std::fs::remove_dir_all(&root);
}

// What x2t writes for one bundled font when the scan gets the \\?\ form of
// the fonts folder, and when it gets the plain one.
const ALLFONTS_EXTENDED: &[u8] =
    b"window[\"__fonts_files\"] = [\"/?/C:/App/binaries/fonts/LiberationSerif-Regular.ttf\"];";
const ALLFONTS_PLAIN: &[u8] =
    b"window[\"__fonts_files\"] = [\"C:/App/binaries/fonts/LiberationSerif-Regular.ttf\"];";
const SELECTION_EXTENDED: &[u8] =
    b"\x10Liberation Serif\\\\?\\C:\\App\\binaries\\fonts\\LiberationSerif-Regular.ttf";
const SELECTION_PLAIN: &[u8] =
    b"\x10Liberation Serif C:\\App\\binaries\\fonts\\LiberationSerif-Regular.ttf";

fn write_cache(dir: &Path, allfonts: &[u8], selection: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("AllFonts.js"), allfonts).unwrap();
    std::fs::write(dir.join("font_selection.bin"), selection).unwrap();
}

#[test]
fn extended_paths_are_found_in_either_cache_file() {
    let root = scratch_root();
    let live = fontdata_dir(&root);

    write_cache(&live, ALLFONTS_EXTENDED, SELECTION_PLAIN);
    assert!(has_extended_paths(&live), "AllFonts.js lists /?/C:/...");
    write_cache(&live, ALLFONTS_PLAIN, SELECTION_EXTENDED);
    assert!(
        has_extended_paths(&live),
        "font_selection.bin lists \\\\?\\C:\\..."
    );
    write_cache(&live, ALLFONTS_PLAIN, SELECTION_PLAIN);
    assert!(
        !has_extended_paths(&live),
        "a cache with plain paths is good"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// A cache an earlier version left with \\?\ paths makes the PDF export swap
// the bundled fonts for another one, so it is rebuilt, and only once.
#[cfg(target_os = "windows")]
#[test]
fn a_windows_cache_with_extended_paths_is_rebuilt_once() {
    let root = scratch_root();
    let live = fontdata_dir(&root);
    write_cache(&live, ALLFONTS_EXTENDED, SELECTION_EXTENDED);
    std::fs::write(root.join(FONTS_MARKER), expected_marker()).unwrap();

    let outcome = ensure_font_cache(
        &root,
        "1-1",
        |dir: &Path| {
            write_cache(dir, ALLFONTS_PLAIN, SELECTION_PLAIN);
            true
        },
        WAIT,
        POLL,
    );

    assert_eq!(outcome, FontCache::Generated);
    assert_eq!(
        std::fs::read(live.join("AllFonts.js")).unwrap(),
        ALLFONTS_PLAIN
    );
    assert_eq!(
        std::fs::read(live.join("font_selection.bin")).unwrap(),
        SELECTION_PLAIN
    );
    assert_eq!(
        ensure_font_cache(&root, "1-2", |_: &Path| panic!("built twice"), WAIT, POLL),
        FontCache::Ready
    );
    let _ = std::fs::remove_dir_all(&root);
}

// --- images pasted from another document ----------------------------------------

const ORIGIN_IMAGE: &[u8] = b"origin: four coloured quadrants";
const DEST_IMAGE: &[u8] = b"destination: plain purple";

// What open_file does to an instance's folder: the outgoing document's images
// leave media/, then x2t writes the next document's. Returns the new number.
fn open_doc(dir: &Path, seq: u64, images: &[(&str, &[u8])]) -> u64 {
    begin_document(dir, seq, seq + 1).unwrap();
    let media = dir.join("media");
    std::fs::create_dir_all(&media).unwrap();
    for (name, bytes) in images {
        std::fs::write(media.join(name), bytes).unwrap();
    }
    seq + 1
}

fn started(root: &Path, millis: u64, pid: u32) -> Instance {
    let instance = begin_instance(root, millis, pid).unwrap();
    write_doc_seq(&instance.dir, 1).unwrap();
    instance
}

// The address a copy writes into the clipboard (bridge documentUrl + sdkjs).
fn image_url(id: &str, seq: u64, name: &str) -> String {
    format!("ascdesktop://docmedia/{}/media/{}", doc_key(id, seq), name)
}

fn paste(root: &Path, instance: &Instance, seq: u64, url: &str) -> Option<String> {
    let doc = OpenDocument { root, id: &instance.id, dir: &instance.dir, seq };
    import_media(&doc, url, crate::file_ops::stage_bytes_into_media)
}

fn media_file(dir: &Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join("media").join(name)).ok()
}

// Same window: copy in one document, open another with its own image1, paste.
#[test]
fn an_image_of_the_previous_document_is_copied_in_beside_this_ones() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let first = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);
    let copied = image_url(&a.id, first, "image1.png");
    let second = open_doc(&a.dir, first, &[("image1.png", DEST_IMAGE)]);

    let name = paste(&root, &a, second, &copied).expect("pasted image");
    assert_eq!(media_file(&a.dir, &name).as_deref(), Some(ORIGIN_IMAGE));
    assert_eq!(media_file(&a.dir, "image1.png").as_deref(), Some(DEST_IMAGE));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_image_of_the_previous_document_reaches_a_document_without_images() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let first = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);
    let copied = image_url(&a.id, first, "image1.png");
    let second = open_doc(&a.dir, first, &[]);

    let name = paste(&root, &a, second, &copied).expect("pasted image");
    assert_eq!(media_file(&a.dir, &name).as_deref(), Some(ORIGIN_IMAGE));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_image_of_another_open_instance_is_copied_in() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let b = started(&root, 1_000, 2);
    let a_seq = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);
    let b_seq = open_doc(&b.dir, 1, &[("image1.png", DEST_IMAGE)]);

    let name = paste(&root, &b, b_seq, &image_url(&a.id, a_seq, "image1.png")).expect("pasted image");
    assert_eq!(media_file(&b.dir, &name).as_deref(), Some(ORIGIN_IMAGE));
    assert_eq!(media_file(&b.dir, "image1.png").as_deref(), Some(DEST_IMAGE));
    assert_eq!(media_file(&a.dir, "image1.png").as_deref(), Some(ORIGIN_IMAGE));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_image_of_an_instance_that_opened_another_document_since_is_still_found() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let b = started(&root, 1_000, 2);
    let a_first = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);
    let copied = image_url(&a.id, a_first, "image1.png");
    open_doc(&a.dir, a_first, &[("image1.png", b"a later document")]);
    let b_seq = open_doc(&b.dir, 1, &[]);

    let name = paste(&root, &b, b_seq, &copied).expect("pasted image");
    assert_eq!(media_file(&b.dir, &name).as_deref(), Some(ORIGIN_IMAGE));
    let _ = std::fs::remove_dir_all(&root);
}

// The source switches documents between reading its media/ and checking the
// number again: the next document's file under the same name must not leak.
#[test]
fn a_document_switch_mid_read_never_hands_out_the_next_documents_image() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let first = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);
    let image = parse_media_path(&image_url(&a.id, first, "image1.png")).expect("address");

    let bytes = read_document_image(&root, &image, || {
        open_doc(&a.dir, first, &[("image1.png", b"the next document")]);
    });
    assert_eq!(bytes.as_deref(), Some(ORIGIN_IMAGE));
    let _ = std::fs::remove_dir_all(&root);
}

// The source instance was closed: absent, never this document's image1.
#[test]
fn an_image_whose_source_is_gone_is_not_resolved_to_one_of_ours() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let b = started(&root, 1_000, 2);
    let a_seq = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);
    let copied = image_url(&a.id, a_seq, "image1.png");
    a.end();
    let b_seq = open_doc(&b.dir, 1, &[("image1.png", DEST_IMAGE)]);

    assert_eq!(paste(&root, &b, b_seq, &copied), None);
    assert_eq!(media_file(&b.dir, "image1.png").as_deref(), Some(DEST_IMAGE));
    let _ = std::fs::remove_dir_all(&root);
}

// Copy and paste inside one document: same name, nothing copied.
#[test]
fn pasting_within_the_open_document_copies_nothing() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let seq = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);

    let name = paste(&root, &a, seq, &image_url(&a.id, seq, "image1.png"));
    assert_eq!(name.as_deref(), Some("image1.png"));
    assert_eq!(std::fs::read_dir(a.dir.join("media")).unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_the_open_documents_images_are_served() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let b = started(&root, 1_000, 2);
    let a_seq = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);
    let b_seq = open_doc(&b.dir, 1, &[("image1.png", DEST_IMAGE)]);
    let b_doc = OpenDocument { root: &root, id: &b.id, dir: &b.dir, seq: b_seq };

    let own = format!("docmedia/{}/media/image1.png", doc_key(&b.id, b_seq));
    assert_eq!(serve_media_path(&b_doc, &own), Some(b.dir.join("media").join("image1.png")));
    let foreign = format!("docmedia/{}/media/image1.png", doc_key(&a.id, a_seq));
    assert_eq!(serve_media_path(&b_doc, &foreign), None);
    let earlier = format!("docmedia/{}/media/image1.png", doc_key(&b.id, b_seq - 1));
    assert_eq!(serve_media_path(&b_doc, &earlier), None);
    // The address every document had before, still in older clipboards.
    assert_eq!(serve_media_path(&b_doc, "docmedia/media/image1.png"), None);
    let _ = std::fs::remove_dir_all(&root);
}

// The request URIs as each webview delivers them to the protocol handler.
#[test]
fn the_doc_key_request_is_recognised_in_every_form_it_arrives() {
    for uri in [
        "ascdesktop://doc-key/",
        "ascdesktop://doc-key",
        "ascdesktop://localhost/doc-key",
        "http://ascdesktop.localhost/doc-key",
    ] {
        assert!(is_doc_key_path(protocol_path(uri)), "{}", uri);
    }
    for uri in ["ascdesktop://doc-keys/", "ascdesktop://docmedia/doc-key", "ascdesktop://doc-key/x"] {
        assert!(!is_doc_key_path(protocol_path(uri)), "{}", uri);
    }
    assert_eq!(
        protocol_path("http://ascdesktop.localhost/docmedia/1000-1/2/media/image1.png"),
        "docmedia/1000-1/2/media/image1.png"
    );
    assert_eq!(protocol_path("ascdesktop://import-media/x"), "import-media/x");
}

#[test]
fn media_addresses_that_leave_the_folder_are_rejected() {
    for path in [
        "docmedia/1000-1/2/media/../Editor.bin",
        "docmedia/1000-1/2/media/..",
        "docmedia/1000-1/2/media/a/b.png",
        "docmedia/1000-1/2/media/a\\b.png",
        "docmedia/../1000-1/2/media/image1.png",
        "docmedia/1000-1/x/media/image1.png",
        "docmedia/1000-1/2/other/image1.png",
        "docmedia/1000-1/2/media/",
        "docmedia/media/image1.png",
    ] {
        assert_eq!(parse_media_path(path), None, "{}", path);
    }
    let image = |seq: u64| MediaRef { instance: "1000-1".into(), seq, name: "image1.png".into() };
    assert_eq!(
        parse_media_path("docmedia/1000-1/2/media/ascdesktop://docmedia/1000-1/3/media/image1.png"),
        Some(image(3))
    );
    assert_eq!(parse_media_path("docmedia/1000-1/2/media/media/image1.png"), Some(image(2)));
}

#[test]
fn only_the_last_three_previous_documents_are_kept() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let mut seq = 1;
    for i in 0..6u8 {
        seq = open_doc(&a.dir, seq, &[("image1.png", &[i][..])]);
    }
    let mut kept: Vec<u64> = std::fs::read_dir(a.dir.join("previous"))
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().and_then(|name| name.parse().ok()))
        .collect();
    kept.sort();
    assert_eq!(kept, vec![seq - 3, seq - 2, seq - 1]);
    let _ = std::fs::remove_dir_all(&root);
}

fn fail_rename(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "file in use"))
}

// When the images cannot be moved aside they are cleared as before: a later
// paste from that document is absent, the next document starts clean.
#[test]
fn images_that_cannot_be_moved_aside_are_cleared_as_before() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let seq = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE)]);

    let outcome = begin_document_with(&a.dir, seq, seq + 1, fail_rename, |path| std::fs::remove_dir_all(path));
    assert!(matches!(outcome, Ok(OutgoingImages::Dropped(_))), "{:?}", outcome);
    assert!(!a.dir.join("media").exists());
    assert_eq!(read_doc_seq(&a.dir), Some(seq + 1));
    let _ = std::fs::remove_dir_all(&root);
}

// If they cannot be cleared either, the open stops: two documents' images must
// never share media/.
#[test]
fn a_partial_clear_stops_the_open_instead_of_mixing_two_documents() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    let seq = open_doc(&a.dir, 1, &[("image1.png", ORIGIN_IMAGE), ("image2.png", ORIGIN_IMAGE)]);

    let outcome = begin_document_with(&a.dir, seq, seq + 1, fail_rename, |path| {
        std::fs::remove_file(path.join("image1.png"))?;
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "image2.png in use"))
    });
    assert!(outcome.is_err(), "{:?}", outcome);
    assert_eq!(read_doc_seq(&a.dir), Some(seq), "the open document did not change");
    let _ = std::fs::remove_dir_all(&root);
}

// A media/ that cannot even be listed is not known to be empty: when moving and
// removing it both fail, the open stops as it does for leftovers. Here it
// cannot be listed because it is a file, which read_dir refuses on every OS.
#[test]
fn an_unlistable_media_folder_stops_the_open_instead_of_counting_as_empty() {
    let root = scratch_root();
    let a = started(&root, 1_000, 1);
    std::fs::write(a.dir.join("media"), ORIGIN_IMAGE).unwrap();

    let outcome = begin_document_with(&a.dir, 1, 2, fail_rename, |_| {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "in use"))
    });
    assert!(outcome.is_err(), "{:?}", outcome);
    assert_eq!(read_doc_seq(&a.dir), Some(1), "the open document did not change");
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

// ---------------------------------------------------------------------------
// Font cache root
// ---------------------------------------------------------------------------

fn os(value: &str) -> Option<std::ffi::OsString> {
    Some(std::ffi::OsString::from(value))
}

#[test]
fn the_font_cache_goes_in_each_platforms_user_cache_folder() {
    let id = "org.euro-office.lite";
    let linux = linux_cache_dir(None, os("/home/ana")).unwrap();
    let xdg = linux_cache_dir(os("/srv/cache"), os("/home/ana")).unwrap();
    let flatpak = linux_cache_dir(
        os("/home/ana/.var/app/org.euro_office.Lite/cache"),
        os("/home/ana"),
    )
    .unwrap();
    let macos = macos_cache_dir(os("/Users/ana")).unwrap();
    let windows = windows_cache_dir(os(r"C:\Users\ana\AppData\Local")).unwrap();

    assert_eq!(font_cache_path(&linux, id), Path::new("/home/ana/.cache/org.euro-office.lite/fonts"));
    assert_eq!(font_cache_path(&xdg, id), Path::new("/srv/cache/org.euro-office.lite/fonts"));
    assert_eq!(
        font_cache_path(&flatpak, id),
        Path::new("/home/ana/.var/app/org.euro_office.Lite/cache/org.euro-office.lite/fonts")
    );
    assert_eq!(font_cache_path(&macos, id), Path::new("/Users/ana/Library/Caches/org.euro-office.lite/fonts"));
    assert_eq!(windows, PathBuf::from(r"C:\Users\ana\AppData\Local"));
}

#[test]
fn a_relative_or_missing_cache_folder_is_not_used() {
    assert_eq!(linux_cache_dir(os("relative"), os("/home/ana")), Some(PathBuf::from("/home/ana/.cache")));
    assert_eq!(linux_cache_dir(None, os("")), None);
    assert_eq!(macos_cache_dir(None), None);
    assert_eq!(windows_cache_dir(os("")), None);
}

#[cfg(unix)]
#[test]
fn the_font_root_is_a_private_folder_in_the_user_cache() {
    let base = scratch_root();
    let temp = base.join("tmp");
    let preferred = font_cache_path(&base.join("cache"), "org.euro-office.lite");

    let choice = choose_font_root(&preferred, &temp, ensure_font_dir);

    assert_eq!(choice.path, preferred);
    assert!(choice.note.is_none());
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&preferred).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn a_font_root_that_is_a_link_falls_back_to_the_temp_root() {
    let base = scratch_root();
    let temp = base.join("tmp");
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let preferred = font_cache_path(&base.join("cache"), "org.euro-office.lite");
    std::fs::create_dir_all(preferred.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &preferred).unwrap();

    let choice = choose_font_root(&preferred, &temp, ensure_font_dir);

    assert_eq!(choice.path, temp);
    assert!(choice.note.unwrap().contains("not a plain folder"));
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn a_font_root_open_to_others_falls_back_to_the_temp_root() {
    use std::os::unix::fs::PermissionsExt;
    let base = scratch_root();
    let temp = base.join("tmp");
    let preferred = font_cache_path(&base.join("cache"), "org.euro-office.lite");
    std::fs::create_dir_all(&preferred).unwrap();
    std::fs::set_permissions(&preferred, std::fs::Permissions::from_mode(0o777)).unwrap();

    let choice = choose_font_root(&preferred, &temp, ensure_font_dir);

    assert_eq!(choice.path, temp);
    assert!(choice.note.unwrap().contains("permissions 777"));
    let _ = std::fs::remove_dir_all(&base);
}

// ---------------------------------------------------------------------------
// Font cache marker
// ---------------------------------------------------------------------------

fn sources_for(binaries: &Path, font_dirs: &[PathBuf]) -> String {
    font_sources_text(binaries, font_dirs)
}

#[test]
fn a_marker_with_only_the_version_is_not_ready() {
    let base = scratch_root();
    let sources = sources_for(&base.join("binaries"), &[]);
    let root = root_with_marker(b"0.17.43-alpha");

    assert!(!fonts_ready_for(&root, &marker_text("0.17.43-alpha", &sources)));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_cache_built_by_another_install_is_not_ready() {
    let base = scratch_root();
    let release = base.join("release/binaries");
    let dev = base.join("dev/binaries");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::create_dir_all(&dev).unwrap();
    let built_by_release = marker_text("0.17.43-alpha", &sources_for(&release, &[]));
    let root = root_with_marker(built_by_release.as_bytes());

    assert!(fonts_ready_for(&root, &built_by_release));
    assert!(!fonts_ready_for(&root, &marker_text("0.17.43-alpha", &sources_for(&dev, &[]))));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_new_font_in_a_scanned_folder_makes_the_cache_stale() {
    let base = scratch_root();
    let binaries = base.join("binaries");
    let user_fonts = base.join("user-fonts");
    std::fs::create_dir_all(&user_fonts).unwrap();
    std::fs::write(user_fonts.join("a.ttf"), b"a").unwrap();
    let before = marker_text("0.17.43-alpha", &sources_for(&binaries, &[user_fonts.clone()]));
    let root = root_with_marker(before.as_bytes());

    std::fs::write(user_fonts.join("b.ttf"), b"b").unwrap();
    let after = marker_text("0.17.43-alpha", &sources_for(&binaries, &[user_fonts.clone()]));

    assert!(!fonts_ready_for(&root, &after));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_font_folder_that_appears_makes_the_cache_stale() {
    let base = scratch_root();
    let binaries = base.join("binaries");
    let user_fonts = base.join("user-fonts");
    let before = marker_text("0.17.43-alpha", &sources_for(&binaries, &[user_fonts.clone()]));
    let root = root_with_marker(before.as_bytes());

    std::fs::create_dir_all(&user_fonts).unwrap();
    let after = marker_text("0.17.43-alpha", &sources_for(&binaries, &[user_fonts.clone()]));

    assert!(!fonts_ready_for(&root, &after));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn the_binaries_folder_is_recorded_by_its_real_path() {
    let base = scratch_root();
    let real = base.join("opt/binaries");
    std::fs::create_dir_all(&real).unwrap();
    let link = base.join("binaries");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    assert_eq!(sources_for(&link, &[]), sources_for(&real, &[]));
    let _ = std::fs::remove_dir_all(&base);
}

// ---------------------------------------------------------------------------
// Font folder fingerprint
// ---------------------------------------------------------------------------

fn fingerprint(dirs: &[PathBuf]) -> String {
    font_sources_text(Path::new("/no-binaries"), dirs)
}

// Folder times have a fine resolution, but not every file system has one.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

#[test]
fn a_new_folder_two_levels_down_changes_the_fingerprint() {
    let root = scratch_root();
    std::fs::create_dir_all(root.join("truetype")).unwrap();
    let before = fingerprint(&[root.clone()]);

    tick();
    std::fs::create_dir(root.join("truetype/new-family")).unwrap();

    assert_ne!(fingerprint(&[root.clone()]), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_new_file_in_a_subfolder_changes_the_fingerprint() {
    let root = scratch_root();
    std::fs::create_dir_all(root.join("truetype/dejavu")).unwrap();
    let before = fingerprint(&[root.clone()]);

    tick();
    std::fs::write(root.join("truetype/dejavu/DejaVuSans.ttf"), b"font").unwrap();

    assert_ne!(fingerprint(&[root.clone()]), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_fingerprint_stops_four_levels_down_and_says_so() {
    let root = scratch_root();
    let deep = root.join("a/b/c/d/e");
    std::fs::create_dir_all(&deep).unwrap();
    let before = fingerprint(&[root.clone()]);

    tick();
    std::fs::create_dir(deep.join("f")).unwrap();

    let after = fingerprint(&[root.clone()]);
    assert_eq!(after, before);
    assert!(after.contains("\ttruncated"), "{}", after);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_fingerprint_stops_at_the_folder_limit_and_says_so() {
    let root = scratch_root();
    for n in 0..FINGERPRINT_MAX_DIRS + 10 {
        std::fs::create_dir(root.join(format!("{:05}", n))).unwrap();
    }
    let before = fingerprint(&[root.clone()]);

    tick();
    let past_the_limit = root.join(format!("{:05}", FINGERPRINT_MAX_DIRS + 5));
    std::fs::create_dir(past_the_limit.join("inner")).unwrap();

    let after = fingerprint(&[root.clone()]);
    assert_eq!(after, before);
    assert!(after.contains(&format!("\t{}\ttruncated", FINGERPRINT_MAX_DIRS + 1)), "{}", after);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_linked_folder_is_not_followed() {
    let base = scratch_root();
    let root = base.join("fonts");
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir_all(root.join("real")).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, root.join("link")).unwrap();
    let before = fingerprint(&[root.clone()]);

    tick();
    std::fs::create_dir(elsewhere.join("new-family")).unwrap();
    assert_eq!(fingerprint(&[root.clone()]), before);

    std::fs::write(root.join("real/a.ttf"), b"font").unwrap();
    assert_ne!(fingerprint(&[root.clone()]), before);
    let _ = std::fs::remove_dir_all(&base);
}

// ---------------------------------------------------------------------------
// Font cache left in the temp root
// ---------------------------------------------------------------------------

fn temp_root_with_old_cache() -> PathBuf {
    let root = root_with_marker(b"0.17.43-alpha");
    std::fs::create_dir_all(root.join(format!("{}1-1", FONTS_STAGING_PREFIX))).unwrap();
    root
}

#[test]
fn the_old_cache_in_the_temp_root_is_removed_when_unused() {
    let temp = temp_root_with_old_cache();
    let own = begin_instance(&temp, now(), 2).unwrap();
    let fonts = scratch_root();

    assert!(remove_old_font_cache(&temp, &fonts, &own.id));

    assert!(!temp.join(FONTS_MARKER).exists());
    assert!(!fontdata_dir(&temp).exists());
    assert!(!temp.join(format!("{}1-1", FONTS_STAGING_PREFIX)).exists());
    let _ = std::fs::remove_dir_all(&temp);
    let _ = std::fs::remove_dir_all(&fonts);
}

#[test]
fn the_old_cache_stays_while_an_earlier_version_holds_its_lock() {
    let temp = temp_root_with_old_cache();
    let own = begin_instance(&temp, now(), 2).unwrap();
    let fonts = scratch_root();
    let _held = take_lock(&temp.join(FONTS_LOCK)).unwrap();

    assert!(!remove_old_font_cache(&temp, &fonts, &own.id));

    assert!(temp.join(FONTS_MARKER).exists());
    assert!(fontdata_dir(&temp).join("AllFonts.js").exists());
    let _ = std::fs::remove_dir_all(&temp);
    let _ = std::fs::remove_dir_all(&fonts);
}

#[test]
fn the_cache_is_kept_when_it_fell_back_to_the_temp_root() {
    let temp = temp_root_with_old_cache();
    let own = begin_instance(&temp, now(), 2).unwrap();

    assert!(!remove_old_font_cache(&temp, &temp, &own.id));

    assert!(temp.join(FONTS_MARKER).exists());
    assert!(fontdata_dir(&temp).join("AllFonts.js").exists());
    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn the_old_cache_stays_while_another_instance_is_alive() {
    let temp = temp_root_with_old_cache();
    let own = begin_instance(&temp, now(), 2).unwrap();
    let fonts = scratch_root();
    let other = begin_instance(&temp, now() - 1, 1).unwrap();

    assert!(!remove_old_font_cache(&temp, &fonts, &own.id));

    assert!(temp.join(FONTS_MARKER).exists());
    assert!(fontdata_dir(&temp).join("AllFonts.js").exists());
    other.end();
    let _ = std::fs::remove_dir_all(&temp);
    let _ = std::fs::remove_dir_all(&fonts);
}
