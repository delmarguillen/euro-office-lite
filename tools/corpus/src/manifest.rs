// tests/corpus/manifest.tsv: one row per document, with what to run on it and
// the text its PDF must contain.

use crate::tsv::Table;
use std::path::Path;

pub const STEPS: [&str; 3] = ["open", "save", "pdf"];
pub const VARIANTS: [&str; 3] = ["plain", "special-name", "special-temp"];
pub const CLASSES: [&str; 2] = ["comparable", "system-fonts"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    // A fixture in tests/corpus.
    Fixture(String),
    // A file of the installer's resource folder, such as templates/blank.docx.
    Installed(String),
    // A 0-byte file created at run time under this name.
    Empty(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedFailure {
    pub step: String,
    pub exit: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    // The manifest's own spelling, used as the document id in every table.
    pub doc: String,
    pub source: Source,
    pub class: String,
    pub steps: Vec<String>,
    pub expect: Vec<String>,
    // A step that must fail with one exit code, as x2t does with an empty
    // CSV (expect "fail:open:88"). Nothing else counts as the expected failure.
    pub expect_fail: Option<ExpectedFailure>,
    pub variants: Vec<String>,
    pub save_as: Vec<String>,
}

impl Entry {
    pub fn has_step(&self, step: &str) -> bool {
        self.steps.iter().any(|s| s == step)
    }

    // The name the document has on disk, which decides its format.
    pub fn file_name(&self) -> &str {
        match &self.source {
            Source::Fixture(name) | Source::Empty(name) => name,
            Source::Installed(path) => path.rsplit('/').next().unwrap_or(path),
        }
    }

    pub fn extension(&self) -> String {
        Path::new(self.file_name())
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }

    // A folder name for this document's results: no @, / or :, which the
    // artifact upload refuses or would nest.
    pub fn id(&self) -> String {
        self.doc
            .trim_start_matches('@')
            .chars()
            .map(|c| if c == '/' || c == ':' { '_' } else { c })
            .collect()
    }
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let table = Table::parse(text);
    for column in ["file", "class", "steps", "expect", "variants", "save_as"] {
        if table.column(column).is_none() {
            return Err(format!("manifest has no {} column", column));
        }
    }
    let mut entries = Vec::new();
    for row in &table.rows {
        let doc = table.get(row, "file").to_string();
        let source = if let Some(path) = doc.strip_prefix("@templates/") {
            Source::Installed(format!("templates/{}", path))
        } else if let Some(name) = doc.strip_prefix("@empty:") {
            Source::Empty(name.to_string())
        } else if doc.starts_with('@') {
            return Err(format!("unknown source {}", doc));
        } else {
            Source::Fixture(doc.clone())
        };
        let expect_field = table.get(row, "expect");
        let expect_fail = match expect_field.strip_prefix("fail") {
            Some(spec) => {
                let parsed = spec.strip_prefix(':').and_then(|s| s.split_once(':')).and_then(|(step, exit)| {
                    Some(ExpectedFailure { step: step.to_string(), exit: exit.parse().ok()? })
                });
                Some(parsed.ok_or_else(|| format!("{}: expect {} is not fail:<step>:<exit code>", doc, expect_field))?)
            }
            None => None,
        };
        let expect = if expect_fail.is_some() {
            Vec::new()
        } else {
            expect_field
                .split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };
        let mut variants = list(table.get(row, "variants"));
        if variants.is_empty() {
            variants.push("plain".to_string());
        }
        let entry = Entry {
            source,
            class: table.get(row, "class").to_string(),
            steps: list(table.get(row, "steps")),
            expect,
            expect_fail,
            variants,
            save_as: list(table.get(row, "save_as")),
            doc,
        };
        validate(&entry)?;
        entries.push(entry);
    }
    Ok(entries)
}

pub fn read(corpus: &Path) -> Result<Vec<Entry>, String> {
    let path = corpus.join("manifest.tsv");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    parse(&text)
}

fn validate(entry: &Entry) -> Result<(), String> {
    let doc = &entry.doc;
    if entry.file_name().contains(['&', '<', '>']) {
        return Err(format!("{}: special names are made at run time, not stored", doc));
    }
    if !CLASSES.contains(&entry.class.as_str()) {
        return Err(format!("{}: unknown class {}", doc, entry.class));
    }
    if !entry.has_step("open") {
        return Err(format!("{}: every document is opened first", doc));
    }
    if let Some(step) = entry.steps.iter().find(|s| !STEPS.contains(&s.as_str())) {
        return Err(format!("{}: unknown step {}", doc, step));
    }
    if let Some(variant) = entry.variants.iter().find(|v| !VARIANTS.contains(&v.as_str())) {
        return Err(format!("{}: unknown variant {}", doc, variant));
    }
    if let Some(ext) = entry
        .save_as
        .iter()
        .find(|ext| crate::x2t_task::detect_format(Path::new(&format!("x.{}", ext))) == 0)
    {
        return Err(format!("{}: x2t has no format for save_as {}", doc, ext));
    }
    if let Some(failure) = &entry.expect_fail {
        if !entry.has_step(&failure.step) {
            return Err(format!("{}: the expected failure names step {}, which does not run", doc, failure.step));
        }
    }
    // A PDF that comes out without its text is a failure only if the manifest
    // says which text; the empty installer templates have none.
    let installed = matches!(entry.source, Source::Installed(_));
    if entry.class == "comparable" && entry.has_step("pdf") && entry.expect.is_empty() && !installed {
        return Err(format!("{}: a comparable PDF needs expected text", doc));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn the_real_manifest_is_valid_and_its_files_exist() {
        let corpus = repo().join("tests/corpus");
        let entries = read(&corpus).unwrap();
        assert!(entries.len() >= 30, "{}", entries.len());
        for entry in &entries {
            match &entry.source {
                Source::Fixture(name) => {
                    let path = corpus.join(name);
                    assert!(path.is_file() && std::fs::metadata(&path).unwrap().len() > 0, "{}", name);
                }
                Source::Installed(path) => {
                    assert!(repo().join("src-tauri").join(path).is_file(), "{}", path)
                }
                Source::Empty(_) => {}
            }
        }
        let mut ids: Vec<String> = entries.iter().map(Entry::id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), entries.len(), "document ids must be unique");
    }

    #[test]
    fn every_stored_fixture_is_in_the_manifest() {
        let corpus = repo().join("tests/corpus");
        let entries = read(&corpus).unwrap();
        for file in std::fs::read_dir(&corpus).unwrap().flatten() {
            let name = file.file_name().to_string_lossy().to_string();
            if file.path().is_dir() || name == "manifest.tsv" || name == "README.md" {
                continue;
            }
            assert!(
                entries.iter().any(|e| e.source == Source::Fixture(name.clone())),
                "{} is not in the manifest",
                name
            );
        }
    }

    // A checkout or a copy that rewrote line endings or text would change what
    // the corpus tests without failing anything else.
    #[test]
    fn packages_are_zips_and_text_fixtures_have_their_exact_bytes() {
        let corpus = repo().join("tests/corpus");
        for entry in read(&corpus).unwrap() {
            let Source::Fixture(name) = &entry.source else { continue };
            let bytes = std::fs::read(corpus.join(name)).unwrap();
            if ["docx", "xlsx", "pptx", "odt", "ods", "odp"].contains(&entry.extension().as_str()) {
                assert!(bytes.starts_with(b"PK\x03\x04"), "{} is not a zip", name);
            }
        }

        // The bytes src/text_fixtures.py writes.
        fn utf16(text: &str, little: bool) -> Vec<u8> {
            let mut out = if little { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
            for unit in text.encode_utf16() {
                out.extend(if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            out
        }
        let bom = |text: &str| [&[0xEFu8, 0xBB, 0xBF][..], text.as_bytes()].concat();
        let expected: [(&str, Vec<u8>); 10] = [
            ("utf8-bom-semicolon-crlf.csv", bom("Name;Price;Quantity\r\nCafé;1,50;3\r\nTé verde;2,25;10\r\n")),
            ("utf8-comma-lf-final.csv", "name,city,amount\nAna,Málaga,10\nBjörn,Göteborg,20\n".as_bytes().to_vec()),
            ("utf8-comma-nofinal.csv", b"name,amount\nfirst,1\nsecond,2".to_vec()),
            (
                "win1252-semicolon.csv",
                b"A\xF1o;Ciudad;Importe\r\n2024;C\xF3rdoba;1.234,50\r\nNi\xF1o;Pe\xF1\xEDscola;7\r\n".to_vec(),
            ),
            ("utf16le-tab.csv", utf16("col1\tcol2\tcol3\r\nalpha α\tbeta β\tgamma γ\r\n1\t2\t3\r\n", true)),
            ("quoted-delimiter.csv", b"id;text;note\n1;\"a;b\";\"say \"\"hi\"\"\"\n2;\"semi;colon\";plain\n".to_vec()),
            ("empty-lines.csv", b"a,b\n1,2\n\n3,4\n".to_vec()),
            ("utf8.txt", "First line\nSegunda línea con ñ\n".as_bytes().to_vec()),
            ("win1252-crlf.txt", b"L\xEDnea uno\r\nCaf\xE9 \x80 \x89\r\n".to_vec()),
            ("utf16be.txt", utf16("Hello\nÜber alles\n", false)),
        ];
        for (name, bytes) in expected {
            assert_eq!(std::fs::read(corpus.join(name)).unwrap(), bytes, "{} differs from text_fixtures.py", name);
        }
    }

    const HEADER: &str = "file\tclass\tsteps\texpect\tvariants\tsave_as\tnote\n";

    #[test]
    fn sources_variants_and_expectations_are_read() {
        let text = format!(
            "{}@templates/blank.docx\tcomparable\topen,save,pdf\t\t\t\tt\n@empty:empty.csv\tcomparable\topen\tfail:open:88\tplain\t\t\na.docx\tcomparable\topen,pdf\tOne|Two\tspecial-name,special-temp\todt,csv\t\n",
            HEADER
        );
        let entries = parse(&text).unwrap();
        assert_eq!(entries[0].source, Source::Installed("templates/blank.docx".into()));
        assert_eq!(entries[0].variants, vec!["plain"]);
        assert_eq!(entries[0].id(), "templates_blank.docx");
        assert_eq!(entries[1].source, Source::Empty("empty.csv".into()));
        assert_eq!(entries[1].expect_fail, Some(ExpectedFailure { step: "open".into(), exit: 88 }));
        assert!(entries[1].expect.is_empty());
        assert_eq!(entries[1].id(), "empty_empty.csv");
        assert_eq!(entries[2].expect, vec!["One", "Two"]);
        assert_eq!(entries[2].variants, vec!["special-name", "special-temp"]);
        assert_eq!(entries[2].save_as, vec!["odt", "csv"]);
    }

    #[test]
    fn bad_rows_are_refused() {
        for row in [
            "R&D.docx\tcomparable\topen\t\t\t\t",
            "a.docx\tother\topen\t\t\t\t",
            "a.docx\tcomparable\tsave\t\t\t\t",
            "a.docx\tcomparable\topen,print\t\t\t\t",
            "a.docx\tcomparable\topen\t\tweird\t\t",
            "a.docx\tcomparable\topen\t\t\txyz\t",
            "a.docx\tcomparable\topen,pdf\t\t\t\t",
            "@nope:a.docx\tcomparable\topen\t\t\t\t",
            "a.csv\tcomparable\topen\tfail\t\t\t",
            "a.csv\tcomparable\topen\tfail:open\t\t\t",
            "a.csv\tcomparable\topen\tfail:open:x\t\t\t",
            "a.csv\tcomparable\topen\tfail:save:88\t\t\t",
        ] {
            assert!(parse(&format!("{}{}\n", HEADER, row)).is_err(), "{}", row);
        }
        assert!(parse("a.docx\tsystem-fonts\topen,pdf\t\t\t\t\n").is_err(), "needs a header");
        assert!(parse(&format!("{}a.docx\tsystem-fonts\topen,pdf\t\t\t\t\n", HEADER)).is_ok());
    }
}
