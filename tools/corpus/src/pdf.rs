// Reading poppler's output: page count from pdfinfo, fonts from pdffonts, and
// the expected phrases in pdftotext's text. PDF bytes are never compared:
// they carry dates, ids and a variable object order.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Font {
    // Without the ABCDEF+ subset prefix.
    pub name: String,
    pub kind: String,
    pub emb: String,
    pub sub: String,
    pub uni: String,
}

impl Font {
    pub fn describe(&self) -> String {
        format!("{}|{}|emb={}|sub={}|uni={}", self.name, self.kind, self.emb, self.sub, self.uni)
    }
}

pub fn pages(pdfinfo: &str) -> Option<u32> {
    pdfinfo
        .lines()
        .find_map(|line| line.strip_prefix("Pages:"))
        .and_then(|value| value.trim().parse().ok())
}

pub fn strip_subset(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => name,
    }
}

// pdffonts prints fixed-width columns under a row of dashes; the dashes give
// the column spans, since the type column holds spaces ("CID TrueType").
pub fn fonts(pdffonts: &str) -> Vec<Font> {
    let mut lines = pdffonts.lines();
    let Some(dashes) = lines.by_ref().find(|line| line.starts_with("---")) else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    let mut start = None;
    for (i, c) in dashes.char_indices().chain(std::iter::once((dashes.len(), ' '))) {
        match (c == '-', start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                spans.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    let cell = |line: &str, index: usize| -> String {
        let Some(&(from, to)) = spans.get(index) else { return String::new() };
        // The last column runs to the end of the line.
        let to = if index + 1 == spans.len() { line.len() } else { to.min(line.len()) };
        line.get(from.min(line.len())..to).unwrap_or("").trim().to_string()
    };
    let mut out: Vec<Font> = lines
        .filter(|line| !line.trim().is_empty())
        .map(|line| Font {
            name: strip_subset(&cell(line, 0)).to_string(),
            kind: cell(line, 1),
            emb: cell(line, 3),
            sub: cell(line, 4),
            uni: cell(line, 5),
        })
        .collect();
    out.sort_by_key(Font::describe);
    out.dedup();
    out
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// The phrases missing from the text, comparing with every run of whitespace
// (line and page breaks included) as one space.
pub fn missing_phrases(text: &str, phrases: &[String]) -> Vec<String> {
    let text = collapse(text);
    phrases
        .iter()
        .filter(|phrase| !text.contains(&collapse(phrase)))
        .cloned()
        .collect()
}

pub const ALLOWED_FAMILIES: [&str; 4] = ["LiberationSans", "LiberationSerif", "LiberationMono", "Carlito"];

// A PDF name with its #xx escapes decoded: the file holds
// "Liberation#20Serif", which pdffonts already prints as "Liberation Serif".
pub fn decode_name(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'#', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// The family of a PDF font name, comparable across spellings: #xx decoded,
// without the ABCDEF+ subset prefix, the style suffix (-BoldItalic, ",Bold")
// or spaces, so "BAAAAA+DejaVuSans-Bold" and "DejaVu Sans" give DejaVuSans.
pub fn family(name: &str) -> String {
    let decoded = decode_name(name);
    let name = strip_subset(&decoded);
    name.split(['-', ',']).next().unwrap_or(name).replace(' ', "")
}

// A PDF font name such as LiberationSerif-BoldItalic belongs to an allowed
// family when its part before the style suffix is one of them.
pub fn allowed(name: &str) -> bool {
    ALLOWED_FAMILIES.contains(&family(name).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PDFFONTS: &str = "\
name                                 type              encoding         emb sub uni object ID
------------------------------------ ----------------- ---------------- --- --- --- ---------
BAAAAA+LiberationSerif               TrueType          WinAnsi          yes yes yes      9  0
CAAAAA+NotoSansCJKjp-Regular         CID Type 0C       Identity-H       yes yes yes     12  0
DejaVuSans                           CID TrueType      Identity-H       no  no  yes     15  0
BAAAAA+LiberationSerif               TrueType          WinAnsi          yes yes yes     20  0
";

    #[test]
    fn pdffonts_columns_are_read_by_their_dashes() {
        let fonts = fonts(PDFFONTS);
        assert_eq!(fonts.len(), 3, "{:?}", fonts);
        assert_eq!(
            fonts[0],
            Font { name: "DejaVuSans".into(), kind: "CID TrueType".into(), emb: "no".into(), sub: "no".into(), uni: "yes".into() }
        );
        assert_eq!(fonts[1].name, "LiberationSerif");
        assert_eq!(fonts[2].kind, "CID Type 0C");
        assert!(super::fonts("Syntax Error: bad").is_empty());
    }

    #[test]
    fn pages_and_subset_prefixes() {
        assert_eq!(pages("Producer:  x\nPages:           12\nEncrypted: no\n"), Some(12));
        assert_eq!(pages("Producer: x\n"), None);
        assert_eq!(strip_subset("ABCDEF+Carlito-Bold"), "Carlito-Bold");
        assert_eq!(strip_subset("Abcdef+X"), "Abcdef+X");
        assert_eq!(strip_subset("Plain"), "Plain");
    }

    #[test]
    fn phrases_survive_line_breaks_and_a_missing_one_is_named() {
        let text = "Justified paragraph begins\nhere and runs\x0c\n  on\n";
        let phrases = vec!["paragraph begins here".to_string(), "runs on".to_string(), "absent text".to_string()];
        assert_eq!(missing_phrases(text, &phrases), vec!["absent text"]);
        // The hotfix/10 case: a PDF with no text at all misses everything.
        assert_eq!(missing_phrases("\x0c\n", &phrases).len(), 3);
    }

    #[test]
    fn font_families_ignore_subset_escapes_and_styles() {
        for name in [
            "BAAAAA+Liberation#20Serif",
            "Liberation Serif",
            "LiberationSerif-BoldItalic",
            "CAAAAA+Liberation#20Serif,Bold",
            "LiberationSerif-Italic",
        ] {
            assert_eq!(family(name), "LiberationSerif", "{}", name);
        }
        assert_eq!(family("BAAAAA+DejaVu#20Sans-Bold"), "DejaVuSans");
        assert_eq!(family("Tahoma,Bold"), "Tahoma");
        assert_eq!(decode_name("A#20B#2"), "A B#2");
        assert_eq!(decode_name("100#"), "100#");
        assert!(allowed("BAAAAA+Liberation#20Serif"));
    }

    #[test]
    fn only_the_bundled_families_are_allowed() {
        for name in ["LiberationSerif", "LiberationSans-BoldItalic", "LiberationMono-Bold", "Carlito-Italic", "Liberation Sans"] {
            assert!(allowed(name), "{}", name);
        }
        for name in ["Arial-BoldMT", "TimesNewRomanPSMT", "Calibri", "DejaVuSans", "LiberationSansNarrow"] {
            assert!(!allowed(name), "{}", name);
        }
    }
}
