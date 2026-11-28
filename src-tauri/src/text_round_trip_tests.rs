// Opens a TXT or CSV with a real x2t the way open_file does and saves it back
// the way save_file does, with no edit in between. Ignored by default: it
// needs x2t and its libraries in EO_X2T_BINARIES (the folder the app ships as
// binaries/), as get-x2t-ci.sh unpacks them.
// Run: EO_X2T_BINARIES=/path/binaries cargo test -- --ignored real_x2t
use crate::text_import::{detect_file, finish_saved_text, TextKind, TextOptions};
use crate::x2t_task::{
    convert_env, convert_params_xml, detect_format, is_x2t_binary, params_path, write_file,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const EDITOR_BIN: i32 = 8192;

fn binaries_dir() -> PathBuf {
    let Ok(binaries) = std::env::var("EO_X2T_BINARIES") else {
        panic!("EO_X2T_BINARIES is not set");
    };
    PathBuf::from(binaries)
}

fn x2t_in(binaries_dir: &Path) -> PathBuf {
    let entries = std::fs::read_dir(binaries_dir).expect("EO_X2T_BINARIES is not a folder");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_x2t_binary(&name) {
            return entry.path();
        }
    }
    panic!("no x2t in EO_X2T_BINARIES");
}

// One x2t run with the parameters and the environment the app gives it.
fn convert(
    from: &Path,
    to: &Path,
    format_to: i32,
    temp_dir: &Path,
    options: Option<TextOptions>,
) -> Result<(), String> {
    let binaries_dir = binaries_dir();
    let params = params_path(temp_dir, "convert");
    let xml = convert_params_xml(
        &from.to_string_lossy(),
        &to.to_string_lossy(),
        format_to,
        &temp_dir.to_string_lossy(),
        &options.map(|options| options.xml_elements()).unwrap_or_default(),
    );
    write_file(&params, &xml).unwrap();
    let status = std::process::Command::new(x2t_in(&binaries_dir))
        .current_dir(&binaries_dir)
        .arg(&params)
        .envs(convert_env(&binaries_dir))
        .status()
        .expect("x2t did not start");
    match status.code() {
        Some(0) => Ok(()),
        code => Err(format!("x2t exit code {:?}", code)),
    }
}

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

// A folder of its own for each file a test opens.
fn fresh_dir() -> PathBuf {
    let n = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("eo-real-x2t-text-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// What the file holds after the app opens it and saves it with no edit, or
// the error the open ends with.
fn open_and_save(name: &str, content: &[u8]) -> Result<Vec<u8>, String> {
    let dir = fresh_dir();
    let original = dir.join(name);
    std::fs::write(&original, content).unwrap();
    let kind = TextKind::for_path(&original).expect("a TXT or CSV name");
    let options = detect_file(&original, kind).map_err(|e| format!("detect: {}", e))?;

    let editor_bin = dir.join("Editor.bin");
    let input = crate::converter::prepare_editor_input(&original, &dir, Some(kind), Some(options))?;
    let opened = convert(input.path(), &editor_bin, EDITOR_BIN, &dir, input.options.or(Some(options)));
    drop(input);
    opened?;

    let saved = dir.join("saved").join(name);
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    let save_options = Some(options.for_kind(kind));
    convert(&editor_bin, &saved, detect_format(&saved), &dir, save_options)?;
    finish_saved_text(&saved, kind, options).map_err(|e| format!("finish: {}", e))?;
    let bytes = std::fs::read(&saved).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(bytes)
}

fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn utf16be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

fn with_bom(bom: &[u8], body: Vec<u8>) -> Vec<u8> {
    [bom, &body].concat()
}

const LATIN: &str = "Accents áéíóú ñ\r\nSecond line\r\n";
const CJK: &str = "一二三 中文 text\nかな カナ\n";

fn assert_saved_as_opened(label: &str, name: &str, content: &[u8]) {
    let saved = open_and_save(name, content).unwrap_or_else(|error| panic!("{label}: {error}"));
    assert_eq!(saved, content, "{label}");
}

#[test]
#[ignore]
fn real_x2t_keeps_every_byte_of_the_texts_it_opened_before() {
    let utf8 = "Accents áéíóú ñ\nSecond line\n".as_bytes().to_vec();
    let cases: [(&str, &str, Vec<u8>); 9] = [
        ("UTF-8", "a.txt", utf8.clone()),
        ("UTF-8 with a BOM", "a.txt", with_bom(&[0xEF, 0xBB, 0xBF], utf8)),
        ("UTF-16 LE with a BOM", "a.txt", with_bom(&[0xFF, 0xFE], utf16le(LATIN))),
        ("UTF-16 BE with a BOM", "a.txt", with_bom(&[0xFE, 0xFF], utf16be(LATIN))),
        ("CRLF", "a.txt", b"first line\r\nsecond line\r\n".to_vec()),
        ("windows-1252", "a.txt", b"caf\xE9 ni\xF1o\r\n".to_vec()),
        ("text that reads like HTML", "a.txt", b"<html><body><b>x</b></body></html>\n".to_vec()),
        ("CSV with an emoji", "a.csv", "name,icon\nsmile,\u{1F600}\n".as_bytes().to_vec()),
        ("windows-1252 CSV", "a.csv", b"a;b\ncaf\xE9;ni\xF1o\n".to_vec()),
    ];
    for (label, name, content) in cases {
        assert_saved_as_opened(label, name, &content);
    }
}

#[test]
#[ignore]
fn real_x2t_keeps_every_byte_of_a_utf16_txt_without_a_bom() {
    for (label, text) in [("Latin", LATIN), ("CJK and ASCII", CJK)] {
        assert_saved_as_opened(&format!("{label} LE"), "a.txt", &utf16le(text));
        assert_saved_as_opened(&format!("{label} BE"), "a.txt", &utf16be(text));
    }
}

// Only Linux opens these: the x2t of Windows and of macOS has not been tried
// with the UTF-16 copy (text_import::read_copy_format).
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn real_x2t_keeps_every_byte_of_a_txt_outside_the_bmp() {
    let emoji = "x\na \u{1F600} b\ny\n";
    let extension_b = "\u{20000}\u{2A6D6} extension B\r\n\r\n";
    let line = "Línea dentro del BMP con acentos y cirílico Привет.\n";
    let long = format!("{}última línea {}\n", line.repeat(2000), '\u{1F600}');
    let cases: [(&str, Vec<u8>); 8] = [
        ("emoji in UTF-8", emoji.as_bytes().to_vec()),
        ("emoji in UTF-8 with a BOM", with_bom(&[0xEF, 0xBB, 0xBF], emoji.as_bytes().to_vec())),
        ("emoji in UTF-16 LE with a BOM", with_bom(&[0xFF, 0xFE], utf16le(emoji))),
        ("emoji in UTF-16 BE with a BOM", with_bom(&[0xFE, 0xFF], utf16be(emoji))),
        ("emoji in UTF-16 LE without a BOM", utf16le("a \u{1F600} b\r\nnext\r\n")),
        ("CJK extension B", extension_b.as_bytes().to_vec()),
        ("HTML with an emoji", "<html><body>\u{1F600}</body></html>\n".as_bytes().to_vec()),
        ("emoji past the first 64 KiB", long.into_bytes()),
    ];
    for (label, content) in cases {
        assert_saved_as_opened(label, "a.txt", &content);
    }
}

// The copy text_import makes for Linux, read by the x2t of this system: what
// to run on Windows and macOS before they open such a TXT too.
#[test]
#[ignore]
fn real_x2t_reads_a_txt_outside_the_bmp_from_utf16_le_without_a_bom() {
    let text = "x\na \u{1F600} b\n\u{20000} y";
    let dir = fresh_dir();
    let copy = dir.join("copy.txt");
    std::fs::write(&copy, utf16le(text)).unwrap();
    let options = crate::text_import::detect(&utf16le("ab"), true, TextKind::Txt);
    assert_eq!(options.encoding, crate::text_import::ENCODING_UTF16_LE);

    let editor_bin = dir.join("Editor.bin");
    convert(&copy, &editor_bin, EDITOR_BIN, &dir, Some(options)).unwrap();
    let written = dir.join("written.txt");
    convert(&editor_bin, &written, detect_format(&written), &dir, Some(options)).unwrap();

    let utf8 = TextOptions { encoding: crate::text_import::ENCODING_UTF8, crlf: false, trailing_breaks: 0, ..options };
    let (rebuilt, _) = crate::text_import::rebuild_txt(&std::fs::read(&written).unwrap(), utf8);
    assert_eq!(String::from_utf8(rebuilt).unwrap(), text);
    let _ = std::fs::remove_dir_all(&dir);
}
