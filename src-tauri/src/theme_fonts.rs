// Repairs the East Asian font names in the default theme x2t writes on Windows.
//
// x2t.exe 10.0.0.169 for Windows (upstream ONLYOFFICE) carries the five names
// below as UTF-16 literals that were compiled from UTF-8 source read as
// Windows-1252: each UTF-8 byte became one cp1252 character. Its default theme
// therefore says typeface="ï¼­ï¼³ ã‚´ã‚·ãƒƒã‚¯" where it means "ＭＳ ゴシック".
// It reaches word/theme/theme1.xml (a DOCX saved from a document without a
// theme), xl/theme/theme.xml of every XLSX and the notes master theme of a
// PPTX. The Linux and macOS builds write the names correctly, so there the
// repair finds nothing and leaves the file alone.
//
// The table is closed: a name not in it is never touched.

use std::io::Read;
use std::path::Path;

const DEFAULT_THEME_EAST_ASIAN_FACES: [&str; 5] =
    ["ＭＳ ゴシック", "ＭＳ 明朝", "맑은 고딕", "宋体", "新細明體"];

const OOXML_EXTENSIONS: [&str; 12] = [
    "docx", "docm", "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "pptx", "pptm", "potx",
    "potm",
];

const EMBEDDED_WORKBOOK_DIRS: [&str; 2] = ["word/embeddings/", "ppt/embeddings/"];

// Windows-1252 for 0x80..=0x9F, the only range where it differs from Latin-1.
// The five bytes cp1252 leaves undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D) keep
// their own code point, as Windows maps them: x2t.exe 10.0.0.169 stores
// "ＭＳ 明朝" and "맑은 고딕" with U+009D where their UTF-8 has 0x9D.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

fn cp1252_char(byte: u8) -> char {
    if (0x80..=0x9F).contains(&byte) {
        return CP1252_HIGH[(byte - 0x80) as usize];
    }
    char::from(byte)
}

// The name as x2t.exe writes it: its UTF-8 bytes read one by one as cp1252.
fn mojibake_of(name: &str) -> String {
    name.bytes().map(cp1252_char).collect()
}

// Replaces the broken form of each known name with the right one, only as a
// whole typeface="…" value. Returns None when nothing matched.
pub fn repair_theme_xml(xml: &str) -> Option<String> {
    let mut repaired = xml.to_string();
    for face in DEFAULT_THEME_EAST_ASIAN_FACES {
        let broken = format!("typeface=\"{}\"", mojibake_of(face));
        let fixed = format!("typeface=\"{}\"", face);
        repaired = repaired.replace(&broken, &fixed);
    }
    if repaired == xml {
        return None;
    }
    Some(repaired)
}

fn has_extension_in(name: &str, extensions: &[&str]) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    extensions.iter().any(|known| ext.eq_ignore_ascii_case(known))
}

fn is_ooxml_package(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    has_extension_in(&name, &OOXML_EXTENSIONS)
}

// <dir>/theme/<file>.xml, as word/theme/theme1.xml, xl/theme/theme.xml or
// ppt/theme/theme2.xml. The theme's own _rels are not XML parts of it.
fn is_theme_part(entry: &str) -> bool {
    let mut segments = entry.rsplit('/');
    let Some(file) = segments.next() else {
        return false;
    };
    let in_theme_dir = segments.next() == Some("theme");
    in_theme_dir && has_extension_in(file, &["xml"])
}

fn is_embedded_workbook(entry: &str) -> bool {
    let in_embeddings = EMBEDDED_WORKBOOK_DIRS.iter().any(|dir| entry.starts_with(dir));
    in_embeddings && has_extension_in(entry, &["xlsx"])
}

// The repaired package, or None when it holds nothing to repair. Workbooks
// embedded in a document or presentation are repaired too, one level deep.
pub fn repair_package(bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    repair_package_at_depth(bytes, 0)
}

fn repair_package_at_depth(bytes: &[u8], depth: u8) -> Result<Option<Vec<u8>>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("Not a readable package: {}", e))?;

    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    let mut edits: Vec<(String, Vec<u8>)> = Vec::new();

    for name in names {
        let repaired = if is_theme_part(&name) {
            repair_theme_entry(&mut archive, &name)?
        } else if depth == 0 && is_embedded_workbook(&name) {
            repair_embedded_entry(&mut archive, &name)?
        } else {
            None
        };
        if let Some(data) = repaired {
            edits.push((name, data));
        }
    }

    if edits.is_empty() {
        return Ok(None);
    }
    let rewritten = crate::note_separator::rewrite_package(&mut archive, &edits)?;
    Ok(Some(rewritten))
}

fn read_named<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, String> {
    let data = crate::note_separator::read_entry(archive, name)?;
    data.ok_or_else(|| format!("{} vanished from the package", name))
}

// A theme part that is not UTF-8 is not one x2t wrote, so it is left alone.
fn repair_theme_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let data = read_named(archive, name)?;
    let Ok(xml) = String::from_utf8(data) else {
        return Ok(None);
    };
    Ok(repair_theme_xml(&xml).map(String::into_bytes))
}

// An embedded workbook that is not a readable zip is left alone: the document
// around it is still repaired.
fn repair_embedded_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let data = read_named(archive, name)?;
    Ok(repair_package_at_depth(&data, 1).unwrap_or(None))
}

// Repairs the OOXML file x2t just wrote at `path`, in place. Returns whether it
// changed. Any other format, and a package with nothing to repair, is not
// rewritten at all.
pub fn repair_saved_file(path: &Path) -> Result<bool, String> {
    if !is_ooxml_package(path) {
        return Ok(false);
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    let Some(repaired) = repair_package(&bytes)? else {
        return Ok(false);
    };

    // Same directory as the destination, so the rename stays on one filesystem.
    let temp_path = crate::note_separator::temp_sibling(path);
    if let Err(e) = std::fs::write(&temp_path, &repaired) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Cannot write {}: {}", temp_path.display(), e));
    }
    if let Err(e) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Cannot replace {}: {}", path.display(), e));
    }
    Ok(true)
}

// Called after x2t has written a save. A failed repair is logged and the save
// still succeeds: the file stays exactly as x2t wrote it.
pub async fn repair_after_save(state: &crate::file_ops::AppState, dest: &Path) {
    let path = dest.to_path_buf();
    let outcome = tauri::async_runtime::spawn_blocking(move || repair_saved_file(&path))
        .await
        .map_err(|e| format!("theme repair task failed: {}", e))
        .and_then(|result| result);

    let line = match outcome {
        Ok(false) => return,
        Ok(true) => format!("[THEME] repaired East Asian font names path={}", dest.display()),
        Err(error) => format!("[THEME] repair failed path={} error={}", dest.display(), error),
    };
    crate::file_ops::log_event(state, &line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const BROKEN_MS_GOTHIC: &str = "ï¼\u{AD}ï¼³ ã‚´ã‚·ãƒƒã‚¯";

    fn font(script: &str, face: &str) -> String {
        format!("<a:font script=\"{}\" typeface=\"{}\"/>", script, face)
    }

    fn theme_with(faces: &[(&str, &str)]) -> String {
        let fonts: String = faces.iter().map(|(s, f)| font(s, f)).collect();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><a:theme><a:majorFont>\
             <a:latin typeface=\"Arial\"/>{}</a:majorFont></a:theme>",
            fonts
        )
    }

    fn broken_theme() -> String {
        let faces: Vec<(&str, String)> = [("Jpan", "ＭＳ ゴシック"), ("Hang", "맑은 고딕")]
            .iter()
            .map(|(s, f)| (*s, mojibake_of(f)))
            .collect();
        let refs: Vec<(&str, &str)> = faces.iter().map(|(s, f)| (*s, f.as_str())).collect();
        theme_with(&refs)
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            out.start_file(*name, options).unwrap();
            out.write_all(data).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    fn entries_of(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut entry = archive.by_index(i).unwrap();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                (entry.name().to_string(), data)
            })
            .collect()
    }

    fn entry(bytes: &[u8], name: &str) -> Vec<u8> {
        entries_of(bytes).into_iter().find(|(n, _)| n == name).unwrap().1
    }

    #[test]
    fn mojibake_of_ms_gothic_matches_what_windows_x2t_writes() {
        assert_eq!(mojibake_of("ＭＳ ゴシック"), BROKEN_MS_GOTHIC);
    }

    #[test]
    fn undefined_cp1252_bytes_keep_their_code_point() {
        // Read from the UTF-16 literals in x2t.exe 10.0.0.169 for Windows.
        assert_eq!(mojibake_of("ＭＳ 明朝"), "ï¼\u{AD}ï¼³ æ˜Žæœ\u{9D}");
        assert_eq!(mojibake_of("맑은 고딕"), "ë§‘ì\u{9D}€ ê³\u{A0}ë”•");
    }

    #[test]
    fn each_broken_name_is_repaired() {
        for face in DEFAULT_THEME_EAST_ASIAN_FACES {
            let broken = font("Jpan", &mojibake_of(face));
            assert_eq!(repair_theme_xml(&broken), Some(font("Jpan", face)), "{}", face);
        }
    }

    #[test]
    fn correct_names_are_left_alone() {
        for face in DEFAULT_THEME_EAST_ASIAN_FACES {
            assert_eq!(repair_theme_xml(&font("Jpan", face)), None, "{}", face);
        }
    }

    #[test]
    fn a_name_outside_the_table_is_left_alone() {
        let foreign = font("Jpan", &mojibake_of("メイリオ"));
        assert_eq!(repair_theme_xml(&foreign), None);
    }

    #[test]
    fn the_broken_text_outside_a_typeface_is_left_alone() {
        let elsewhere = format!("<a:t>{}</a:t><a:x name=\"{}\"/>", BROKEN_MS_GOTHIC, BROKEN_MS_GOTHIC);
        assert_eq!(repair_theme_xml(&elsewhere), None);
    }

    #[test]
    fn theme_parts_are_recognised() {
        assert!(is_theme_part("word/theme/theme1.xml"));
        assert!(is_theme_part("xl/theme/theme.xml"));
        assert!(is_theme_part("ppt/theme/theme2.xml"));
        assert!(!is_theme_part("ppt/theme/_rels/theme1.xml.rels"));
        assert!(!is_theme_part("word/document.xml"));
    }

    #[test]
    fn docx_theme_is_repaired_and_other_entries_keep_their_bytes() {
        let document = b"<w:document>\xE5\xAE\x8B</w:document>".to_vec();
        let package = zip_of(&[
            ("[Content_Types].xml", b"<Types/>"),
            ("word/document.xml", &document),
            ("word/theme/theme1.xml", broken_theme().as_bytes()),
        ]);

        let repaired = repair_package(&package).unwrap().expect("rewritten");

        let theme = String::from_utf8(entry(&repaired, "word/theme/theme1.xml")).unwrap();
        assert!(theme.contains("typeface=\"ＭＳ ゴシック\""));
        assert!(theme.contains("typeface=\"맑은 고딕\""));
        let before = entries_of(&package);
        let after = entries_of(&repaired);
        let names = |v: &[(String, Vec<u8>)]| v.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        assert_eq!(names(&before), names(&after));
        assert_eq!(before[0], after[0]);
        assert_eq!(before[1], after[1]);
    }

    #[test]
    fn xlsx_orphan_theme_is_repaired() {
        let package = zip_of(&[
            ("xl/workbook.xml", b"<workbook/>"),
            ("xl/theme/theme.xml", broken_theme().as_bytes()),
        ]);
        let repaired = repair_package(&package).unwrap().expect("rewritten");
        let theme = String::from_utf8(entry(&repaired, "xl/theme/theme.xml")).unwrap();
        assert!(theme.contains("typeface=\"ＭＳ ゴシック\""));
    }

    #[test]
    fn workbook_embedded_in_docx_is_repaired() {
        let workbook = zip_of(&[("xl/theme/theme.xml", broken_theme().as_bytes())]);
        let package = zip_of(&[
            ("word/document.xml", b"<w:document/>"),
            ("word/embeddings/Microsoft_Excel_Worksheet.xlsx", &workbook),
        ]);

        let repaired = repair_package(&package).unwrap().expect("rewritten");

        let inner = entry(&repaired, "word/embeddings/Microsoft_Excel_Worksheet.xlsx");
        let theme = String::from_utf8(entry(&inner, "xl/theme/theme.xml")).unwrap();
        assert!(theme.contains("typeface=\"ＭＳ ゴシック\""));
    }

    #[test]
    fn package_without_broken_names_is_not_rewritten_on_disk() {
        let good = theme_with(&[("Jpan", "ＭＳ ゴシック")]);
        let package = zip_of(&[("word/theme/theme1.xml", good.as_bytes())]);
        let dir = std::env::temp_dir().join(format!("eo-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clean.docx");
        std::fs::write(&path, &package).unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();

        assert!(!repair_saved_file(&path).unwrap());

        assert_eq!(std::fs::read(&path).unwrap(), package);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn broken_docx_on_disk_is_replaced() {
        let package = zip_of(&[("word/theme/theme1.xml", broken_theme().as_bytes())]);
        let dir = std::env::temp_dir().join(format!("eo-theme-disk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.docx");
        std::fs::write(&path, &package).unwrap();

        assert!(repair_saved_file(&path).unwrap());

        let theme = String::from_utf8(entry(&std::fs::read(&path).unwrap(), "word/theme/theme1.xml")).unwrap();
        assert!(theme.contains("typeface=\"ＭＳ ゴシック\""));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn other_formats_are_not_opened() {
        let path = std::path::Path::new("/nonexistent/document.odt");
        assert!(!repair_saved_file(path).unwrap());
    }
}
