// Saves with a real x2t through convert_and_place while the disk of the
// instance folder is nearly full. Ignored by default: it needs x2t and its
// libraries in EO_X2T_BINARIES, as get-x2t-ci.sh unpacks them, and in
// EO_SMALL_DISK an empty folder on a file system of its own that the test
// fills, such as a tmpfs of 64 MB. A PDF is saved only when editors/ sits next
// to the binaries folder, as in an installed app.
// Run: EO_X2T_BINARIES=/path/binaries EO_SMALL_DISK=/mnt/small \
//      cargo test -- --ignored --nocapture --test-threads=1 real_x2t_save
use crate::save_space::{needed_for_save, SaveSpace};
use crate::template_round_trip_tests::x2t_in;
use crate::text_import::{finish_saved_text, TextKind, TextOptions, ENCODING_UTF8};
use crate::x2t_task::{
    convert_env, convert_params_xml, convert_to_pdf, detect_format, params_path,
    run_with_time_limit, write_file, DOCUMENT_TIME_LIMIT,
};
use std::io::Read;
use std::path::{Path, PathBuf};

const EDITOR_BIN: i32 = 8192;
const PDF: i32 = 513;
const KIB: u64 = 1024;
const OLD: &[u8] = b"the file as the user last saved it";

// What the tests need from the environment.
struct Setup {
    binaries: PathBuf,
    // The file system the instance folder is on.
    small_disk: PathBuf,
    // The font cache a PDF is written with, None when editors/ is missing.
    fontdata: Option<PathBuf>,
}

impl Setup {
    fn from_env() -> Setup {
        let binaries = PathBuf::from(std::env::var("EO_X2T_BINARIES").expect("EO_X2T_BINARIES is not set"));
        let small_disk = PathBuf::from(std::env::var("EO_SMALL_DISK").expect("EO_SMALL_DISK is not set"));
        clear_dir(&small_disk);
        let fontdata = font_cache(&binaries, &small_disk);
        Setup { binaries, small_disk, fontdata }
    }

    fn instance(&self) -> PathBuf {
        self.small_disk.join("instance")
    }

    fn filler(&self) -> PathBuf {
        self.small_disk.join("filler")
    }

    fn free(&self) -> u64 {
        let volume = crate::converter::volume_of(&self.small_disk).expect("the free space cannot be told");
        volume.free
    }

    // Leaves `free` bytes on the small disk.
    fn leave_free(&self, free: u64) {
        let _ = std::fs::remove_file(self.filler());
        let surplus = self.free().checked_sub(free).expect("EO_SMALL_DISK is too small");
        std::fs::write(self.filler(), vec![0u8; surplus as usize]).unwrap();
        assert_eq!(self.free(), free, "the filler did not leave the space asked for");
    }

    // The instance folder as the app has it once `source` is open: Editor.bin
    // and media/, nothing a save left behind.
    fn open(&self, source: &Path, extra: &str) {
        let _ = std::fs::remove_file(self.filler());
        let instance = self.instance();
        let _ = std::fs::remove_dir_all(&instance);
        std::fs::create_dir_all(&instance).unwrap();
        let editor_bin = instance.join("Editor.bin");
        run_x2t(&self.binaries, source, &editor_bin, EDITOR_BIN, &instance, extra).unwrap();
        self.forget_earlier_saves();
    }

    fn forget_earlier_saves(&self) {
        for entry in std::fs::read_dir(self.instance()).unwrap().flatten() {
            let name = entry.file_name();
            if name == "Editor.bin" || name == "media" {
                continue;
            }
            let _ = std::fs::remove_dir_all(entry.path());
            let _ = std::fs::remove_file(entry.path());
        }
    }

    // One save of the open document to `dest`, as save_editor_bin does it.
    fn save(&self, dest: &Path) -> Saved {
        let instance = self.instance();
        let text_options = text_options_for(dest);
        let mut x2t = String::from("not run");
        let convert = |output: PathBuf| {
            let converted = self.convert(&instance.join("Editor.bin"), &output, text_options);
            x2t = exit_of(&converted);
            async move { converted }
        };
        let reshape = |staged: &Path| reshape_text(staged, text_options);
        let staging = instance.join("save_output");
        let space = SaveSpace {
            needed: needed_for_save(&instance, detect_format(dest)),
            volume: &crate::converter::volume_of,
        };
        let saved = crate::save_target::convert_and_place(&staging, dest, &space, convert, reshape);
        let result = tauri::async_runtime::block_on(saved);
        Saved { x2t, result }
    }

    fn convert(&self, input: &Path, output: &Path, text_options: Option<TextOptions>) -> Result<String, String> {
        let format_to = detect_format(output);
        if format_to != PDF {
            let extra = text_options.map(|options| options.xml_elements()).unwrap_or_default();
            return run_x2t(&self.binaries, input, output, format_to, &self.instance(), &extra);
        }
        convert_to_pdf(
            &self.binaries,
            None,
            &input.to_string_lossy(),
            &output.to_string_lossy(),
            &self.instance(),
            self.fontdata.as_deref().expect("a PDF needs editors/"),
            &|_| {},
            None,
        )
    }
}

struct Saved {
    // x2t's exit code, or why it did not run.
    x2t: String,
    result: Result<(), String>,
}

// One x2t run with the parameters and the environment converter::convert_file
// gives it.
fn run_x2t(
    binaries: &Path,
    input: &Path,
    output: &Path,
    format_to: i32,
    temp_dir: &Path,
    extra: &str,
) -> Result<String, String> {
    let params = params_path(temp_dir, "convert");
    let xml = convert_params_xml(
        &input.to_string_lossy(),
        &output.to_string_lossy(),
        format_to,
        &temp_dir.to_string_lossy(),
        extra,
    );
    write_file(&params, &xml)?;
    let mut command = std::process::Command::new(x2t_in(binaries));
    command.current_dir(binaries).arg(&params).envs(convert_env(binaries));
    let run = run_with_time_limit(&mut command, DOCUMENT_TIME_LIMIT);
    let _ = std::fs::remove_file(&params);
    let status = run.map_err(|error| error.to_string())?.status;
    if status.success() {
        return Ok("ok".to_string());
    }
    Err(format!("x2t conversion failed (exit code {})", status.code().unwrap_or(-1)))
}

fn exit_of(converted: &Result<String, String>) -> String {
    match converted {
        Ok(_) => "exit 0".to_string(),
        Err(error) => error.clone(),
    }
}

// UTF-8 without a BOM, LF and one final break: x2t writes a BOM, so the save
// rewrites the staged file as it does for most text files.
fn text_options_for(dest: &Path) -> Option<TextOptions> {
    let kind = TextKind::for_path(dest)?;
    let delimiter = match kind {
        TextKind::Csv => Some(4),
        TextKind::Txt => None,
    };
    Some(TextOptions { encoding: ENCODING_UTF8, delimiter, bom: false, crlf: false, trailing_breaks: 1 })
}

fn reshape_text(staged: &Path, text_options: Option<TextOptions>) {
    let (Some(options), Some(kind)) = (text_options, TextKind::for_path(staged)) else {
        return;
    };
    let _ = finish_saved_text(staged, kind, options);
}

fn clear_dir(dir: &Path) {
    for entry in std::fs::read_dir(dir).expect("EO_SMALL_DISK is not a folder").flatten() {
        let _ = std::fs::remove_dir_all(entry.path());
        let _ = std::fs::remove_file(entry.path());
    }
}

// Built on the small disk before it is filled, as the app builds it at startup.
fn font_cache(binaries: &Path, small_disk: &Path) -> Option<PathBuf> {
    let editors = binaries.parent()?.join("editors");
    if !editors.is_dir() {
        return None;
    }
    let fontdata = small_disk.join("fontdata");
    std::fs::create_dir_all(&fontdata).unwrap();
    assert!(crate::generate_fonts(&fontdata, binaries), "the font cache was not built");
    Some(fontdata)
}

fn user_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eo-save-full-temp-{}-{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn corpus_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus").join(name)
}

// About 128 KB of text in lines of different lengths.
fn long_text() -> String {
    let mut text = String::new();
    let mut line = 0u32;
    while text.len() < 128_000 {
        let words = 6 + (line * 7) % 9;
        for word in 0..words {
            text.push_str(["lorem", "ipsum", "dolor", "sit", "amet"][((line + word) % 5) as usize]);
            text.push(' ');
        }
        text.push_str(&format!("{}\n", line));
        line += 1;
    }
    text
}

// The parts of a package with their content, None for a file that is not a
// whole ZIP. Two runs of x2t differ only in the time stored with each part.
fn package_parts(path: &Path) -> Option<Vec<(String, Vec<u8>)>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut parts = Vec::new();
    for index in 0..archive.len() {
        let mut part = archive.by_index(index).ok()?;
        let mut content = Vec::new();
        part.read_to_end(&mut content).ok()?;
        parts.push((part.name().to_string(), content));
    }
    Some(parts)
}

// The dates x2t stamps a PDF with change from one second to the next.
fn without_pdf_dates(mut bytes: Vec<u8>) -> Vec<u8> {
    const STAMP_LEN: usize = 48;
    let mut at = 0;
    while let Some(found) = find(&bytes[at..], b"Date") {
        let start = at + found;
        let end = (start + STAMP_LEN).min(bytes.len());
        bytes[start..end].fill(b'#');
        at = end;
    }
    bytes
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn same_document(saved: &Path, whole: &Path) -> bool {
    if let Some(parts) = package_parts(whole) {
        return package_parts(saved) == Some(parts);
    }
    let (saved, whole) = (std::fs::read(saved).unwrap(), std::fs::read(whole).unwrap());
    without_pdf_dates(saved) == without_pdf_dates(whole)
}

// What a save with `free` bytes on the disk left at the destination.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    // The save failed and the destination is as it was.
    RefusedUntouched,
    // The save succeeded with the whole document.
    Whole,
    // The destination was replaced with something else: its length.
    Replaced { len: u64 },
    RefusedButChanged,
}

fn outcome(saved: &Saved, dest: &Path, whole: &Path) -> Outcome {
    let now = std::fs::read(dest).unwrap();
    match (&saved.result, now == OLD) {
        (Err(_), true) => Outcome::RefusedUntouched,
        (Err(_), false) => Outcome::RefusedButChanged,
        (Ok(_), _) if same_document(dest, whole) => Outcome::Whole,
        (Ok(_), _) => Outcome::Replaced { len: now.len() as u64 },
    }
}

// The free space tried: the small amounts a full disk leaves, steps up to
// twice what the save uses, and every 8 KiB around what it uses.
fn free_space_steps(used: u64) -> Vec<u64> {
    let mut steps = vec![0, 4 * KIB, 64 * KIB, 256 * KIB];
    let coarse = (used / 16).max(64 * KIB) / (4 * KIB) * (4 * KIB);
    steps.extend((512 * KIB..used * 2).step_by(coarse as usize));
    let around = used.saturating_sub(128 * KIB)..used + 128 * KIB;
    steps.extend(around.step_by((8 * KIB) as usize));
    steps.sort_unstable();
    steps.dedup();
    steps
}

struct Row {
    free: u64,
    // x2t's exit and what the save left, as the table prints them.
    what: String,
    outcome: Outcome,
}

// Saves the open document to `name` once for each amount of free space and
// prints what happened. Returns the rows.
fn save_with_little_room(setup: &Setup, name: &str) -> Vec<Row> {
    let user = user_dir("sweep");
    let whole = user_dir("whole").join(name);
    setup.forget_earlier_saves();
    let _ = std::fs::remove_file(setup.filler());
    let before = setup.free();
    setup.save(&whole).result.expect("the save with room failed");
    let used = before - setup.free();
    println!("\n{}: {} bytes, {} bytes of the disk used with room", name, file_len(&whole), used);

    let dest = user.join(name);
    let mut rows = Vec::new();
    for free in free_space_steps(used) {
        std::fs::write(&dest, OLD).unwrap();
        setup.forget_earlier_saves();
        setup.leave_free(free);
        let saved = setup.save(&dest);
        let outcome = outcome(&saved, &dest, &whole);
        let error = saved.result.err().unwrap_or_default();
        let what = format!("x2t: {:<8} {:?} {}", short(&saved.x2t), outcome, short(&error));
        rows.push(Row { free, what, outcome });
    }
    print_ranges(&rows);
    let _ = std::fs::remove_file(setup.filler());
    let _ = std::fs::remove_dir_all(&user);
    let _ = std::fs::remove_dir_all(whole.parent().unwrap());
    rows
}

// One line for each run of rows that ended the same way.
fn print_ranges(rows: &[Row]) {
    for range in rows.chunk_by(|a, b| a.what == b.what) {
        let (first, last) = (&range[0], &range[range.len() - 1]);
        println!("  free {:>9} to {:>9}  {}", first.free, last.free, first.what);
    }
}

// The start of an error, without the paths that follow it.
fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    let start = line.split(" /").next().unwrap_or_default();
    start.chars().take(60).collect()
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().len()
}

fn names_to_save(setup: &Setup, names: &[&'static str]) -> Vec<&'static str> {
    let with_pdf = setup.fontdata.is_some();
    names.iter().copied().filter(|name| with_pdf || !name.ends_with(".pdf")).collect()
}

// With too little room x2t still exits 0 and leaves a truncated file. The
// destination must then stay as it was.
#[test]
#[ignore]
fn real_x2t_save_with_a_nearly_full_disk_never_places_a_truncated_file() {
    let setup = Setup::from_env();
    let user = user_dir("source");
    let source = user.join("long.txt");
    std::fs::write(&source, long_text()).unwrap();
    setup.open(&source, "<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>\n");

    let mut truncated = Vec::new();
    for name in names_to_save(&setup, &["notes.txt", "report.docx", "report.odt", "report.rtf", "report.pdf"]) {
        for row in save_with_little_room(&setup, name) {
            if !matches!(row.outcome, Outcome::RefusedUntouched | Outcome::Whole) {
                truncated.push(format!("{} with {} bytes free: {:?}", name, row.free, row.outcome));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&user);
    assert!(truncated.is_empty(), "the destination was replaced:\n{}", truncated.join("\n"));
}

// The documents of tests/corpus saved in each format with room to spare. The
// saved files stay in EO_SAVED_FILES when it is set, to compare two builds.
#[test]
#[ignore]
fn real_x2t_save_with_room_writes_every_format() {
    const SAVES: [(&str, &[&str]); 4] = [
        ("rich.docx", &["rich.docx", "rich.odt", "rich.rtf", "rich.txt", "rich.dotx", "rich.pdf"]),
        ("sheet.xlsx", &["sheet.xlsx", "sheet.ods", "sheet.csv", "sheet.xltx", "sheet.pdf"]),
        ("slides.pptx", &["slides.pptx", "slides.odp", "slides.potx", "slides.pdf"]),
        ("utf8-comma-lf-final.csv", &["table.csv", "table.xlsx"]),
    ];
    let setup = Setup::from_env();
    let kept = std::env::var("EO_SAVED_FILES").map(PathBuf::from);
    let user = kept.clone().unwrap_or_else(|_| user_dir("room"));
    std::fs::create_dir_all(&user).unwrap();

    for (source, names) in SAVES {
        let csv_options = text_options_for(Path::new(source)).map(|options| options.xml_elements());
        setup.open(&corpus_file(source), &csv_options.unwrap_or_default());
        for name in names_to_save(&setup, names) {
            let dest = user.join(name);
            let saved = setup.save(&dest);
            assert_eq!(saved.result, Ok(()), "{}", name);
            assert!(file_len(&dest) > 0, "{} is empty", name);
            println!("{}: {} bytes", name, file_len(&dest));
        }
    }
    if kept.is_err() {
        let _ = std::fs::remove_dir_all(&user);
    }
}
