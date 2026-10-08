// Import options for CSV and TXT files, detected from the first bytes of the file.
//
// x2t refuses to read CSV or TXT without an encoding and, for CSV, a delimiter,
// and it does not work them out itself: it does not detect the delimiter, and for
// CSV a BOM is not enough for the encoding. A wrong value does not fail either, it
// gives a wrong document (everything in one column, or garbled accents). So they
// are detected here instead of asking: the common cases (UTF-8 or UTF-16 with a
// BOM, UTF-8, windows-1252; comma, semicolon or tab) open with no dialog.

use std::io::Read;
use std::path::Path;

// How much of the file the detection looks at.
pub const HEAD_LEN: usize = 64 * 1024;

// Indices into sdkjs's c_oAscEncodings, which is what x2t expects.
pub const ENCODING_WINDOWS_1252: u32 = 44;
pub const ENCODING_UTF8: u32 = 46;
pub const ENCODING_UTF16_LE: u32 = 48;
pub const ENCODING_UTF16_BE: u32 = 49;

// x2t's delimiter ids (1 tab, 2 semicolon, 3 colon, 4 comma, 5 space).
pub const DELIMITER_TAB: u32 = 1;
pub const DELIMITER_SEMICOLON: u32 = 2;
pub const DELIMITER_COMMA: u32 = 4;

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

// `complete` is false when `head` stops before the end of the file, so a
// multibyte character cut at the end of the block is not taken for invalid UTF-8.
pub fn detect(head: &[u8], complete: bool, kind: TextKind) -> TextOptions {
    let (encoding, body) = detect_encoding(head, complete);
    let delimiter = match kind {
        TextKind::Csv => Some(detect_delimiter(encoding, body)),
        TextKind::Txt => None,
    };
    TextOptions {
        encoding,
        delimiter,
    }
}

// Returns the encoding and the bytes after the BOM, if any.
fn detect_encoding(head: &[u8], complete: bool) -> (u32, &[u8]) {
    if let Some(body) = head.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (ENCODING_UTF8, body);
    }
    if let Some(body) = head.strip_prefix(&[0xFF, 0xFE]) {
        return (ENCODING_UTF16_LE, body);
    }
    if let Some(body) = head.strip_prefix(&[0xFE, 0xFF]) {
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

// The most frequent of comma, semicolon and tab in the first record, outside
// double quotes. A tie, or none of them (a single column), means comma.
fn detect_delimiter(encoding: u32, body: &[u8]) -> u32 {
    // In UTF-8 and windows-1252 every byte below 0x80 is that ASCII character,
    // so only UTF-16 needs decoding into code units to compare against.
    let units: Box<dyn Iterator<Item = u16> + '_> = match encoding {
        ENCODING_UTF16_LE => Box::new(
            body.chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
        ),
        ENCODING_UTF16_BE => Box::new(
            body.chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]])),
        ),
        _ => Box::new(body.iter().map(|&byte| byte as u16)),
    };

    let (mut comma, mut semicolon, mut tab) = (0u32, 0u32, 0u32);
    let mut in_quotes = false;
    for unit in units {
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
                delimiter: Some(DELIMITER_COMMA)
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
                delimiter: Some(DELIMITER_SEMICOLON)
            }
        );
    }

    #[test]
    fn utf16le_with_bom_is_decoded_for_the_delimiter() {
        assert_eq!(
            csv(&utf16le_with_bom("año\tcafé\tx\n1\t2\t3\n")),
            TextOptions {
                encoding: ENCODING_UTF16_LE,
                delimiter: Some(DELIMITER_TAB)
            }
        );
    }

    #[test]
    fn utf16be_with_bom() {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in "a;b;c\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        assert_eq!(
            csv(&bytes),
            TextOptions {
                encoding: ENCODING_UTF16_BE,
                delimiter: Some(DELIMITER_SEMICOLON)
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
                delimiter: Some(DELIMITER_SEMICOLON)
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
                delimiter: None
            }
        );
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
        let options = TextOptions {
            encoding: ENCODING_WINDOWS_1252,
            delimiter: Some(DELIMITER_SEMICOLON),
        };
        assert_eq!(options.for_kind(TextKind::Csv), options);
        assert_eq!(options.for_kind(TextKind::Txt).delimiter, None);
        assert_eq!(
            options.for_kind(TextKind::Txt).encoding,
            ENCODING_WINDOWS_1252
        );
    }

    #[test]
    fn xml_elements_carry_only_numbers() {
        let csv = TextOptions {
            encoding: ENCODING_UTF8,
            delimiter: Some(DELIMITER_SEMICOLON),
        };
        assert_eq!(
            csv.xml_elements(),
            "<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>\n<m_nCsvDelimiter>2</m_nCsvDelimiter>\n"
        );
        let txt = TextOptions {
            encoding: ENCODING_WINDOWS_1252,
            delimiter: None,
        };
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
}
