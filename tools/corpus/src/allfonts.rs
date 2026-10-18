// Reads the font list x2t writes into AllFonts.js, without running any JS:
//   window["__fonts_files"] = [ "path", ... ];
//   window["__fonts_infos"] = [ ["Name", iR, fR, iI, fI, iB, fB, iBI, fBI], ... ];
// Each info row is one family; the i* are indices into __fonts_files, -1 when
// the family has no face in that style. That file is the one a PDF embeds.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontInfo {
    pub name: String,
    // Indices of the regular, italic, bold and bold italic files.
    pub faces: [i64; 4],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontList {
    pub files: Vec<String>,
    pub infos: Vec<FontInfo>,
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Str(String),
    Int(i64),
    List(Vec<Value>),
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip_space();
        match self.bytes.get(self.pos) {
            Some(b'"') => self.string().map(Value::Str),
            Some(b'[') => self.list().map(Value::List),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.int().map(Value::Int),
            other => Err(format!("unexpected {:?} at byte {}", other.map(|c| *c as char), self.pos)),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut out = Vec::new();
        while let Some(&c) = self.bytes.get(self.pos) {
            self.pos += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|e| e.to_string()),
                b'\\' => {
                    let escaped = *self.bytes.get(self.pos).ok_or("unterminated escape")?;
                    self.pos += 1;
                    out.push(escaped);
                }
                _ => out.push(c),
            }
        }
        Err("unterminated string".to_string())
    }

    fn int(&mut self) -> Result<i64, String> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .unwrap_or("")
            .parse()
            .map_err(|_| format!("bad number at byte {}", start))
    }

    fn list(&mut self) -> Result<Vec<Value>, String> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_space();
            match self.bytes.get(self.pos) {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(items);
                }
                Some(b',') => self.pos += 1,
                Some(_) => items.push(self.value()?),
                None => return Err("unterminated list".to_string()),
            }
        }
    }
}

// The array assigned to window["<name>"].
fn array_after(text: &str, name: &str) -> Result<Vec<Value>, String> {
    let marker = format!("window[\"{}\"]", name);
    let start = text.find(&marker).ok_or_else(|| format!("{} not found", marker))?;
    let rest = &text[start + marker.len()..];
    let open = rest.find('[').ok_or_else(|| format!("{} has no array", marker))?;
    let mut parser = Parser {
        bytes: rest[open..].as_bytes(),
        pos: 0,
    };
    parser.list()
}

pub fn parse(text: &str) -> Result<FontList, String> {
    let files = array_after(text, "__fonts_files")?
        .into_iter()
        .map(|value| match value {
            Value::Str(path) => Ok(path),
            other => Err(format!("__fonts_files holds {:?}", other)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut infos = Vec::new();
    for row in array_after(text, "__fonts_infos")? {
        let Value::List(cells) = row else {
            return Err("__fonts_infos row is not a list".to_string());
        };
        let name = match cells.first() {
            Some(Value::Str(name)) => name.clone(),
            _ => return Err("__fonts_infos row without a name".to_string()),
        };
        let index = |i: usize| match cells.get(i) {
            Some(Value::Int(n)) => Ok(*n),
            _ => Err(format!("__fonts_infos row {} lacks field {}", name, i)),
        };
        infos.push(FontInfo {
            faces: [index(1)?, index(3)?, index(5)?, index(7)?],
            name,
        });
    }
    Ok(FontList { files, infos })
}

// The spelling x2t would write for a path: forward slashes, without the
// extended prefix in either form.
fn plain(path: &str) -> String {
    let path = path.replace('\\', "/");
    path.strip_prefix("//?/")
        .or_else(|| path.strip_prefix("/?/"))
        .unwrap_or(&path)
        .to_string()
}

// "bundled" for a file in the app's binaries/fonts, "system" for anything
// else: a system copy of a bundled family takes over its row.
pub fn origin(path: &str, binaries_dir: &str) -> &'static str {
    let prefix = format!("{}/fonts/", plain(binaries_dir).trim_end_matches('/'));
    if plain(path).starts_with(&prefix) {
        "bundled"
    } else {
        "system"
    }
}

pub const CHECKED_FAMILIES: [&str; 4] = ["Liberation Sans", "Liberation Serif", "Liberation Mono", "Carlito"];
pub const STYLES: [&str; 4] = ["regular", "italic", "bold", "bold-italic"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub family: String,
    pub style: String,
    pub path: String,
    // bundled, system or missing.
    pub origin: String,
}

// Which file each of the families the comparable documents use resolves to.
pub fn check(list: &FontList, binaries_dir: &str) -> Vec<Check> {
    let mut rows = Vec::new();
    for family in CHECKED_FAMILIES {
        let info = list.infos.iter().find(|info| info.name == family);
        for (style_index, style) in STYLES.iter().enumerate() {
            let path = info
                .map(|info| info.faces[style_index])
                .filter(|index| *index >= 0)
                .and_then(|index| list.files.get(index as usize));
            let (path, origin) = match path {
                Some(path) => (path.clone(), origin(path, binaries_dir).to_string()),
                None => (String::new(), "missing".to_string()),
            };
            rows.push(Check {
                family: family.to_string(),
                style: style.to_string(),
                path,
                origin,
            });
        }
    }
    rows
}

// "ok" when every checked face is bundled, otherwise the faces that are not.
pub fn check_summary(rows: &[Check]) -> String {
    let wrong: Vec<String> = rows
        .iter()
        .filter(|row| row.origin != "bundled")
        .map(|row| format!("{} {} {}", row.family, row.style, row.origin))
        .collect();
    if wrong.is_empty() {
        "ok".to_string()
    } else {
        format!("spoofed: {}", wrong.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIN: &str = "/usr/lib/Euro-Office-Lite/binaries";

    fn sample() -> String {
        let mut files = Vec::new();
        let mut infos = Vec::new();
        for family in CHECKED_FAMILIES {
            let stem = family.replace(' ', "");
            let base = files.len() as i64;
            for style in ["Regular", "Italic", "Bold", "BoldItalic"] {
                files.push(format!("\"{}/fonts/{}-{}.ttf\"", BIN, stem, style));
            }
            infos.push(format!("[\"{}\",{},0,{},0,{},0,{},0]", family, base, base + 1, base + 2, base + 3));
        }
        files.push("\"/usr/share/fonts/truetype/a \\\"quoted\\\" \\\\dir/Odd.ttf\"".to_string());
        infos.push(format!("[\"Odd \\\"Name\\\"\",{},0,-1,-1,-1,-1,-1,-1]", files.len() - 1));
        format!(
            "\u{feff}window[\"__all_fonts_js_version__\"] = 2;\n\nwindow[\"__fonts_files\"] = [\n{}\n];\nwindow[\"__fonts_infos\"] = [\n{}\n];\nwindow[\"__fonts_ranges\"] = [1,2];\n",
            files.join(",\n"),
            infos.join(",\n")
        )
    }

    #[test]
    fn the_real_format_parses_with_escapes_and_missing_faces() {
        let list = parse(&sample()).unwrap();
        assert_eq!(list.files.len(), 17);
        assert_eq!(list.files[16], "/usr/share/fonts/truetype/a \"quoted\" \\dir/Odd.ttf");
        assert_eq!(list.infos[0], FontInfo { name: "Liberation Sans".into(), faces: [0, 1, 2, 3] });
        assert_eq!(list.infos[4], FontInfo { name: "Odd \"Name\"".into(), faces: [16, -1, -1, -1] });
    }

    #[test]
    fn origin_follows_the_bundled_fonts_folder_in_any_spelling() {
        assert_eq!(origin(&format!("{}/fonts/Carlito-Bold.ttf", BIN), BIN), "bundled");
        assert_eq!(origin("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf", BIN), "system");
        assert_eq!(origin("/?/C:/App/binaries/fonts/LiberationSans-Regular.ttf", r"C:\App\binaries"), "bundled");
        assert_eq!(origin(r"\\?\C:\App\binaries\fonts\Carlito-Regular.ttf", r"\\?\C:\App\binaries"), "bundled");
        assert_eq!(origin("C:/Windows/Fonts/arial.ttf", r"C:\App\binaries"), "system");
        // A sibling folder that only shares the prefix is not the fonts folder.
        assert_eq!(origin(&format!("{}/fonts-extra/X.ttf", BIN), BIN), "system");
    }

    #[test]
    fn all_sixteen_faces_bundled_is_ok() {
        let list = parse(&sample()).unwrap();
        let rows = check(&list, BIN);
        assert_eq!(rows.len(), 16);
        assert!(rows.iter().all(|row| row.origin == "bundled"), "{:?}", rows);
        assert_eq!(check_summary(&rows), "ok");
    }

    #[test]
    fn a_system_face_or_a_missing_family_is_reported() {
        let text = sample().replace(
            &format!("\"{}/fonts/LiberationSerif-Bold.ttf\"", BIN),
            "\"/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf\"",
        );
        let text = text.replace("[\"Carlito\"", "[\"Carlito Other\"");
        let rows = check(&parse(&text).unwrap(), BIN);
        let serif_bold = rows.iter().find(|r| r.family == "Liberation Serif" && r.style == "bold").unwrap();
        assert_eq!(serif_bold.origin, "system");
        assert!(rows.iter().filter(|r| r.family == "Carlito").all(|r| r.origin == "missing" && r.path.is_empty()));
        let summary = check_summary(&rows);
        assert!(summary.starts_with("spoofed: Liberation Serif bold system, Carlito regular missing"), "{}", summary);
    }

    #[test]
    fn a_file_without_the_arrays_is_an_error() {
        assert!(parse("window[\"__fonts_infos\"] = [];").is_err());
        assert!(parse("window[\"__fonts_files\"] = [\"a\"").is_err());
    }
}
