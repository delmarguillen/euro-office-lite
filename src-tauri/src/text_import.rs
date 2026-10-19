// Import options for CSV and TXT files, detected from the first bytes of the file.
//
// x2t refuses to read CSV or TXT without an encoding and, for CSV, a delimiter,
// and it does not work them out itself: it does not detect the delimiter, and for
// CSV a BOM is not enough for the encoding. A wrong value does not fail either, it
// gives a wrong document (everything in one column, or garbled accents). So they
// are detected here instead of asking: the common cases (UTF-8 or UTF-16 with a
// BOM, UTF-8, windows-1252; comma, semicolon or tab) open with no dialog.
//
// The same options also undo what x2t changes when it writes the file back: it
// writes every TXT as UTF-8 with a BOM and CRLF, whatever it is asked for, and
// it rewrites line endings and the final line break of a CSV.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

// How much of the file the detection looks at.
pub const HEAD_LEN: usize = 64 * 1024;
// How much of the end of the file is read to count its trailing line breaks.
const TAIL_LEN: u64 = 4096;

// Indices into sdkjs's c_oAscEncodings, which is what x2t expects.
pub const ENCODING_WINDOWS_1252: u32 = 44;
pub const ENCODING_UTF8: u32 = 46;
pub const ENCODING_UTF16_LE: u32 = 48;
pub const ENCODING_UTF16_BE: u32 = 49;

// x2t's delimiter ids (1 tab, 2 semicolon, 3 colon, 4 comma, 5 space).
pub const DELIMITER_TAB: u32 = 1;
pub const DELIMITER_SEMICOLON: u32 = 2;
pub const DELIMITER_COMMA: u32 = 4;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16_BE_BOM: &[u8] = &[0xFE, 0xFF];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Csv,
    Txt,
}

impl TextKind {
    pub fn for_path(path: &Path) -> Option<TextKind> {
        let ext = path.extension()?.to_str()?.to_lowercase();
        match ext.as_str() {
            "csv" => Some(TextKind::Csv),
            "txt" => Some(TextKind::Txt),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextOptions {
    pub encoding: u32,
    // Only CSV has one.
    pub delimiter: Option<u32>,
    // Whether the file started with a UTF-8 BOM. x2t always writes one with
    // UTF-8, so saving back has to drop it when the original had none.
    pub utf8_bom: bool,
    // Line ending style of the original, from its first line break.
    pub crlf: bool,
    // How many line breaks the original ended with.
    pub trailing_breaks: u32,
}

impl TextOptions {
    // The same options seen from a file of another kind: a TXT has no delimiter.
    pub fn for_kind(self, kind: TextKind) -> TextOptions {
        match kind {
            TextKind::Csv => self,
            TextKind::Txt => TextOptions {
                delimiter: None,
                ..self
            },
        }
    }

    // The elements x2t reads them from, on both the reading and the writing side.
    pub fn xml_elements(&self) -> String {
        let mut xml = format!("<m_nCsvTxtEncoding>{}</m_nCsvTxtEncoding>\n", self.encoding);
        if let Some(delimiter) = self.delimiter {
            xml.push_str(&format!(
                "<m_nCsvDelimiter>{}</m_nCsvDelimiter>\n",
                delimiter
            ));
        }
        xml
    }
}

// Reads up to HEAD_LEN bytes and says whether that is the whole file.
pub fn read_head(path: &Path) -> std::io::Result<(Vec<u8>, bool)> {
    let file = std::fs::File::open(path)?;
    let mut head = Vec::with_capacity(HEAD_LEN + 1);
    file.take(HEAD_LEN as u64 + 1).read_to_end(&mut head)?;
    let complete = head.len() <= HEAD_LEN;
    head.truncate(HEAD_LEN);
    Ok((head, complete))
}

// The last bytes of the file, from an even offset so UTF-16 stays aligned.
fn read_tail(path: &Path, len: u64) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut start = size.saturating_sub(len);
    start += start % 2;
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    Ok(tail)
}

// Detection on the whole file: the head decides everything but the trailing
// line breaks, which need the end of a file longer than the head.
pub fn detect_file(path: &Path, kind: TextKind) -> std::io::Result<TextOptions> {
    let (head, complete) = read_head(path)?;
    let mut options = detect(&head, complete, kind);
    if !complete {
        let tail = read_tail(path, TAIL_LEN)?;
        options.trailing_breaks = count_trailing_breaks(&code_units(options.encoding, &tail));
    }
    Ok(options)
}

// `complete` is false when `head` stops before the end of the file, so a
// multibyte character cut at the end of the block is not taken for invalid
// UTF-8. trailing_breaks is only right when `complete` (see detect_file).
pub fn detect(head: &[u8], complete: bool, kind: TextKind) -> TextOptions {
    let (encoding, body) = detect_encoding(head, complete);
    let units = code_units(encoding, body);
    let delimiter = match kind {
        TextKind::Csv => Some(detect_delimiter(&units)),
        TextKind::Txt => None,
    };
    TextOptions {
        encoding,
        delimiter,
        utf8_bom: head.starts_with(UTF8_BOM),
        crlf: first_break_is_crlf(&units),
        trailing_breaks: count_trailing_breaks(&units),
    }
}

// Returns the encoding and the bytes after the BOM, if any.
fn detect_encoding(head: &[u8], complete: bool) -> (u32, &[u8]) {
    if let Some(body) = head.strip_prefix(UTF8_BOM) {
        return (ENCODING_UTF8, body);
    }
    if let Some(body) = head.strip_prefix(UTF16_LE_BOM) {
        return (ENCODING_UTF16_LE, body);
    }
    if let Some(body) = head.strip_prefix(UTF16_BE_BOM) {
        return (ENCODING_UTF16_BE, body);
    }
    let utf8 = match std::str::from_utf8(head) {
        Ok(_) => true,
        // error_len() is None only for a sequence cut short by the end of the
        // input, which is fine when the input is only the start of the file.
        Err(e) => !complete && e.error_len().is_none(),
    };
    let encoding = if utf8 {
        ENCODING_UTF8
    } else {
        ENCODING_WINDOWS_1252
    };
    (encoding, head)
}

// In UTF-8 and windows-1252 every byte below 0x80 is that ASCII character, so
// only UTF-16 needs decoding into code units to find separators and breaks.
fn code_units(encoding: u32, bytes: &[u8]) -> Vec<u16> {
    match encoding {
        ENCODING_UTF16_LE => bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect(),
        ENCODING_UTF16_BE => bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect(),
        _ => bytes.iter().map(|&byte| byte as u16).collect(),
    }
}

// The most frequent of comma, semicolon and tab in the first record, outside
// double quotes. A tie, or none of them (a single column), means comma.
fn detect_delimiter(units: &[u16]) -> u32 {
    let (mut comma, mut semicolon, mut tab) = (0u32, 0u32, 0u32);
    let mut in_quotes = false;
    for &unit in units {
        match unit {
            0x22 => in_quotes = !in_quotes,
            // A line break inside quotes is part of the field, not the record end.
            0x0A | 0x0D if !in_quotes => break,
            0x2C if !in_quotes => comma += 1,
            0x3B if !in_quotes => semicolon += 1,
            0x09 if !in_quotes => tab += 1,
            _ => {}
        }
    }

    if semicolon > comma && semicolon > tab {
        DELIMITER_SEMICOLON
    } else if tab > comma && tab > semicolon {
        DELIMITER_TAB
    } else {
        DELIMITER_COMMA
    }
}

fn first_break_is_crlf(units: &[u16]) -> bool {
    match units.iter().position(|&u| u == 0x0A || u == 0x0D) {
        Some(i) => units[i] == 0x0D && units.get(i + 1) == Some(&0x0A),
        None => false,
    }
}

// CRLF, LF and a lone CR each count as one break.
fn count_trailing_breaks(units: &[u16]) -> u32 {
    let mut count = 0;
    let mut end = units.len();
    while end > 0 {
        match units[end - 1] {
            0x0A => {
                end -= 1;
                if end > 0 && units[end - 1] == 0x0D {
                    end -= 1;
                }
            }
            0x0D => end -= 1,
            _ => break,
        }
        count += 1;
    }
    count
}

// x2t writes TXT as UTF-8 with a BOM and CRLF whatever it is asked for, adds
// or drops a line break at the end depending on its version, and the original
// may have been windows-1252 or UTF-16. So what it wrote is decoded and
// written again the way the original was. Returns the new bytes and how many
// characters windows-1252 could not represent (written as '?').
pub fn rebuild_txt(written: &[u8], options: TextOptions) -> (Vec<u8>, usize) {
    let text = decode_written(written);
    let body = text.trim_end_matches(['\r', '\n']).replace("\r\n", "\n");
    let newline = if options.crlf { "\r\n" } else { "\n" };
    let mut text = if options.crlf {
        body.replace('\n', "\r\n")
    } else {
        body
    };
    for _ in 0..options.trailing_breaks {
        text.push_str(newline);
    }
    encode(&text, options)
}

// Decodes by what x2t actually wrote, not by what it was asked for.
fn decode_written(written: &[u8]) -> String {
    if let Some(rest) = written.strip_prefix(UTF8_BOM) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = written.strip_prefix(UTF16_LE_BOM) {
        String::from_utf16_lossy(&code_units(ENCODING_UTF16_LE, rest))
    } else if let Some(rest) = written.strip_prefix(UTF16_BE_BOM) {
        String::from_utf16_lossy(&code_units(ENCODING_UTF16_BE, rest))
    } else {
        match std::str::from_utf8(written) {
            Ok(text) => text.to_string(),
            Err(_) => written.iter().map(|&b| windows_1252_char(b)).collect(),
        }
    }
}

fn encode(text: &str, options: TextOptions) -> (Vec<u8>, usize) {
    match options.encoding {
        ENCODING_WINDOWS_1252 => {
            let mut unmappable = 0;
            let bytes = text
                .chars()
                .map(|c| {
                    windows_1252_byte(c).unwrap_or_else(|| {
                        unmappable += 1;
                        b'?'
                    })
                })
                .collect();
            (bytes, unmappable)
        }
        ENCODING_UTF16_LE | ENCODING_UTF16_BE => {
            let le = options.encoding == ENCODING_UTF16_LE;
            let mut bytes = if le { UTF16_LE_BOM } else { UTF16_BE_BOM }.to_vec();
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&if le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            (bytes, 0)
        }
        _ => {
            let mut bytes = if options.utf8_bom {
                UTF8_BOM.to_vec()
            } else {
                Vec::new()
            };
            bytes.extend_from_slice(text.as_bytes());
            (bytes, 0)
        }
    }
}

// windows-1252 0x80-0x9F; the five holes keep the C1 control they decode to,
// so they survive a round trip. Everything else is Latin-1.
const WINDOWS_1252_HIGH: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];

fn windows_1252_char(byte: u8) -> char {
    match byte {
        0x80..=0x9F => char::from_u32(WINDOWS_1252_HIGH[(byte - 0x80) as usize] as u32).unwrap(),
        _ => byte as char,
    }
}

fn windows_1252_byte(c: char) -> Option<u8> {
    let code = c as u32;
    if code < 0x80 || (0xA0..=0xFF).contains(&code) {
        return Some(code as u8);
    }
    WINDOWS_1252_HIGH
        .iter()
        .position(|&u| u as u32 == code)
        .map(|i| 0x80 + i as u8)
}

// x2t writes a CSV in the encoding it is asked for, but with LF line endings,
// always a final line break, and a UTF-8 BOM. Only those are put back the way
// the original had them; no row is added or removed and nothing is re-encoded.
pub fn rebuild_csv(written: &[u8], options: TextOptions) -> Vec<u8> {
    let (bom, body, unit_encoding) = if written.starts_with(UTF8_BOM) {
        (UTF8_BOM, &written[3..], ENCODING_UTF8)
    } else if written.starts_with(UTF16_LE_BOM) {
        (UTF16_LE_BOM, &written[2..], ENCODING_UTF16_LE)
    } else if written.starts_with(UTF16_BE_BOM) {
        (UTF16_BE_BOM, &written[2..], ENCODING_UTF16_BE)
    } else {
        (&[][..], written, ENCODING_UTF8)
    };

    let mut units: Vec<u16> = Vec::with_capacity(body.len() + 16);
    let source = code_units(unit_encoding, body);
    // Only record separators change. A break inside quotes is cell content
    // (Excel writes those as LF even in a CRLF file) and stays as written.
    let mut in_quotes = false;
    for (i, &unit) in source.iter().enumerate() {
        if unit == 0x22 {
            in_quotes = !in_quotes;
        }
        if !in_quotes {
            if unit == 0x0D && source.get(i + 1) == Some(&0x0A) {
                continue;
            }
            if unit == 0x0A && options.crlf {
                units.push(0x0D);
            }
        }
        units.push(unit);
    }
    // After an unterminated quote the end is cell content, so it stays as is.
    let ends_with_break = units.last() == Some(&0x0A);
    if !in_quotes && options.trailing_breaks == 0 && ends_with_break {
        units.pop();
        if options.crlf {
            units.pop();
        }
    } else if !in_quotes && options.trailing_breaks > 0 && !ends_with_break && !units.is_empty() {
        if options.crlf {
            units.push(0x0D);
        }
        units.push(0x0A);
    }

    let drop_bom = bom == UTF8_BOM && options.encoding == ENCODING_UTF8 && !options.utf8_bom;
    let mut out = if drop_bom { Vec::new() } else { bom.to_vec() };
    for unit in units {
        match unit_encoding {
            ENCODING_UTF16_LE => out.extend_from_slice(&unit.to_le_bytes()),
            ENCODING_UTF16_BE => out.extend_from_slice(&unit.to_be_bytes()),
            _ => out.push(unit as u8),
        }
    }
    out
}

// Puts a CSV or TXT that x2t just wrote back into the shape of the original.
// Returns how many characters windows-1252 could not represent.
pub fn finish_saved_text(
    path: &Path,
    kind: TextKind,
    options: TextOptions,
) -> std::io::Result<usize> {
    let mut unmappable = 0;
    rewrite_atomically(path, |written| {
        let rebuilt = match kind {
            TextKind::Txt => {
                let (bytes, count) = rebuild_txt(written, options);
                unmappable = count;
                bytes
            }
            TextKind::Csv => rebuild_csv(written, options),
        };
        (rebuilt != written).then_some(rebuilt)
    })?;
    Ok(unmappable)
}

// Rewrites `path` with what `rebuild` returns, if anything. Through a temp file
// and a rename, so a crash mid-rewrite cannot leave the user's file truncated.
// The rename goes over the resolved target, so a symlink stays a symlink.
fn rewrite_atomically(
    path: &Path,
    rebuild: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> std::io::Result<()> {
    let path = &std::fs::canonicalize(path)?;
    let written = std::fs::read(path)?;
    let Some(rebuilt) = rebuild(&written) else {
        return Ok(());
    };
    let tmp = temp_path_for(path);
    let result = write_synced(&tmp, &rebuilt, path).and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

// Next to `path`, so the rename stays on the same filesystem.
fn temp_path_for(path: &Path) -> std::path::PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{}.{}.eo-tmp", name, std::process::id()))
}

fn write_synced(tmp: &Path, bytes: &[u8], original: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    // The rename replaces the file, so it takes the original's permissions.
    std::fs::set_permissions(tmp, std::fs::metadata(original)?.permissions())
}

// x2t's UTF-8 CSV reader adds an empty row after a final line break (it pads
// the decoded text and takes the padding for one more row), so it gets a copy
// without that break. Returns how many bytes to cut from the end, 0 for none.
pub fn final_break_to_cut(kind: TextKind, options: TextOptions, tail: &[u8]) -> u64 {
    if kind != TextKind::Csv || options.encoding != ENCODING_UTF8 {
        return 0;
    }
    if tail.ends_with(b"\r\n") {
        2
    } else if tail.ends_with(b"\n") || tail.ends_with(b"\r") {
        1
    } else {
        0
    }
}

pub fn read_copy_cut(path: &Path, kind: TextKind, options: TextOptions) -> std::io::Result<u64> {
    Ok(final_break_to_cut(kind, options, &read_tail(path, 2)?))
}

pub fn copy_without_final_break(source: &Path, copy: &Path, cut: u64) -> std::io::Result<()> {
    std::fs::copy(source, copy)?;
    let file = std::fs::OpenOptions::new().write(true).open(copy)?;
    let len = file.metadata()?.len();
    file.set_len(len.saturating_sub(cut))
}

// The text of a UTF-16 file, refusing what is not valid UTF-16: replacing it
// would open, and then save, something other than what the file holds.
fn decode_utf16_strict(bytes: &[u8], encoding: u32) -> std::io::Result<String> {
    let invalid = |detail: String| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("not valid UTF-16 text: {}", detail),
        )
    };
    if bytes.len() % 2 != 0 {
        return Err(invalid(format!("odd length of {} bytes", bytes.len())));
    }
    let mut text = String::with_capacity(bytes.len());
    let mut unit = 0;
    for decoded in char::decode_utf16(code_units(encoding, bytes)) {
        match decoded {
            Ok(c) => {
                text.push(c);
                unit += c.len_utf16();
            }
            Err(e) => {
                return Err(invalid(format!(
                    "unpaired surrogate {:04X} at code unit {}",
                    e.unpaired_surrogate(),
                    unit
                )))
            }
        }
    }
    Ok(text)
}

// The UTF-8 copy x2t reads a UTF-16 TXT from. x2t 9.4 loses the text of a
// UTF-16 TXT with a BOM whatever encoding it is given (its TXT reader looks
// the BOM's converter up by a code page number that fails), while UTF-8 reads
// right with x2t 8.2 and 9.4. Saving still goes back to UTF-16 in rebuild_txt.
fn write_utf8_copy(source: &Path, copy: &Path, encoding: u32) -> std::io::Result<()> {
    let bytes = std::fs::read(source)?;
    let bom = if encoding == ENCODING_UTF16_LE { UTF16_LE_BOM } else { UTF16_BE_BOM };
    let text = decode_utf16_strict(bytes.strip_prefix(bom).unwrap_or(&bytes), encoding)?;
    // x2t drops a leading EF BB BF as a BOM, so a text that starts with U+FEFF
    // gets a BOM of its own in front. So does an empty one: x2t refuses an
    // empty TXT (exit 88) but opens a lone BOM as an empty document.
    let mut utf8 = Vec::with_capacity(text.len() + UTF8_BOM.len());
    if text.is_empty() || text.starts_with('\u{FEFF}') {
        utf8.extend_from_slice(UTF8_BOM);
    }
    utf8.extend_from_slice(text.as_bytes());
    std::fs::write(copy, utf8)
}

// A copy of the original for x2t to read, and the options to read it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadCopy {
    pub path: std::path::PathBuf,
    pub options: TextOptions,
}

// Copy of a CSV or TXT for x2t to read, in `temp_dir`, when the original needs
// one. Takes the folder instead of the app state so the corpus can open files
// the same way.
pub fn prepare_read_copy(
    source: &Path,
    temp_dir: &Path,
    kind: Option<TextKind>,
    options: Option<TextOptions>,
) -> std::io::Result<Option<ReadCopy>> {
    let (Some(kind), Some(options)) = (kind, options) else {
        return Ok(None);
    };
    if kind == TextKind::Txt
        && (options.encoding == ENCODING_UTF16_LE || options.encoding == ENCODING_UTF16_BE)
    {
        let copy = temp_dir.join("eo-read-copy.txt");
        write_utf8_copy(source, &copy, options.encoding)?;
        return Ok(Some(ReadCopy {
            path: copy,
            options: TextOptions {
                encoding: ENCODING_UTF8,
                ..options
            },
        }));
    }
    let cut = read_copy_cut(source, kind, options)?;
    if cut == 0 {
        return Ok(None);
    }
    let copy = temp_dir.join("eo-read-copy.csv");
    copy_without_final_break(source, &copy, cut)?;
    Ok(Some(ReadCopy {
        path: copy,
        options,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv(bytes: &[u8]) -> TextOptions {
        detect(bytes, true, TextKind::Csv)
    }

    fn utf16le_with_bom(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    fn utf16be_with_bom(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        bytes
    }

    fn options(encoding: u32, utf8_bom: bool, crlf: bool, trailing_breaks: u32) -> TextOptions {
        TextOptions {
            encoding,
            delimiter: Some(DELIMITER_SEMICOLON),
            utf8_bom,
            crlf,
            trailing_breaks,
        }
    }

    fn utf8(utf8_bom: bool) -> TextOptions {
        options(ENCODING_UTF8, utf8_bom, false, 1)
    }

    fn txt(written: &str, options: TextOptions) -> Vec<u8> {
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice(written.as_bytes());
        rebuild_txt(&bytes, options).0
    }

    // ---- detection ----

    #[test]
    fn comma_separated() {
        assert_eq!(csv(b"a,b,c\n1,2,3\n").delimiter, Some(DELIMITER_COMMA));
    }

    #[test]
    fn semicolon_separated() {
        // The usual export of a spreadsheet in locales with a decimal comma.
        assert_eq!(
            csv(b"name;price;qty\nfoo;1,50;3\n").delimiter,
            Some(DELIMITER_SEMICOLON)
        );
    }

    #[test]
    fn tab_separated() {
        assert_eq!(csv(b"a\tb\tc\n1\t2\t3\n").delimiter, Some(DELIMITER_TAB));
    }

    #[test]
    fn separators_inside_quotes_do_not_count() {
        // Four commas inside quotes, two semicolons outside.
        assert_eq!(
            csv(b"\"Smith, John, Jr.\";\"a,b,c\";x\n").delimiter,
            Some(DELIMITER_SEMICOLON)
        );
    }

    #[test]
    fn a_quoted_line_break_does_not_end_the_first_record() {
        assert_eq!(
            csv(b"\"multi\nline\";b;c\n1,2,3,4,5,6\n").delimiter,
            Some(DELIMITER_SEMICOLON)
        );
    }

    #[test]
    fn only_the_first_record_is_counted() {
        assert_eq!(
            csv(b"a;b;c\r\n1,5;2,5;3,5\r\n").delimiter,
            Some(DELIMITER_SEMICOLON)
        );
    }

    #[test]
    fn a_tie_falls_back_to_comma() {
        assert_eq!(csv(b"a;b,c\n").delimiter, Some(DELIMITER_COMMA));
    }

    #[test]
    fn a_single_column_is_comma() {
        assert_eq!(csv(b"name\nfoo\nbar\n").delimiter, Some(DELIMITER_COMMA));
    }

    #[test]
    fn an_empty_file_is_utf8_and_comma() {
        assert_eq!(
            csv(b""),
            TextOptions {
                encoding: ENCODING_UTF8,
                delimiter: Some(DELIMITER_COMMA),
                utf8_bom: false,
                crlf: false,
                trailing_breaks: 0
            }
        );
    }

    #[test]
    fn utf8_without_bom() {
        assert_eq!(csv("año;café\n".as_bytes()).encoding, ENCODING_UTF8);
    }

    #[test]
    fn utf8_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("año;café\n".as_bytes());
        assert_eq!(
            csv(&bytes),
            TextOptions {
                encoding: ENCODING_UTF8,
                delimiter: Some(DELIMITER_SEMICOLON),
                utf8_bom: true,
                crlf: false,
                trailing_breaks: 1
            }
        );
    }

    #[test]
    fn utf16le_with_bom_is_decoded_for_the_delimiter() {
        assert_eq!(
            csv(&utf16le_with_bom("año\tcafé\tx\r\n1\t2\t3\r\n\r\n")),
            TextOptions {
                encoding: ENCODING_UTF16_LE,
                delimiter: Some(DELIMITER_TAB),
                utf8_bom: false,
                crlf: true,
                trailing_breaks: 2
            }
        );
    }

    #[test]
    fn utf16be_with_bom_is_detected() {
        assert_eq!(
            csv(&utf16be_with_bom("a;b;c\n")),
            TextOptions {
                encoding: ENCODING_UTF16_BE,
                delimiter: Some(DELIMITER_SEMICOLON),
                utf8_bom: false,
                crlf: false,
                trailing_breaks: 1
            }
        );
    }

    #[test]
    fn windows_1252_with_accents() {
        // "año;café" in windows-1252: ñ = 0xF1, é = 0xE9.
        let bytes = b"a\xF1o;caf\xE9\n";
        assert_eq!(
            csv(bytes),
            TextOptions {
                encoding: ENCODING_WINDOWS_1252,
                delimiter: Some(DELIMITER_SEMICOLON),
                utf8_bom: false,
                crlf: false,
                trailing_breaks: 1
            }
        );
    }

    #[test]
    fn a_multibyte_character_cut_by_the_block_end_is_still_utf8() {
        // "é" is C3 A9; the block ends right after C3.
        let bytes = b"a,b\ncaf\xC3";
        assert_eq!(detect(bytes, false, TextKind::Csv).encoding, ENCODING_UTF8);
    }

    #[test]
    fn a_truncated_sequence_at_the_real_end_of_the_file_is_not_utf8() {
        // A windows-1252 file that ends in "é" (E9, a UTF-8 lead byte).
        let bytes = b"a,b\ncaf\xE9";
        assert_eq!(
            detect(bytes, true, TextKind::Csv).encoding,
            ENCODING_WINDOWS_1252
        );
    }

    #[test]
    fn txt_has_no_delimiter() {
        assert_eq!(
            detect(b"a;b;c\n", true, TextKind::Txt),
            TextOptions {
                encoding: ENCODING_UTF8,
                delimiter: None,
                utf8_bom: false,
                crlf: false,
                trailing_breaks: 1
            }
        );
    }

    #[test]
    fn line_ending_style_and_trailing_breaks() {
        let t = |bytes: &[u8]| {
            let o = detect(bytes, true, TextKind::Txt);
            (o.crlf, o.trailing_breaks)
        };
        assert_eq!(t(b"a\nb"), (false, 0));
        assert_eq!(t(b"a\r\nb\r\n"), (true, 1));
        assert_eq!(t(b"a\n\n"), (false, 2));
        assert_eq!(t(b"a\r\n\r\n\r\n"), (true, 3));
        assert_eq!(t(b"a"), (false, 0));
        assert_eq!(t(b"\n"), (false, 1));
    }

    #[test]
    fn kind_comes_from_the_extension_in_any_case() {
        assert_eq!(
            TextKind::for_path(Path::new("x/a.CSV")),
            Some(TextKind::Csv)
        );
        assert_eq!(TextKind::for_path(Path::new("a.txt")), Some(TextKind::Txt));
        assert_eq!(TextKind::for_path(Path::new("a.xlsx")), None);
        assert_eq!(TextKind::for_path(Path::new("csv")), None);
    }

    #[test]
    fn csv_options_seen_from_a_txt_drop_the_delimiter() {
        let options = options(ENCODING_WINDOWS_1252, false, true, 2);
        assert_eq!(options.for_kind(TextKind::Csv), options);
        assert_eq!(options.for_kind(TextKind::Txt).delimiter, None);
        assert_eq!(
            options.for_kind(TextKind::Txt),
            TextOptions {
                delimiter: None,
                ..options
            }
        );
    }

    #[test]
    fn xml_elements_carry_only_numbers() {
        let csv = utf8(false);
        assert_eq!(
            csv.xml_elements(),
            "<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>\n<m_nCsvDelimiter>2</m_nCsvDelimiter>\n"
        );
        let txt = options(ENCODING_WINDOWS_1252, false, false, 0).for_kind(TextKind::Txt);
        assert_eq!(
            txt.xml_elements(),
            "<m_nCsvTxtEncoding>44</m_nCsvTxtEncoding>\n"
        );
    }

    #[test]
    fn read_head_reports_whether_it_saw_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("eo-text-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let small = dir.join("small.csv");
        let large = dir.join("large.csv");
        std::fs::write(&small, b"a;b\n").unwrap();
        std::fs::write(&large, vec![b'x'; HEAD_LEN + 10]).unwrap();

        let (head, complete) = read_head(&small).unwrap();
        assert_eq!((head.as_slice(), complete), (&b"a;b\n"[..], true));
        let (head, complete) = read_head(&large).unwrap();
        assert_eq!((head.len(), complete), (HEAD_LEN, false));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_file_counts_trailing_breaks_past_the_head() {
        let dir = std::env::temp_dir().join(format!("eo-text-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.txt");
        let mut bytes = b"line\r\n".repeat(HEAD_LEN / 6 + 100);
        bytes.extend_from_slice(b"last\r\n\r\n");
        std::fs::write(&path, &bytes).unwrap();

        let options = detect_file(&path, TextKind::Txt).unwrap();
        assert_eq!((options.crlf, options.trailing_breaks), (true, 2));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- TXT after x2t ----

    #[test]
    fn txt_gets_its_line_endings_and_final_breaks_back() {
        // Linux x2t adds a break at the end, Windows x2t drops one; both write CRLF.
        let lf = |n| options(ENCODING_UTF8, false, false, n);
        assert_eq!(txt("a\r\nb\r\n\r\n", lf(1)), b"a\nb\n");
        assert_eq!(txt("a\r\nb", lf(1)), b"a\nb\n");
        assert_eq!(txt("a\r\nb\r\n", lf(0)), b"a\nb");
        assert_eq!(txt("a\r\n\r\n", lf(2)), b"a\n\n");
        let crlf = |n| options(ENCODING_UTF8, false, true, n);
        assert_eq!(txt("a\r\nb\r\n\r\n", crlf(1)), b"a\r\nb\r\n");
        assert_eq!(txt("a\r\nb", crlf(0)), b"a\r\nb");
    }

    #[test]
    fn txt_keeps_or_drops_the_utf8_bom_like_the_original() {
        assert_eq!(
            txt("a\r\n", options(ENCODING_UTF8, true, false, 1)),
            b"\xEF\xBB\xBFa\n"
        );
        assert_eq!(
            txt("a\r\n", options(ENCODING_UTF8, false, false, 1)),
            b"a\n"
        );
    }

    #[test]
    fn txt_goes_back_to_windows_1252() {
        let (bytes, unmappable) = rebuild_txt(
            "\u{FEFF}año café € ‰\r\n".as_bytes(),
            options(ENCODING_WINDOWS_1252, false, false, 1),
        );
        assert_eq!(bytes, b"a\xF1o caf\xE9 \x80 \x89\n");
        assert_eq!(unmappable, 0);
    }

    #[test]
    fn characters_windows_1252_lacks_become_question_marks() {
        let (bytes, unmappable) = rebuild_txt(
            "\u{FEFF}a ж 😀\r\n".as_bytes(),
            options(ENCODING_WINDOWS_1252, false, false, 1),
        );
        assert_eq!(bytes, b"a ? ?\n");
        assert_eq!(unmappable, 2);
    }

    #[test]
    fn windows_1252_round_trips_every_byte() {
        let all: Vec<u8> = (0u8..=255).collect();
        let text: String = all.iter().map(|&b| windows_1252_char(b)).collect();
        let back: Vec<u8> = text
            .chars()
            .map(|c| windows_1252_byte(c).unwrap())
            .collect();
        assert_eq!(back, all);
    }

    #[test]
    fn txt_goes_back_to_utf16() {
        let le = rebuild_txt(
            "\u{FEFF}añ\r\n".as_bytes(),
            options(ENCODING_UTF16_LE, false, false, 1),
        )
        .0;
        assert_eq!(le, utf16le_with_bom("añ\n"));
        let be = rebuild_txt(
            "\u{FEFF}añ\r\n".as_bytes(),
            options(ENCODING_UTF16_BE, false, true, 1),
        )
        .0;
        assert_eq!(be, utf16be_with_bom("añ\r\n"));
    }

    #[test]
    fn txt_decodes_whatever_x2t_wrote() {
        let o = options(ENCODING_UTF8, false, false, 1);
        assert_eq!(
            rebuild_txt(&utf16le_with_bom("é\r\n"), o).0,
            "é\n".as_bytes()
        );
        assert_eq!(rebuild_txt(b"caf\xE9\r\n", o).0, "café\n".as_bytes());
        assert_eq!(rebuild_txt("café\r\n".as_bytes(), o).0, "café\n".as_bytes());
    }

    // ---- CSV after x2t ----

    #[test]
    fn csv_gets_crlf_back() {
        let o = options(ENCODING_UTF8, false, true, 1);
        assert_eq!(rebuild_csv(b"\xEF\xBB\xBFa;b\nc;d\n", o), b"a;b\r\nc;d\r\n");
    }

    #[test]
    fn a_line_break_inside_a_quoted_cell_is_left_as_written() {
        let crlf = options(ENCODING_UTF8, false, true, 1);
        assert_eq!(
            rebuild_csv(b"\"line 1\nline 2\";x\nc;d\n", crlf),
            b"\"line 1\nline 2\";x\r\nc;d\r\n"
        );
        let lf = options(ENCODING_UTF8, false, false, 1);
        assert_eq!(
            rebuild_csv(b"\"line 1\r\nline 2\";x\r\nc;d\r\n", lf),
            b"\"line 1\r\nline 2\";x\nc;d\n"
        );
    }

    #[test]
    fn escaped_quotes_do_not_break_the_quote_tracking() {
        let crlf = options(ENCODING_UTF8, false, true, 1);
        assert_eq!(
            rebuild_csv(b"\"a \"\"q\"\"\nb\";x\nc;d\n", crlf),
            b"\"a \"\"q\"\"\nb\";x\r\nc;d\r\n"
        );
        // The final break is a record separator only outside quotes.
        let none = options(ENCODING_UTF8, false, false, 0);
        assert_eq!(rebuild_csv(b"a;\"b\n", none), b"a;\"b\n");
    }

    #[test]
    fn csv_final_break_follows_the_original() {
        let none = options(ENCODING_UTF8, false, false, 0);
        assert_eq!(rebuild_csv(b"a;b\nc;d\n", none), b"a;b\nc;d");
        let crlf_none = options(ENCODING_UTF8, false, true, 0);
        assert_eq!(rebuild_csv(b"a;b\nc;d\n", crlf_none), b"a;b\r\nc;d");
        let one = options(ENCODING_UTF8, false, false, 1);
        assert_eq!(rebuild_csv(b"a;b\nc;d", one), b"a;b\nc;d\n");
        // Two breaks in the original still mean one: rows are never added.
        let two = options(ENCODING_UTF8, false, false, 2);
        assert_eq!(rebuild_csv(b"a;b\n", two), b"a;b\n");
    }

    #[test]
    fn csv_rows_and_blank_lines_are_kept() {
        let o = options(ENCODING_UTF8, false, false, 1);
        assert_eq!(rebuild_csv(b"a;b\n;\n\nc;d\n", o), b"a;b\n;\n\nc;d\n");
    }

    #[test]
    fn csv_is_never_re_encoded() {
        let o = options(ENCODING_WINDOWS_1252, false, true, 1);
        assert_eq!(rebuild_csv(b"caf\xE9;1\n", o), b"caf\xE9;1\r\n");
        let le = options(ENCODING_UTF16_LE, false, true, 1);
        assert_eq!(
            rebuild_csv(&utf16le_with_bom("é;1\n"), le),
            utf16le_with_bom("é;1\r\n")
        );
    }

    #[test]
    fn the_bom_x2t_adds_goes_when_the_original_had_none() {
        assert_eq!(rebuild_csv(b"\xEF\xBB\xBFa;b\n", utf8(false)), b"a;b\n");
    }

    #[test]
    fn the_bom_stays_when_the_original_had_one() {
        assert_eq!(
            rebuild_csv(b"\xEF\xBB\xBFa;b\n", utf8(true)),
            b"\xEF\xBB\xBFa;b\n"
        );
    }

    #[test]
    fn nothing_changes_when_x2t_wrote_the_original_shape() {
        assert_eq!(rebuild_csv(b"a;b\n", utf8(false)), b"a;b\n");
        assert_eq!(rebuild_csv(b"", utf8(false)), b"");
    }

    // ---- the rewrite ----

    #[test]
    fn finish_saved_text_rewrites_the_file_in_place() {
        let dir = std::env::temp_dir().join(format!("eo-text-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let with_bom = dir.join("with.csv");
        let empty = dir.join("empty.csv");
        let text = dir.join("text.txt");
        std::fs::write(&with_bom, b"\xEF\xBB\xBFa;b\n").unwrap();
        std::fs::write(&empty, b"").unwrap();
        std::fs::write(&text, "\u{FEFF}é ж\r\n\r\n".as_bytes()).unwrap();

        finish_saved_text(&with_bom, TextKind::Csv, utf8(false)).unwrap();
        finish_saved_text(&empty, TextKind::Csv, utf8(false)).unwrap();
        let o = options(ENCODING_WINDOWS_1252, false, false, 1);
        assert_eq!(finish_saved_text(&text, TextKind::Txt, o).unwrap(), 1);
        assert_eq!(std::fs::read(&with_bom).unwrap(), b"a;b\n");
        assert_eq!(std::fs::read(&empty).unwrap(), b"");
        assert_eq!(std::fs::read(&text).unwrap(), b"\xE9 ?\n");

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["empty.csv", "text.txt", "with.csv"],
            "no temp file left behind"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_rewritten_through_the_link() {
        let dir = std::env::temp_dir().join(format!("eo-text-bom-link-{}", std::process::id()));
        let target_dir = dir.join("real");
        std::fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("data.csv");
        let link = dir.join("link.csv");
        std::fs::write(&target, b"\xEF\xBB\xBFa;b\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        finish_saved_text(&link, TextKind::Csv, utf8(false)).unwrap();

        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"a;b\n");
        let left: Vec<String> = std::fs::read_dir(&target_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, ["data.csv"], "no temp file left next to the target");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_rewrite_leaves_the_saved_file_intact() {
        // A directory squatting on the temp name makes File::create fail.
        let dir = std::env::temp_dir().join(format!("eo-text-bom-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = dir.join("saved.csv");
        std::fs::write(&saved, b"\xEF\xBB\xBFa;b\n").unwrap();
        let squatter = temp_path_for(&std::fs::canonicalize(&saved).unwrap());
        std::fs::create_dir_all(&squatter).unwrap();

        assert!(finish_saved_text(&saved, TextKind::Csv, utf8(false)).is_err());
        assert_eq!(std::fs::read(&saved).unwrap(), b"\xEF\xBB\xBFa;b\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- the read copy for x2t's UTF-8 CSV reader ----

    #[test]
    fn a_utf8_csv_loses_its_final_break_for_x2t() {
        let o = utf8(false);
        assert_eq!(final_break_to_cut(TextKind::Csv, o, b"d\n"), 1);
        assert_eq!(final_break_to_cut(TextKind::Csv, o, b"d\r\n"), 2);
        assert_eq!(final_break_to_cut(TextKind::Csv, o, b"d\r"), 1);
    }

    #[test]
    fn no_copy_without_a_final_break_or_for_other_encodings_or_txt() {
        assert_eq!(final_break_to_cut(TextKind::Csv, utf8(false), b"cd"), 0);
        assert_eq!(final_break_to_cut(TextKind::Csv, utf8(false), b""), 0);
        let w1252 = options(ENCODING_WINDOWS_1252, false, false, 1);
        assert_eq!(final_break_to_cut(TextKind::Csv, w1252, b"d\n"), 0);
        let le = options(ENCODING_UTF16_LE, false, false, 1);
        assert_eq!(final_break_to_cut(TextKind::Csv, le, b"\n\x00"), 0);
        assert_eq!(final_break_to_cut(TextKind::Txt, utf8(false), b"d\n"), 0);
    }

    #[test]
    fn the_read_copy_is_the_file_minus_its_final_break() {
        let dir = std::env::temp_dir().join(format!("eo-text-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("data.csv");
        let copy = dir.join("copy.csv");
        std::fs::write(&source, b"a;b\r\nc;d\r\n").unwrap();

        let cut = read_copy_cut(&source, TextKind::Csv, utf8(false)).unwrap();
        copy_without_final_break(&source, &copy, cut).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"a;b\r\nc;d");
        assert_eq!(std::fs::read(&source).unwrap(), b"a;b\r\nc;d\r\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn read_copy_dirs(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("eo-read-copy-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let temp = dir.join("temp");
        std::fs::create_dir_all(&temp).unwrap();
        (dir, temp)
    }

    fn try_prepared(source: &Path, temp: &Path) -> std::io::Result<Option<ReadCopy>> {
        let kind = TextKind::for_path(source);
        let options = kind.map(|kind| detect_file(source, kind).unwrap());
        prepare_read_copy(source, temp, kind, options)
    }

    fn prepared(source: &Path, temp: &Path) -> Option<ReadCopy> {
        try_prepared(source, temp).unwrap()
    }

    #[test]
    fn a_utf8_csv_with_a_final_break_is_read_from_a_copy_in_the_temp_dir() {
        let (dir, temp) = read_copy_dirs("lf");
        let source = dir.join("data.csv");
        std::fs::write(&source, b"a,b\nc,d\n").unwrap();

        let copy = prepared(&source, &temp).unwrap();

        assert_eq!(copy.path, temp.join("eo-read-copy.csv"));
        assert_eq!(copy.options, detect_file(&source, TextKind::Csv).unwrap());
        assert_eq!(std::fs::read(temp.join("eo-read-copy.csv")).unwrap(), b"a,b\nc,d");
        assert_eq!(std::fs::read(&source).unwrap(), b"a,b\nc,d\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_read_copy_without_a_final_break_in_1252_or_for_other_files() {
        let (dir, temp) = read_copy_dirs("none");
        let cases: [(&str, &[u8]); 4] = [
            ("nofinal.csv", b"a,b\nc,d"),
            ("latin.csv", b"a\xF1o;caf\xE9\n"),
            ("notes.txt", b"a\nb\n"),
            ("book.docx", b"PK"),
        ];
        for (name, bytes) in cases {
            let source = dir.join(name);
            std::fs::write(&source, bytes).unwrap();
            assert_eq!(prepared(&source, &temp), None, "{}", name);
        }
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "nothing written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_utf16_txt_is_read_from_a_utf8_copy() {
        let (dir, temp) = read_copy_dirs("utf16");
        let text = "Hello\nÜber alles\n";
        for (name, bytes) in [("le.txt", utf16le_with_bom(text)), ("be.txt", utf16be_with_bom(text))] {
            let source = dir.join(name);
            std::fs::write(&source, &bytes).unwrap();
            let copy = prepared(&source, &temp).expect(name);
            assert_eq!(copy.path, temp.join("eo-read-copy.txt"));
            assert_eq!(std::fs::read(&copy.path).unwrap(), text.as_bytes(), "{}", name);
            assert_eq!(std::fs::read(&source).unwrap(), bytes, "the original is left alone");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn x2t_reads_the_copy_as_utf8_and_the_save_still_goes_back_to_utf16() {
        let (dir, temp) = read_copy_dirs("utf16-options");
        let original = utf16le_with_bom("Línea uno\r\nÜber alles €\r\n");
        let source = dir.join("notepad.txt");
        std::fs::write(&source, &original).unwrap();
        let options = detect_file(&source, TextKind::Txt).unwrap();

        let copy = prepared(&source, &temp).unwrap();

        assert_eq!(options.encoding, ENCODING_UTF16_LE, "the save keeps the original's");
        assert_eq!(copy.options, TextOptions { encoding: ENCODING_UTF8, ..options });
        assert_eq!(copy.options.xml_elements(), "<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>\n");
        // What x2t 9.4 (CRLF, no final break) and 8.2 (one more break) write
        // back is turned into the original bytes.
        for written in ["\u{FEFF}Línea uno\r\nÜber alles €", "\u{FEFF}Línea uno\r\nÜber alles €\r\n\r\n"] {
            assert_eq!(rebuild_txt(written.as_bytes(), options).0, original);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_utf8_copy_keeps_line_endings_and_characters_outside_the_bmp() {
        let (dir, temp) = read_copy_dirs("utf16-wide");
        let text = "a\r\nж 中 😀\rb\n\n";
        let source = dir.join("wide.txt");
        std::fs::write(&source, utf16be_with_bom(text)).unwrap();

        let copy = prepared(&source, &temp).unwrap();

        assert_eq!(std::fs::read(&copy.path).unwrap(), text.as_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_utf16_txt_is_read_from_a_lone_utf8_bom() {
        let (dir, temp) = read_copy_dirs("utf16-empty");
        let source = dir.join("empty.txt");
        std::fs::write(&source, UTF16_LE_BOM).unwrap();

        let copy = prepared(&source, &temp).unwrap();

        assert_eq!(std::fs::read(&copy.path).unwrap(), UTF8_BOM);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // What x2t reads from a UTF-8 copy: everything after one leading BOM.
    fn x2t_reads(copy: &[u8]) -> String {
        String::from_utf8(copy.strip_prefix(UTF8_BOM).unwrap_or(copy).to_vec()).unwrap()
    }

    #[test]
    fn a_leading_u_feff_survives_the_copy_and_the_save() {
        let (dir, temp) = read_copy_dirs("utf16-feff");
        let cases: [(&str, &str); 3] = [
            ("bom only", ""),
            ("bom and U+FEFF", "\u{FEFF}"),
            ("bom, U+FEFF and text", "\u{FEFF}Hi\nthere\n"),
        ];
        for (case, text) in cases {
            for original in [utf16le_with_bom(text), utf16be_with_bom(text)] {
                let source = dir.join("feff.txt");
                std::fs::write(&source, &original).unwrap();
                let options = detect_file(&source, TextKind::Txt).unwrap();
                let copy = prepared(&source, &temp).unwrap();
                let read = x2t_reads(&std::fs::read(&copy.path).unwrap());
                assert_eq!(read, text, "{}: x2t reads the text", case);
                // x2t 9.4 writes CRLF without the final break, 8.2 adds one.
                let body = read.trim_end_matches('\n').replace('\n', "\r\n");
                for written in [format!("\u{FEFF}{}", body), format!("\u{FEFF}{}\r\n\r\n", body)] {
                    assert_eq!(rebuild_txt(written.as_bytes(), options).0, original, "{}", case);
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_utf16_fails_the_open_instead_of_being_replaced() {
        let (dir, temp) = read_copy_dirs("utf16-invalid");
        let mut odd = utf16le_with_bom("ab");
        odd.push(b'c');
        // A high surrogate followed by "a" instead of its low half.
        let mut lone = utf16le_with_bom("x");
        lone.extend_from_slice(&[0x00, 0xD8, 0x61, 0x00]);
        let cases: [(&str, Vec<u8>, &str); 2] = [
            ("odd.txt", odd, "odd length of 5 bytes"),
            ("lone.txt", lone, "unpaired surrogate D800 at code unit 1"),
        ];
        for (name, bytes, detail) in cases {
            let source = dir.join(name);
            std::fs::write(&source, &bytes).unwrap();
            let error = try_prepared(&source, &temp).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{}", name);
            assert_eq!(error.to_string(), format!("not valid UTF-16 text: {}", detail));
        }
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "no copy left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_utf16_csv_is_still_read_from_the_original() {
        let (dir, temp) = read_copy_dirs("utf16-csv");
        let source = dir.join("data.csv");
        std::fs::write(&source, utf16le_with_bom("a\tb\r\n1\t2\r\n")).unwrap();

        assert_eq!(prepared(&source, &temp), None);
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "nothing written");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
