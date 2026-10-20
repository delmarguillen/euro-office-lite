// tests/corpus/known.tsv: the differences the corpus accepts, each with its
// reason. An entry names where a value may differ (document, variant, step
// and check, exact or with a trailing *) and on which platforms (all, an x2t
// family such as 8.2, or platform ids). It never says what the value is: the
// committed baseline of each platform does that.

use std::path::Path;

pub const COLUMNS: [&str; 8] = ["doc", "variant", "step", "check", "platforms", "kind", "reason", "issue"];

// The ids of the platform table in .github/workflows/corpus.yml.
pub const PLATFORM_IDS: [&str; 6] = ["win-x64", "win-arm64", "mac-x64", "mac-arm64", "linux-x64", "linux-arm64"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    // Depends on the runner: its system fonts, its OS.
    Env,
    // Comes from the x2t version.
    X2t,
    // A real fault, tracked by an issue.
    Bug,
    // A crash that comes and goes; see report::flaky.
    Flaky,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Platforms {
    All,
    Family(String),
    Ids(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownEntry {
    pub doc: String,
    pub variant: String,
    pub step: String,
    pub check: String,
    pub platforms: Platforms,
    pub kind: Kind,
    pub reason: String,
    pub issue: String,
    // Its line in the file, to name it in the reports.
    pub line: usize,
}

fn field_matches(pattern: &str, value: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

fn is_pattern(field: &str) -> bool {
    field.ends_with('*')
}

impl KnownEntry {
    pub fn matches(&self, doc: &str, variant: &str, step: &str, check: &str) -> bool {
        field_matches(&self.doc, doc)
            && field_matches(&self.variant, variant)
            && field_matches(&self.step, step)
            && field_matches(&self.check, check)
    }

    // The family is the one the platform shows in this run, so an entry for
    // 8.2 stops applying to a platform that moves to another x2t.
    pub fn covers(&self, id: &str, family: &str) -> bool {
        match &self.platforms {
            Platforms::All => true,
            Platforms::Family(f) => f == family,
            Platforms::Ids(ids) => ids.iter().any(|i| i == id),
        }
    }

    // A hard failure is only ever covered on purpose: by a bug entry that
    // names its document and check without a wildcard.
    pub fn covers_failure(&self, doc: &str, check: &str) -> bool {
        self.kind == Kind::Bug && self.doc == doc && self.check == check
    }

    pub fn describe(&self) -> String {
        let kind = match self.kind {
            Kind::Env => "env",
            Kind::X2t => "x2t",
            Kind::Bug => "bug",
            Kind::Flaky => "flaky",
        };
        let issue = if self.issue.is_empty() { String::new() } else { format!(", {}", self.issue) };
        format!("known.tsv line {} ({}{}): {}", self.line, kind, issue, self.reason)
    }
}

fn platforms(value: &str) -> Result<Platforms, String> {
    if value == "all" {
        return Ok(Platforms::All);
    }
    let family = value.split_once('.').is_some_and(|(a, b)| {
        !a.is_empty() && !b.is_empty() && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit())
    });
    if family {
        return Ok(Platforms::Family(value.to_string()));
    }
    let ids: Vec<String> = value.split(',').map(str::to_string).collect();
    match ids.iter().find(|id| !PLATFORM_IDS.contains(&id.as_str())) {
        Some(bad) => Err(format!("unknown platform {:?}", bad)),
        None => Ok(Platforms::Ids(ids)),
    }
}

// #12 or owner/repo#12.
fn valid_issue(issue: &str) -> bool {
    let Some((repo, number)) = issue.rsplit_once('#') else { return false };
    let number_ok = !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit());
    let name_ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    number_ok && (repo.is_empty() || repo.split_once('/').is_some_and(|(o, r)| name_ok(o) && name_ok(r)))
}

fn entry(cells: &[&str], line: usize) -> Result<KnownEntry, String> {
    // The issue is the last column and may be left out, with its tab.
    let (doc, variant, step, check, platforms_field, kind, reason, issue) = match cells {
        [d, v, s, c, p, k, r, i] => (d, v, s, c, p, k, r, i),
        [d, v, s, c, p, k, r] => (d, v, s, c, p, k, r, &""),
        _ => return Err(format!("{} columns instead of {}", cells.len(), COLUMNS.len())),
    };
    for (name, value) in [("doc", doc), ("variant", variant), ("step", step), ("check", check)] {
        if value.is_empty() || value[..value.len().saturating_sub(1)].contains('*') {
            return Err(format!("{} {:?}: give a value, *, or a prefix ending in *", name, value));
        }
    }
    let kind = match *kind {
        "env" => Kind::Env,
        "x2t" => Kind::X2t,
        "bug" => Kind::Bug,
        "flaky" => Kind::Flaky,
        other => return Err(format!("kind {:?} is not env, x2t, bug or flaky", other)),
    };
    if reason.trim().is_empty() {
        return Err("every entry needs its reason".to_string());
    }
    if !issue.is_empty() && !valid_issue(issue) {
        return Err(format!("issue {:?} is not #N or owner/repo#N", issue));
    }
    if kind == Kind::Bug && issue.is_empty() {
        return Err("a bug entry needs its issue".to_string());
    }
    if kind == Kind::Flaky && (is_pattern(doc) || is_pattern(step) || *check != "*") {
        return Err("a flaky entry names its document and step exactly, with check *".to_string());
    }
    Ok(KnownEntry {
        doc: doc.to_string(),
        variant: variant.to_string(),
        step: step.to_string(),
        check: check.to_string(),
        platforms: platforms(platforms_field)?,
        kind,
        reason: reason.trim().to_string(),
        issue: issue.to_string(),
        line,
    })
}

pub fn parse(text: &str) -> Result<Vec<KnownEntry>, String> {
    let mut lines = text.lines().enumerate().filter(|(_, l)| !l.is_empty());
    match lines.next() {
        Some((_, header)) if header.split('\t').eq(COLUMNS) => {}
        _ => return Err(format!("known.tsv must start with the header {}", COLUMNS.join(" "))),
    }
    lines
        .map(|(n, line)| {
            let cells: Vec<&str> = line.split('\t').collect();
            entry(&cells, n + 1).map_err(|e| format!("known.tsv line {}: {}", n + 1, e))
        })
        .collect()
}

pub fn read(path: &Path) -> Result<Vec<KnownEntry>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "doc\tvariant\tstep\tcheck\tplatforms\tkind\treason\tissue\n";

    fn one(row: &str) -> Result<KnownEntry, String> {
        parse(&format!("{}{}\n", HEADER, row)).map(|mut e| e.remove(0))
    }

    #[test]
    fn entries_are_read_with_their_patterns_and_platforms() {
        let e = one("*\t*\tsave*\tpart:word/fontTable.xml\tall\tenv\tpanose from the system\t").unwrap();
        assert!(e.matches("rich.docx", "plain", "save", "part:word/fontTable.xml"));
        assert!(e.matches("a.odt", "special-name", "save_as.docx", "part:word/fontTable.xml"));
        assert!(!e.matches("a.docx", "plain", "open", "part:word/fontTable.xml"));
        assert!(!e.matches("a.docx", "plain", "save", "part:word/fontTable.xml.rels"));
        assert!(e.covers("mac-x64", "8.2") && e.covers("win-x64", "9.4"));
        assert_eq!(e.line, 2);

        let e = one("b.csv\tplain\tsave\tpart:ppt/*\t8.2\tx2t\tonly 9.4 writes it\t").unwrap();
        assert!(e.matches("b.csv", "plain", "save", "part:ppt/theme/theme0.xml"));
        assert!(e.covers("linux-x64", "8.2") && !e.covers("linux-x64", "9.4"));

        let e = one("b.pptx\tplain\tpdf\tfonts\twin-x64,win-arm64\tenv\tCalibri on Windows\tdelmarguillen/euro-office-lite#12").unwrap();
        assert!(e.covers("win-arm64", "9.4") && !e.covers("linux-arm64", "9.4"));
        assert_eq!(e.platforms, Platforms::Ids(vec!["win-x64".into(), "win-arm64".into()]));
        assert!(e.describe().contains("delmarguillen/euro-office-lite#12"));
    }

    #[test]
    fn bad_entries_are_refused() {
        for row in [
            "a.docx\tplain\tsave\tfonts\tall\tenv\t\t",
            "a.docx\tplain\tsave\tfonts\tall\tenv\t  \t",
            "a.docx\tplain\tsave\tfonts\tall\tother\tr\t",
            "a.docx\tplain\tsave\tfonts\twin-x86\tenv\tr\t",
            "a.docx\tplain\tsave\tfonts\twin-x64,\tenv\tr\t",
            "a.docx\tplain\tsave\tfonts\t9.\tenv\tr\t",
            "a*.docx\tplain\tsave\tfonts\tall\tenv\tr\t",
            "\tplain\tsave\tfonts\tall\tenv\tr\t",
            "a.docx\tplain\tsave\tfonts\tall\tbug\tr\t",
            "a.docx\tplain\tsave\tfonts\tall\tbug\tr\tsee #12",
            "a.docx\tplain\tsave\tfonts\tall\tenv\tr\towner#12",
            "*\tplain\tpdf\t*\tall\tflaky\tr\t",
            "a.docx\tplain\tpdf*\t*\tall\tflaky\tr\t",
            "a.docx\tplain\tpdf\tstatus\tall\tflaky\tr\t",
            "a.docx\tplain\tpdf\tfonts\tall\tenv",
            "a.docx\tplain\tpdf\tfonts\tall\tenv\tr\t#1\textra",
        ] {
            assert!(one(row).is_err(), "{}", row);
        }
        // Without an issue the last column may go, with its tab.
        assert_eq!(one("a.docx\tplain\tpdf\tfonts\tall\tenv\tr").unwrap().issue, "");
        assert!(parse("doc\tvariant\n").is_err());
        assert!(one("a.docx\tplain\tpdf\t*\tlinux-arm64\tflaky\tcomes and goes\t").is_ok());
        assert!(one("a.docx\tplain\tsave\tfonts.script\tall\tbug\tr\t#7").is_ok());
        assert_eq!(parse(HEADER).unwrap(), Vec::new());
    }

    // Only a bug entry that names the document and the check covers a failure.
    #[test]
    fn a_failure_is_covered_only_by_a_bug_named_exactly() {
        let bug = one("a.docx\tplain\tpdf\tfonts.script\tall\tbug\tr\t#7").unwrap();
        assert!(bug.covers_failure("a.docx", "fonts.script"));
        assert!(!bug.covers_failure("b.docx", "fonts.script"));
        let wide = one("a.docx\t*\t*\tfonts*\tall\tbug\tr\t#7").unwrap();
        assert!(!wide.covers_failure("a.docx", "fonts.script"));
        let env = one("a.docx\tplain\tpdf\tfonts.script\tall\tenv\tr\t").unwrap();
        assert!(!env.covers_failure("a.docx", "fonts.script"));
    }

    #[test]
    fn the_platform_ids_are_the_workflows() {
        let workflow = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/corpus.yml"),
        )
        .unwrap();
        for id in PLATFORM_IDS {
            assert!(workflow.contains(&format!("\n          {} ", id)) || workflow.contains(&format!("'{} ", id)), "{}", id);
        }
    }
}
