// What a file x2t wrote shows about whether x2t wrote all of it. x2t exits 0
// when a write fails, so a file cut short looks like a finished save.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const NOT_WHOLE: &str = "x2t did not write the whole document";

const ZIP_START: &[u8] = b"PK\x03\x04";
const PDF_START: &[u8] = b"%PDF";
const RTF_START: &[u8] = b"{\\rtf";

// A PDF reader looks for the end mark in the last 1024 bytes.
const PDF_TAIL: u64 = 1024;
// x2t ends an RTF with the closing brace, a line break and a NUL.
const RTF_TAIL: u64 = 16;

// Err when `staged` starts as a ZIP package, a PDF or an RTF and is not a
// whole one. A CSV or TXT has no end to look for, and neither has a format
// that starts in another way.
pub fn check_whole(staged: &Path) -> Result<(), String> {
    if crate::text_import::TextKind::for_path(staged).is_some() {
        return Ok(());
    }
    let mut file = File::open(staged).map_err(|e| e.to_string())?;
    let start = read_start(&mut file).map_err(|e| e.to_string())?;
    if start.starts_with(ZIP_START) {
        return check_package(file);
    }
    if start.starts_with(PDF_START) {
        return check_tail(file, PDF_TAIL, has_pdf_end, "the PDF has no end mark");
    }
    if start.starts_with(RTF_START) {
        return check_tail(file, RTF_TAIL, has_rtf_end, "the RTF has no closing brace");
    }
    Ok(())
}

fn read_start(file: &mut File) -> std::io::Result<Vec<u8>> {
    let mut start = Vec::new();
    file.take(RTF_START.len() as u64).read_to_end(&mut start)?;
    Ok(start)
}

// A package cut short has no directory at its end; a part cut short does not
// match the checksum the directory holds for it.
fn check_package(file: File) -> Result<(), String> {
    let mut package = zip::ZipArchive::new(file).map_err(|e| not_whole(&e.to_string()))?;
    for index in 0..package.len() {
        let mut part = package.by_index(index).map_err(|e| not_whole(&e.to_string()))?;
        std::io::copy(&mut part, &mut std::io::sink()).map_err(|e| not_whole(&e.to_string()))?;
    }
    Ok(())
}

fn check_tail(
    mut file: File,
    tail_len: u64,
    is_whole: fn(&[u8]) -> bool,
    reason: &str,
) -> Result<(), String> {
    let tail = read_tail(&mut file, tail_len).map_err(|e| e.to_string())?;
    if is_whole(&tail) {
        return Ok(());
    }
    Err(not_whole(reason))
}

fn read_tail(file: &mut File, tail_len: u64) -> std::io::Result<Vec<u8>> {
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(tail_len)))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    Ok(tail)
}

fn has_pdf_end(tail: &[u8]) -> bool {
    tail.windows(5).any(|window| window == b"%%EOF")
}

fn has_rtf_end(tail: &[u8]) -> bool {
    let text_end = tail.iter().rposition(|byte| !byte.is_ascii_whitespace() && *byte != 0);
    text_end.is_some_and(|at| tail[at] == b'}')
}

fn not_whole(reason: &str) -> String {
    format!("{}: {}", NOT_WHOLE, reason)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn test_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eo-staged-output-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    // A package with two parts, the second one long enough to cut.
    pub fn package_bytes() -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("[Content_Types].xml", options).unwrap();
        writer.write_all(b"<Types/>").unwrap();
        writer.start_file("word/document.xml", options).unwrap();
        for paragraph in 0..2000 {
            writeln!(writer, "<w:p><w:r><w:t>paragraph {}</w:t></w:r></w:p>", paragraph).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    pub const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\nstartxref\n9\n%%EOF\n";
    pub const RTF: &[u8] = b"{\\rtf1\\ansi{\\fonttbl{\\f0 Arial;}}\\pard text\\par\n}\n\0";

    fn check(name: &str, bytes: &[u8]) -> Result<(), String> {
        let path = test_file(name, bytes);
        let result = check_whole(&path);
        let _ = std::fs::remove_file(&path);
        result
    }

    #[test]
    fn a_whole_package_passes() {
        assert_eq!(check("whole.docx", &package_bytes()), Ok(()));
    }

    // What x2t leaves of a DOCX on a full disk: the first blocks.
    #[test]
    fn a_package_cut_short_is_not_whole() {
        let package = package_bytes();

        let error = check("cut.docx", &package[..package.len() / 2]).unwrap_err();

        assert!(error.starts_with(NOT_WHOLE), "{}", error);
    }

    #[test]
    fn a_package_with_a_part_that_does_not_match_its_checksum_is_not_whole() {
        let mut package = package_bytes();
        let inside_document_xml = package.len() / 2;
        package[inside_document_xml] ^= 0xFF;

        let error = check("damaged.odt", &package).unwrap_err();

        assert!(error.starts_with(NOT_WHOLE), "{}", error);
    }

    #[test]
    fn a_whole_pdf_passes() {
        assert_eq!(check("whole.pdf", PDF), Ok(()));
    }

    #[test]
    fn a_pdf_without_its_end_mark_is_not_whole() {
        let error = check("cut.pdf", &PDF[..PDF.len() - 6]).unwrap_err();

        assert_eq!(error, "x2t did not write the whole document: the PDF has no end mark");
    }

    #[test]
    fn a_whole_rtf_passes() {
        assert_eq!(check("whole.rtf", RTF), Ok(()));
    }

    #[test]
    fn an_rtf_without_its_closing_brace_is_not_whole() {
        let error = check("cut.rtf", &RTF[..RTF.len() - 3]).unwrap_err();

        assert_eq!(error, "x2t did not write the whole document: the RTF has no closing brace");
    }

    // A text may start with any bytes, those of a package too.
    #[test]
    fn a_text_file_is_never_checked() {
        let package = package_bytes();

        assert_eq!(check("notes.txt", &package[..package.len() / 2]), Ok(()));
        assert_eq!(check("table.csv", b"%PDF is a format"), Ok(()));
        assert_eq!(check("empty.txt", b""), Ok(()));
    }

    // The legacy binary formats start in another way.
    #[test]
    fn a_format_with_another_start_passes() {
        assert_eq!(check("sheet.xls", b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 workbook"), Ok(()));
    }
}
