// `corpus matrix`: lines up the results of several platforms, one row per
// value and one column per platform, grouped by the x2t family each platform
// ships. A platform that was expected and left no results fails it; with
// known.tsv, so does a difference inside one family, a value missing on one
// platform or a failure, that no entry explains. 8.2 against 9.4 never does.

use crate::compare::{self, Key};
use crate::known::{Kind, KnownEntry};
use crate::manifest::{self, Entry};
use crate::markdown::clean;
use crate::pdf;
use crate::run::Infra;
use crate::tsv::Table;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct MatrixArgs {
    // id and the folder of one run (run1) of that platform.
    pub platforms: Vec<(String, PathBuf)>,
    pub expect: Vec<String>,
    pub out: PathBuf,
    // known.tsv when given: then an unexplained difference fails too.
    pub known: Option<Vec<KnownEntry>>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    // Expected platforms that left no usable results.
    pub infra: usize,
    // diff-in-family, missing and FAIL rows that no known entry explains.
    pub unexplained: usize,
}

// Platform ids go into file names and table headers.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

// "OOX/binary file converter. Version: 8.2.0.147" -> "8.2".
pub fn family(version: &str) -> String {
    let token = version.rsplit("Version:").next().unwrap_or("").split_whitespace().next().unwrap_or("");
    let parts: Vec<&str> = token.split('.').take(2).collect();
    if parts.len() == 2 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        parts.join(".")
    } else {
        "unknown".to_string()
    }
}

fn full_version(version: &str) -> String {
    version.rsplit("Version:").next().unwrap_or("").trim().to_string()
}

// "Key: value" lines, as the workflow writes package.txt and platform.txt.
fn key_values(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

pub struct Loaded {
    pub id: String,
    pub dir: PathBuf,
    pub family: String,
    pub env: BTreeMap<String, String>,
    pub values: BTreeMap<Key, String>,
    pub report: Table,
    // The class report gave each step's output, by (doc, variant, step).
    pub classes: BTreeMap<(String, String, String), String>,
    pub ran: BTreeSet<(String, String)>,
    pub entries: Vec<Entry>,
    pub package: BTreeMap<String, String>,
    pub platform_text: String,
    pub font_names: BTreeSet<String>,
}

pub enum Column {
    Loaded(Box<Loaded>),
    Infra { id: String, reason: String },
}

impl Column {
    pub fn id(&self) -> &str {
        match self {
            Column::Loaded(l) => &l.id,
            Column::Infra { id, .. } => id,
        }
    }

    fn family(&self) -> &str {
        match self {
            Column::Loaded(l) => &l.family,
            Column::Infra { .. } => "",
        }
    }
}

pub fn load(id: &str, dir: &Path) -> Result<Loaded, String> {
    let read = |name: &str| Table::read(&dir.join(name));
    let env_table = read("env.tsv")?;
    let run = read("run.tsv")?;
    let report = read("report.tsv")?;
    let manifest_text = std::fs::read_to_string(dir.join("manifest.tsv"))
        .map_err(|e| format!("cannot read {}: {}", dir.join("manifest.tsv").display(), e))?;
    let entries = manifest::parse(&manifest_text)?;
    let env: BTreeMap<String, String> = env_table
        .rows
        .iter()
        .map(|row| (env_table.get(row, "key").to_string(), env_table.get(row, "value").to_string()))
        .collect();
    let mut classes = BTreeMap::new();
    for row in report.rows.iter().filter(|row| report.get(row, "check") == "class") {
        let at = |c: &str| report.get(row, c).to_string();
        classes.insert((at("doc"), at("variant"), at("step")), at("value"));
    }
    let ran = run
        .rows
        .iter()
        .map(|row| (run.get(row, "doc").to_string(), run.get(row, "variant").to_string()))
        .collect();
    let parent = dir.parent().unwrap_or(dir);
    let package = key_values(&std::fs::read_to_string(parent.join("package.txt")).unwrap_or_default());
    let platform_text = std::fs::read_to_string(parent.join("platform.txt")).unwrap_or_default();
    let font_names = std::fs::read_to_string(dir.join("fonts").join("AllFonts.js"))
        .ok()
        .and_then(|text| crate::allfonts::parse(&text).ok())
        .map(|list| list.infos.into_iter().map(|info| info.name).collect())
        .unwrap_or_default();
    Ok(Loaded {
        id: id.to_string(),
        dir: dir.to_path_buf(),
        family: family(env.get("pdf_x2t.version").map(String::as_str).unwrap_or("")),
        values: compare::values(&env_table, &run, &report),
        env,
        report,
        classes,
        ran,
        entries,
        package,
        platform_text,
        font_names,
    })
}

// What one platform shows for one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Value(String),
    // Shown, but left out of the comparison, with the reason.
    Excluded(&'static str, String),
    // The platform ran the document and step but has no such value.
    Absent,
    NotRun,
    Infra,
}

impl Cell {
    pub fn text(&self) -> String {
        match self {
            Cell::Value(v) => v.clone(),
            Cell::Excluded(reason, v) => format!("n/c:{} {}", reason, v),
            Cell::Absent => "<missing>".to_string(),
            Cell::NotRun => "<not run>".to_string(),
            Cell::Infra => "<infra>".to_string(),
        }
    }

    // The cell as matrix.md shows it, with the value cut and cleaned.
    fn markdown(&self) -> String {
        match self {
            Cell::Value(v) => short(v),
            Cell::Excluded(reason, v) => format!("n/c:{} {}", reason, short(v)),
            Cell::Absent => "(missing)".to_string(),
            Cell::NotRun => "(not run)".to_string(),
            Cell::Infra => "(infra)".to_string(),
        }
    }

    fn value(&self) -> Option<&str> {
        match self {
            Cell::Value(v) | Cell::Excluded(_, v) => Some(v),
            _ => None,
        }
    }
}

// The hard failures of `corpus report`, seen row by row.
pub fn is_hard_failure(key: &Key, value: &str) -> bool {
    let (_, doc, _, step, check) = key;
    let txt = doc.to_lowercase().ends_with(".txt");
    (check == "status" && value != "ok" && value != "expected-fail")
        || (step == "pdf" && check == "expected" && value.starts_with("missing:"))
        || (check == "fonts.script" && value != "ok")
        || check == "report.error"
        || (txt && step == "save" && check == "roundtrip.bytes" && value != "equal")
}

// How x2t ended is compared everywhere; everything else leaves the comparison
// when it depends on the fonts of the machine.
fn exclusion(platform: &Loaded, key: &Key) -> Option<&'static str> {
    let (_, doc, variant, step, check) = key;
    if ["status", "exit", "signal"].contains(&check.as_str()) {
        return None;
    }
    let class = platform
        .classes
        .get(&(doc.clone(), variant.clone(), step.clone()))
        .cloned()
        .or_else(|| platform.entries.iter().find(|e| &e.doc == doc).map(|e| e.class.clone()))
        .unwrap_or_default();
    let spoofed = platform.env.get("fonts.check").map(String::as_str) != Some("ok");
    match class.as_str() {
        "system-fonts" => Some("system-fonts"),
        "auto-system-fonts" => Some("auto-system-fonts"),
        "spoofed" => Some("spoofed"),
        "comparable" if spoofed => Some("spoofed"),
        _ => None,
    }
}

pub fn cell(column: &Column, key: &Key) -> Cell {
    let Column::Loaded(platform) = column else { return Cell::Infra };
    let (_, doc, variant, _, _) = key;
    match platform.values.get(key) {
        Some(value) => match exclusion(platform, key) {
            Some(reason) => Cell::Excluded(reason, value.clone()),
            None => Cell::Value(value.clone()),
        },
        None if platform.ran.contains(&(doc.clone(), variant.clone())) => Cell::Absent,
        None => Cell::NotRun,
    }
}

fn equal(key: &Key, a: &str, b: &str) -> bool {
    a == b || (compare::is_text_open_size(key) && compare::within_one_percent(a, b))
}

// Every pair, since "within 1%" does not chain.
fn all_equal(key: &Key, values: &[&str]) -> bool {
    values.iter().enumerate().all(|(i, a)| values[i + 1..].iter().all(|b| equal(key, a, b)))
}

// A failure counts in every cell except one a bug entry covers on purpose.
// A cell a known entry took out was not failing (an absent value included).
fn fails(key: &Key, cell: &Cell) -> bool {
    !matches!(cell, Cell::Excluded(KNOWN_BUG | KNOWN, _)) && cell.value().is_some_and(|v| is_hard_failure(key, v))
}

// One verdict per key, from the cells of every platform and their families.
pub fn verdict(key: &Key, cells: &[(&str, Cell)]) -> &'static str {
    if cells.iter().any(|(_, c)| fails(key, c)) {
        return "FAIL";
    }
    if cells.iter().any(|(_, c)| *c == Cell::Absent) {
        return "missing";
    }
    let compared: Vec<(&str, &str)> = cells
        .iter()
        .filter_map(|(family, c)| match c {
            Cell::Value(v) => Some((*family, v.as_str())),
            _ => None,
        })
        .collect();
    if compared.len() < 2 {
        return "n/c";
    }
    let values: Vec<&str> = compared.iter().map(|(_, v)| *v).collect();
    if all_equal(key, &values) {
        return "same";
    }
    let families: BTreeSet<&str> = compared.iter().map(|(f, _)| *f).collect();
    let each_family_agrees = families.iter().all(|family| {
        let inside: Vec<&str> = compared.iter().filter(|(f, _)| f == family).map(|(_, v)| *v).collect();
        all_equal(key, &inside)
    });
    if each_family_agrees {
        "same-in-family"
    } else {
        "diff-in-family"
    }
}

fn note(key: &Key, columns: &[&Column], cells: &[(&str, Cell)], verdict: &str) -> String {
    let ids = |pick: &dyn Fn(&Cell) -> bool| -> Vec<&str> {
        columns.iter().zip(cells).filter(|(_, (_, c))| pick(c)).map(|(col, _)| col.id()).collect()
    };
    let mut parts = Vec::new();
    match verdict {
        "FAIL" => parts.push(format!("fails on {}", ids(&|c| fails(key, c)).join(","))),
        "missing" => parts.push(format!("missing on {}", ids(&|c| *c == Cell::Absent).join(","))),
        "diff-in-family" => {
            let mut families: BTreeSet<&str> = BTreeSet::new();
            for (family, _) in cells.iter().filter(|(_, c)| matches!(c, Cell::Value(_))) {
                let inside: Vec<&str> = cells
                    .iter()
                    .filter(|(f, c)| f == family && matches!(c, Cell::Value(_)))
                    .filter_map(|(_, c)| c.value())
                    .collect();
                if !all_equal(key, &inside) {
                    families.insert(*family);
                }
            }
            let list: Vec<&str> = families.into_iter().collect();
            parts.push(format!("differs within {}", list.join(",")));
        }
        _ => {}
    }
    let mut reasons: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (column, (_, c)) in columns.iter().zip(cells) {
        if let Cell::Excluded(reason, _) = c {
            reasons.entry(reason).or_default().push(column.id());
        }
    }
    for (reason, list) in reasons {
        parts.push(format!("n/c:{} on {}", reason, list.join(",")));
    }
    parts.join("; ")
}

// Columns go by family, in the order each family first appears.
pub fn order(columns: Vec<Column>) -> Vec<Column> {
    let mut families: Vec<String> = Vec::new();
    for column in &columns {
        if let Column::Loaded(l) = column {
            if !families.contains(&l.family) {
                families.push(l.family.clone());
            }
        }
    }
    let rank = |c: &Column| match c {
        Column::Loaded(l) => families.iter().position(|f| *f == l.family).unwrap_or(usize::MAX - 1),
        Column::Infra { .. } => usize::MAX,
    };
    let mut indexed: Vec<(usize, usize, Column)> = columns.into_iter().enumerate().map(|(i, c)| (rank(&c), i, c)).collect();
    indexed.sort_by_key(|(r, i, _)| (*r, *i));
    indexed.into_iter().map(|(_, _, c)| c).collect()
}

pub struct Row {
    pub key: Key,
    pub cells: Vec<Cell>,
    pub verdict: &'static str,
    pub note: String,
}

const KNOWN: &str = "known";
const KNOWN_BUG: &str = "known-bug";

// Takes the cells of the platforms a known entry covers out of the
// comparison. A failing cell leaves only for a bug entry that names its
// document and check. Returns the entries that took out a cell.
fn apply_known<'k>(key: &Key, columns: &[Column], cells: &mut [(&str, Cell)], known: &'k [KnownEntry]) -> Vec<&'k KnownEntry> {
    let (_, doc, variant, step, check) = key;
    let mut used: Vec<&KnownEntry> = Vec::new();
    for (column, (family, cell)) in columns.iter().zip(cells.iter_mut()) {
        let value = match cell {
            Cell::Value(v) | Cell::Excluded(_, v) => v.clone(),
            Cell::Absent => String::new(),
            Cell::NotRun | Cell::Infra => continue,
        };
        let failing = fails(key, cell);
        let entry = known.iter().find(|e| {
            e.kind != Kind::Flaky
                && e.matches(doc, variant, step, check)
                && e.covers(column.id(), family)
                && (!failing || e.covers_failure(doc, check))
        });
        if let Some(entry) = entry {
            *cell = Cell::Excluded(if failing { KNOWN_BUG } else { KNOWN }, value);
            if !used.contains(&entry) {
                used.push(entry);
            }
        } else if failing {
            // An unexplained failure keeps the row failing as it was.
            return Vec::new();
        }
    }
    used
}

// Rows and the known entries that explained at least one of them.
pub fn rows(columns: &[Column], known: &[KnownEntry]) -> (Vec<Row>, BTreeSet<usize>) {
    let mut keys: BTreeSet<(String, String, String, String, String)> = BTreeSet::new();
    for column in columns {
        if let Column::Loaded(l) = column {
            for (source, doc, variant, step, check) in l.values.keys() {
                // The environment goes to environment.md, not into the rows.
                if source != "env" {
                    keys.insert((doc.clone(), variant.clone(), step.clone(), source.clone(), check.clone()));
                }
            }
        }
    }
    let refs: Vec<&Column> = columns.iter().collect();
    let mut used_lines = BTreeSet::new();
    let rows = keys
        .into_iter()
        .map(|(doc, variant, step, source, check)| {
            let key = (source, doc, variant, step, check);
            let cells: Vec<(&str, Cell)> = columns.iter().map(|c| (c.family(), cell(c, &key))).collect();
            let plain = verdict(&key, &cells);
            let mut known_cells = cells.clone();
            let used = if plain == "same" || plain == "n/c" { Vec::new() } else { apply_known(&key, columns, &mut known_cells, known) };
            let after = verdict(&key, &known_cells);
            // An entry explains a row only when, without the cells it covers,
            // nothing is left that differs, fails or is missing.
            let explained = !used.is_empty() && ["same", "same-in-family", "n/c"].contains(&after);
            let (cells, verdict) = if explained {
                let bug = known_cells.iter().any(|(_, c)| matches!(c, Cell::Excluded(KNOWN_BUG, _)));
                used_lines.extend(used.iter().map(|e| e.line));
                (known_cells, if bug { KNOWN_BUG } else { KNOWN })
            } else {
                (cells, plain)
            };
            let mut note = note(&key, &refs, &cells, verdict);
            if explained {
                for entry in &used {
                    note.push_str(&format!("{}{}", if note.is_empty() { "" } else { "; " }, entry.describe()));
                }
            }
            Row { cells: cells.into_iter().map(|(_, c)| c).collect(), key, verdict, note }
        })
        .collect();
    (rows, used_lines)
}

pub const VERDICTS: [&str; 8] = ["FAIL", "missing", "diff-in-family", "known-bug", "known", "same-in-family", "n/c", "same"];

// What fails the corpus once known.tsv is given: a difference inside one x2t
// family, a value one platform lacks or a failure, with no entry that
// explains it. A FAIL row can hide a missing value that an entry left over.
pub fn is_unexplained(verdict: &str) -> bool {
    verdict == "diff-in-family" || verdict == "missing" || verdict == "FAIL"
}

fn short(value: &str) -> String {
    let hex = value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit());
    let limit = if hex { 16 } else { 60 };
    let cut: String = value.chars().take(limit).collect();
    let cut = if cut.len() < value.len() { format!("{}...", cut) } else { cut };
    clean(&cut)
}

fn columns_line(columns: &[Column]) -> String {
    let mut groups: Vec<(String, Vec<&str>)> = Vec::new();
    for column in columns {
        let family = match column {
            Column::Loaded(l) => format!("x2t {}", l.family),
            Column::Infra { .. } => "no results".to_string(),
        };
        match groups.iter_mut().find(|(f, _)| *f == family) {
            Some((_, ids)) => ids.push(column.id()),
            None => groups.push((family, vec![column.id()])),
        }
    }
    groups.iter().map(|(f, ids)| format!("{}: {}", f, ids.join(", "))).collect::<Vec<_>>().join(" | ")
}

const MD_ROWS: usize = 300;

pub fn markdown(columns: &[Column], rows: &[Row], unused: &[&KnownEntry]) -> String {
    let mut s = String::from("## Platforms side by side\n\n");
    s.push_str(&format!("Columns by x2t family: {}\n\n", columns_line(columns)));
    s.push_str("| Verdict | Rows |\n|---|---|\n");
    for verdict in VERDICTS {
        let count = rows.iter().filter(|r| r.verdict == verdict).count();
        s.push_str(&format!("| {} | {} |\n", verdict, count));
    }
    // Entries that explain nothing any more (after an x2t update, say) are
    // listed to be removed; they never fail the corpus.
    if !unused.is_empty() {
        s.push_str("\nKnown entries that explained no row in this run:\n\n");
        // known.tsv comes from the checkout, not from the results: its
        // patterns are shown as written, inside a code span.
        for entry in unused {
            let pattern = format!("{} {} {} {}", entry.doc, entry.variant, entry.step, entry.check).replace('`', "");
            s.push_str(&format!("- line {}: `{}`: {}\n", entry.line, pattern, clean(&entry.reason)));
        }
    }
    let shown: Vec<&Row> = rows.iter().filter(|r| r.verdict != "same").collect();
    if shown.is_empty() {
        return s;
    }
    s.push_str("\nEvery row that is not `same`, by document (all rows are in matrix.tsv). The extracted text of a \
                step is text/<doc>/<variant>/<step>.txt in each platform's results.\n");
    let header: Vec<&str> = columns.iter().map(Column::id).collect();
    let mut current_doc = None;
    for row in shown.iter().take(MD_ROWS) {
        let (source, doc, variant, step, check) = &row.key;
        if current_doc != Some(doc) {
            current_doc = Some(doc);
            s.push_str(&format!(
                "\n### `{}`\n\n| Variant | Step | Check | {} | Verdict | Note |\n|---|---|---|{}---|---|\n",
                clean(doc),
                header.join(" | "),
                "---|".repeat(header.len())
            ));
        }
        let cells: Vec<String> = row.cells.iter().map(Cell::markdown).collect();
        s.push_str(&format!(
            "| {} | {} | {}:{} | {} | {} | {} |\n",
            clean(variant),
            clean(step),
            clean(source),
            clean(check),
            cells.join(" | "),
            row.verdict,
            row.note.replace('|', "/")
        ));
    }
    if shown.len() > MD_ROWS {
        s.push_str(&format!("\n{} more rows in matrix.tsv.\n", shown.len() - MD_ROWS));
    }
    s
}

pub fn table(columns: &[Column], rows: &[Row]) -> Table {
    let mut header = vec!["source", "doc", "variant", "step", "check"];
    header.extend(columns.iter().map(Column::id));
    header.extend(["verdict", "note"]);
    let mut table = Table::new(&header);
    for row in rows {
        let (source, doc, variant, step, check) = &row.key;
        let mut cells = vec![source.clone(), doc.clone(), variant.clone(), step.clone(), check.clone()];
        cells.extend(row.cells.iter().map(Cell::text));
        cells.push(row.verdict.to_string());
        cells.push(row.note.clone());
        table.push(cells);
    }
    table
}

// The value as recorded, for decisions.
fn env_raw<'a>(l: &'a Loaded, key: &str) -> &'a str {
    l.env.get(key).map(String::as_str).unwrap_or_default()
}

// The value as shown.
fn env_cell(l: &Loaded, key: &str) -> String {
    clean(env_raw(l, key))
}

const FONT_NAMES_SHOWN: usize = 50;
const PLATFORM_LINES_SHOWN: usize = 100;

pub fn environment(columns: &[Column]) -> String {
    let loaded: Vec<&Loaded> = columns
        .iter()
        .filter_map(|c| match c {
            Column::Loaded(l) => Some(l.as_ref()),
            _ => None,
        })
        .collect();
    let mut s = String::from("## Environment per platform\n\n");
    let ids: Vec<&str> = columns.iter().map(Column::id).collect();
    s.push_str(&format!("| | {} |\n|---|{}\n", ids.join(" | "), "---|".repeat(ids.len())));
    type Getter = dyn Fn(&Loaded) -> String;
    let app = |l: &Loaded, check: &str| -> String {
        let values: BTreeSet<&str> = l
            .report
            .rows
            .iter()
            .filter(|r| l.report.get(r, "check") == check)
            .map(|r| l.report.get(r, "value"))
            .filter(|v| !v.is_empty())
            .collect();
        values.into_iter().map(clean).collect::<Vec<_>>().join(", ")
    };
    let lines: Vec<(&str, Box<Getter>)> = vec![
        ("x2t family", Box::new(|l: &Loaded| l.family.clone())),
        (
            "Runner",
            Box::new(|l: &Loaded| {
                let p = key_values(&l.platform_text);
                let get = |k: &str| clean(p.get(k).map(String::as_str).unwrap_or_default());
                format!("{} {}", get("ImageOS"), get("ImageVersion"))
            }),
        ),
        ("Installer", Box::new(|l: &Loaded| clean(l.package.get("Installer").map(String::as_str).unwrap_or_default()))),
        ("Installer sha256", Box::new(|l: &Loaded| short(&l.package.get("SHA256").cloned().unwrap_or_default()))),
        ("Sidecar x2t", Box::new(|l: &Loaded| clean(&full_version(env_raw(l, "sidecar.version"))))),
        ("PDF x2t", Box::new(|l: &Loaded| clean(&full_version(env_raw(l, "pdf_x2t.version"))))),
        ("Same x2t file", Box::new(|l: &Loaded| env_cell(l, "x2t_same_file"))),
        (
            "Architecture (host / sidecar / PDF x2t)",
            Box::new(|l: &Loaded| {
                let host = env_raw(l, "host.arch");
                let (a, b) = (env_raw(l, "sidecar.arch"), env_raw(l, "pdf_x2t.arch"));
                let emulated = crate::arch::emulated(a, host) || crate::arch::emulated(b, host);
                format!("{} / {} / {}{}", clean(host), clean(a), clean(b), if emulated { " (emulated)" } else { "" })
            }),
        ),
        (
            "Font cache (total / bundled / system)",
            Box::new(|l: &Loaded| format!("{} / {} / {}", env_cell(l, "fonts.total"), env_cell(l, "fonts.bundled"), env_cell(l, "fonts.system"))),
        ),
        ("Liberation and Carlito", Box::new(|l: &Loaded| env_cell(l, "fonts.check"))),
        ("AllFonts.js sha256", Box::new(|l: &Loaded| short(env_raw(l, "allfonts.sha256")))),
        ("app.application", Box::new(move |l: &Loaded| app(l, "app.application"))),
        ("app.version", Box::new(move |l: &Loaded| app(l, "app.version"))),
    ];
    for (n, (label, get)) in lines.iter().enumerate() {
        // The reason a platform has no results is under the runner notes.
        let cells: Vec<String> = columns
            .iter()
            .map(|c| match c {
                Column::Loaded(l) => get(l).replace('|', "/"),
                Column::Infra { .. } if n == 0 => "no results".to_string(),
                Column::Infra { .. } => String::new(),
            })
            .collect();
        s.push_str(&format!("| {} | {} |\n", label, cells.join(" | ")));
    }

    let mut by_family: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for l in &loaded {
        by_family.entry(l.family.as_str()).or_default().insert(full_version(env_raw(l, "pdf_x2t.version")));
    }
    for (family, versions) in by_family.iter().filter(|(_, v)| v.len() > 1) {
        s.push_str(&format!(
            "\nWarning: the x2t {} family holds different versions: {}.\n",
            family,
            versions.iter().map(|v| clean(v)).collect::<Vec<_>>().join(", ")
        ));
    }

    s.push_str("\n### Font families not on every platform\n\n");
    let common: BTreeSet<String> = match loaded.split_first() {
        Some((first, rest)) => first
            .font_names
            .iter()
            .filter(|name| rest.iter().all(|l| l.font_names.contains(*name)))
            .cloned()
            .collect(),
        None => BTreeSet::new(),
    };
    for l in &loaded {
        let only: Vec<&String> = l.font_names.iter().filter(|n| !common.contains(*n)).collect();
        let mut listed: Vec<String> = only.iter().take(FONT_NAMES_SHOWN).map(|n| clean(n)).collect();
        if only.len() > FONT_NAMES_SHOWN {
            listed.push(format!("and {} more", only.len() - FONT_NAMES_SHOWN));
        }
        s.push_str(&format!("- {} ({} families, {} not on every platform): {}\n", l.id, l.font_names.len(), only.len(), listed.join(", ")));
    }

    s.push_str("\n### Fonts embedded in the comparable PDFs\n\nA (!) marks a font other than Liberation or Carlito.\n\n");
    s.push_str(&format!("| Document | Variant | {} |\n|---|---|{}\n", ids.join(" | "), "---|".repeat(ids.len())));
    let mut pdfs: BTreeSet<(String, String)> = BTreeSet::new();
    for l in &loaded {
        for row in l.report.rows.iter().filter(|r| l.report.get(r, "step") == "pdf" && l.report.get(r, "check") == "fonts") {
            let (doc, variant) = (l.report.get(row, "doc"), l.report.get(row, "variant"));
            if l.entries.iter().any(|e| e.doc == doc && e.class == "comparable") {
                pdfs.insert((doc.to_string(), variant.to_string()));
            }
        }
    }
    for (doc, variant) in &pdfs {
        let cells: Vec<String> = columns
            .iter()
            .map(|c| {
                let Column::Loaded(l) = c else { return String::new() };
                let value = l
                    .report
                    .rows
                    .iter()
                    .find(|r| {
                        l.report.get(r, "doc") == doc
                            && l.report.get(r, "variant") == variant
                            && l.report.get(r, "step") == "pdf"
                            && l.report.get(r, "check") == "fonts"
                    })
                    .map(|r| l.report.get(r, "value"))
                    .unwrap_or("");
                let mut names: Vec<String> = value
                    .split(", ")
                    .filter_map(|f| f.split('|').next())
                    .filter(|n| !n.is_empty())
                    .map(|n| if pdf::allowed(n) { clean(n) } else { format!("{} (!)", clean(n)) })
                    .collect();
                names.dedup();
                names.join(", ")
            })
            .collect();
        s.push_str(&format!("| `{}` | {} | {} |\n", clean(doc), clean(variant), cells.join(" | ")));
    }

    s.push_str("\n### Runner notes\n");
    for column in columns {
        match column {
            Column::Loaded(l) => {
                let lines: Vec<&str> = l.platform_text.lines().collect();
                s.push_str(&format!("\n<details><summary>{} ({})</summary>\n\n```\n", l.id, clean(&l.dir.display().to_string())));
                for line in lines.iter().take(PLATFORM_LINES_SHOWN) {
                    s.push_str(&format!("{}\n", clean(line)));
                }
                if lines.len() > PLATFORM_LINES_SHOWN {
                    s.push_str(&format!("... {} more lines in platform.txt\n", lines.len() - PLATFORM_LINES_SHOWN));
                }
                s.push_str("```\n\n</details>\n");
            }
            Column::Infra { id, reason } => s.push_str(&format!("\n- {}: no results ({})\n", id, clean(reason))),
        }
    }
    s
}

pub fn matrix(args: &MatrixArgs) -> Result<Outcome, Infra> {
    let mut ids: Vec<String> = args.expect.clone();
    for (id, _) in &args.platforms {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let mut columns = Vec::new();
    for id in &ids {
        let column = match args.platforms.iter().find(|(p, _)| p == id) {
            None => Column::Infra { id: id.clone(), reason: "no results folder given".to_string() },
            Some((_, dir)) if !dir.is_dir() => Column::Infra { id: id.clone(), reason: format!("{} does not exist", dir.display()) },
            Some((_, dir)) => match load(id, dir) {
                Ok(loaded) => Column::Loaded(Box::new(loaded)),
                Err(reason) => Column::Infra { id: id.clone(), reason },
            },
        };
        columns.push(column);
    }
    let columns = order(columns);
    let known = args.known.as_deref().unwrap_or_default();
    let (rows, used) = rows(&columns, known);
    let unused: Vec<&KnownEntry> = known.iter().filter(|e| e.kind != Kind::Flaky && !used.contains(&e.line)).collect();
    std::fs::create_dir_all(&args.out).map_err(|e| Infra(format!("cannot create {}: {}", args.out.display(), e)))?;
    table(&columns, &rows).write(&args.out.join("matrix.tsv")).map_err(Infra)?;
    let write = |name: &str, text: String| {
        let path = args.out.join(name);
        std::fs::write(&path, text).map_err(|e| Infra(format!("cannot write {}: {}", path.display(), e)))
    };
    write("matrix.md", markdown(&columns, &rows, &unused))?;
    write("environment.md", environment(&columns))?;
    Ok(Outcome {
        infra: columns.iter().filter(|c| matches!(c, Column::Infra { .. })).count(),
        unexplained: if args.known.is_some() { rows.iter().filter(|r| is_unexplained(r.verdict)).count() } else { 0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "file\tclass\tsteps\texpect\tvariants\tsave_as\tnote\n\
a.docx\tcomparable\topen,save,pdf\tHello\t\t\t\n\
b.csv\tcomparable\topen,save\t\t\t\t\n\
s.docx\tsystem-fonts\topen,pdf\t\t\t\t\n";

    // One platform's results, with a few values that tests then change.
    struct Fake {
        version: &'static str,
        check: &'static str,
        pdf_text: &'static str,
        pdf_status: &'static str,
        pdf_expected: &'static str,
        csv_size: &'static str,
        s_text: &'static str,
        save_class: &'static str,
        with_save: bool,
        // A fonts.script row for s.docx's PDF when not empty.
        script: &'static str,
    }

    impl Default for Fake {
        fn default() -> Fake {
            Fake {
                version: "OOX/binary file converter. Version: 9.4.0.129",
                check: "ok",
                pdf_text: "t1",
                pdf_status: "ok",
                pdf_expected: "ok",
                csv_size: "1600",
                s_text: "s1",
                save_class: "comparable",
                with_save: true,
                script: "",
            }
        }
    }

    fn write(root: &Path, id: &str, fake: Fake) -> PathBuf {
        let dir = root.join(id).join("run1");
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        let env = format!(
            "key\tvalue\nhost.arch\tx86_64\nexe\t/{id}/euro-office-lite\npdf_x2t.version\t{}\nsidecar.version\t{}\nfonts.check\t{}\n",
            fake.version, fake.version, fake.check
        );
        let mut run = String::from("doc\tvariant\tstep\tstatus\texit\tsignal\tsize\tsha256\toutput\tms\tmessage\n");
        run.push_str("a.docx\tplain\topen\tok\t0\t\t10\tbin\t\t5\t\n");
        if fake.with_save {
            run.push_str("a.docx\tplain\tsave\tok\t0\t\t900\tzip\tfiles/a.docx/plain/save.docx\t5\t\n");
        }
        run.push_str(&format!("a.docx\tplain\tpdf\t{}\t0\t\t900\tpdf\tfiles/a.docx/plain/export.pdf\t5\t\n", fake.pdf_status));
        run.push_str(&format!("b.csv\tplain\topen\tok\t0\t\t{}\tcsvbin-{id}\t\t5\t\n", fake.csv_size));
        run.push_str("s.docx\tplain\topen\tok\t0\t\t10\tsbin\t\t5\t\n");
        run.push_str("s.docx\tplain\tpdf\tok\t0\t\t900\tpdf\tfiles/s.docx/plain/export.pdf\t5\t\n");
        let mut report = String::from("doc\tvariant\tstep\tcheck\tvalue\n");
        if fake.with_save {
            report.push_str(&format!("a.docx\tplain\tsave\ttext.sha256\tsave-text\na.docx\tplain\tsave\tclass\t{}\n", fake.save_class));
            report.push_str(&format!("a.docx\tplain\tsave\tapp.application\tx2t-{id}\n"));
        }
        report.push_str(&format!(
            "a.docx\tplain\tpdf\ttext.sha256\t{}\na.docx\tplain\tpdf\texpected\t{}\na.docx\tplain\tpdf\tfonts\tLiberationSans|TrueType|emb=yes|sub=yes|uni=yes\na.docx\tplain\tpdf\tclass\t{}\n",
            fake.pdf_text,
            fake.pdf_expected,
            if fake.check == "ok" { "comparable" } else { "spoofed" }
        ));
        report.push_str(&format!("s.docx\tplain\tpdf\ttext.sha256\t{}\ns.docx\tplain\tpdf\tclass\tsystem-fonts\n", fake.s_text));
        if !fake.script.is_empty() {
            report.push_str(&format!("s.docx\tplain\tpdf\tfonts.script\t{}\n", fake.script));
        }
        std::fs::write(dir.join("env.tsv"), env).unwrap();
        std::fs::write(dir.join("run.tsv"), run).unwrap();
        std::fs::write(dir.join("report.tsv"), report).unwrap();
        std::fs::write(dir.join("manifest.tsv"), MANIFEST).unwrap();
        std::fs::write(
            dir.join("fonts/AllFonts.js"),
            format!("window[\"__fonts_files\"] = [];\nwindow[\"__fonts_infos\"] = [[\"Carlito\",0,0,-1,0,-1,0,-1,0],[\"Only {id}\",0,0,-1,0,-1,0,-1,0]];\n"),
        )
        .unwrap();
        std::fs::write(root.join(id).join("package.txt"), format!("Installer: eol-{id}.bin\nSHA256: {}\n", "ab".repeat(32))).unwrap();
        std::fs::write(root.join(id).join("platform.txt"), format!("Platform: {id}\nImageOS: img-{id}\nImageVersion: 20261018.1\n")).unwrap();
        dir
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("corpus-matrix-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const V82: &str = "OOX/binary file converter. Version: 8.2.0.147";

    fn run_matrix(root: &Path, fakes: Vec<(&str, Fake)>, expect: &[&str]) -> (usize, Table, String, String) {
        let (outcome, tsv, md, env) = run_matrix_known(root, fakes, expect, None);
        (outcome.infra, tsv, md, env)
    }

    fn run_matrix_known(root: &Path, fakes: Vec<(&str, Fake)>, expect: &[&str], known: Option<&str>) -> (Outcome, Table, String, String) {
        let platforms = fakes.into_iter().map(|(id, fake)| (id.to_string(), write(root, id, fake))).collect();
        let known = known.map(|rows| crate::known::parse(&format!("{}{}", crate::known::COLUMNS.join("\t") + "\n", rows)).unwrap());
        let args = MatrixArgs {
            platforms,
            expect: expect.iter().map(|s| s.to_string()).collect(),
            out: root.join("matrix"),
            known,
        };
        let outcome = matrix(&args).unwrap();
        let tsv = Table::read(&root.join("matrix/matrix.tsv")).unwrap();
        let md = std::fs::read_to_string(root.join("matrix/matrix.md")).unwrap();
        let env = std::fs::read_to_string(root.join("matrix/environment.md")).unwrap();
        (outcome, tsv, md, env)
    }

    fn verdict_of(tsv: &Table, source: &str, doc: &str, step: &str, check: &str) -> String {
        let row = tsv
            .rows
            .iter()
            .find(|r| tsv.get(r, "source") == source && tsv.get(r, "doc") == doc && tsv.get(r, "step") == step && tsv.get(r, "check") == check)
            .unwrap_or_else(|| panic!("no row {} {} {} {}", source, doc, step, check));
        tsv.get(row, "verdict").to_string()
    }

    #[test]
    fn the_family_is_read_from_the_version_line() {
        assert_eq!(family("OOX/binary file converter. Version: 8.2.0.147"), "8.2");
        assert_eq!(family("x2t Version: 9.4.0.129 (build 1)"), "9.4");
        assert_eq!(family("unknown"), "unknown");
        assert_eq!(family(""), "unknown");
    }

    #[test]
    fn three_platforms_in_two_families() {
        let root = scratch("three");
        let (missing, tsv, md, env) = run_matrix(
            &root,
            vec![
                ("win-x64", Fake::default()),
                // Same x2t as win-x64, another PDF text: the case that matters.
                ("win-arm64", Fake { pdf_text: "t2", csv_size: "1610", ..Fake::default() }),
                ("linux-x64", Fake { version: V82, s_text: "s9", ..Fake::default() }),
            ],
            &[],
        );
        assert_eq!(missing, 0);
        // Grouped by family, and 9.4 first because win-x64 comes first.
        assert_eq!(&tsv.header[5..8], ["win-x64", "win-arm64", "linux-x64"]);
        assert_eq!(verdict_of(&tsv, "report", "a.docx", "pdf", "text.sha256"), "diff-in-family");
        assert_eq!(verdict_of(&tsv, "report", "a.docx", "save", "text.sha256"), "same");
        assert_eq!(verdict_of(&tsv, "run", "a.docx", "open", "status"), "same");
        // CSV Editor.bin: bytes are not compared, size within 1%.
        assert_eq!(verdict_of(&tsv, "run", "b.csv", "open", "size"), "same");
        assert!(!tsv.rows.iter().any(|r| tsv.get(r, "doc") == "b.csv" && tsv.get(r, "check") == "sha256"));
        // A system-fonts document is shown but never compared.
        assert_eq!(verdict_of(&tsv, "report", "s.docx", "pdf", "text.sha256"), "n/c");
        // Environment and app.* never become rows.
        assert!(!tsv.rows.iter().any(|r| tsv.get(r, "source") == "env" || tsv.get(r, "check").starts_with("app.")));
        assert!(md.contains("Columns by x2t family: x2t 9.4: win-x64, win-arm64 | x2t 8.2: linux-x64"), "{}", md);
        assert!(md.contains("| diff-in-family | 1 |"), "{}", md);
        assert!(env.contains("| app.application | x2t-win-x64 | x2t-win-arm64 | x2t-linux-x64 |"), "{}", env);
        assert!(env.contains("img-win-arm64 20261018.1"), "{}", env);
        assert!(env.contains("Only win-x64"), "{}", env);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn six_platforms_with_every_verdict() {
        let root = scratch("six");
        let all = ["win-x64", "win-arm64", "linux-arm64", "linux-x64", "mac-x64", "mac-arm64"];
        let (missing, tsv, md, env) = run_matrix(
            &root,
            vec![
                ("win-x64", Fake { csv_size: "1700", ..Fake::default() }),
                ("win-arm64", Fake { pdf_status: "signal", ..Fake::default() }),
                ("linux-arm64", Fake { with_save: false, ..Fake::default() }),
                ("linux-x64", Fake { version: V82, pdf_text: "t8", ..Fake::default() }),
                ("mac-x64", Fake { version: V82, pdf_text: "t8", check: "spoofed: Carlito regular system", pdf_expected: "missing:Hello", ..Fake::default() }),
                ("mac-arm64", Fake { version: V82, pdf_text: "t8", save_class: "auto-system-fonts", ..Fake::default() }),
            ],
            &all,
        );
        assert_eq!(missing, 0);
        assert_eq!(verdict_of(&tsv, "run", "a.docx", "pdf", "status"), "FAIL");
        assert_eq!(verdict_of(&tsv, "report", "a.docx", "pdf", "expected"), "FAIL");
        assert_eq!(verdict_of(&tsv, "report", "a.docx", "save", "text.sha256"), "missing");
        assert_eq!(verdict_of(&tsv, "report", "a.docx", "pdf", "text.sha256"), "same-in-family");
        assert_eq!(verdict_of(&tsv, "run", "b.csv", "open", "size"), "diff-in-family");
        assert_eq!(verdict_of(&tsv, "run", "a.docx", "open", "sha256"), "same");
        let row = tsv
            .rows
            .iter()
            .find(|r| tsv.get(r, "doc") == "a.docx" && tsv.get(r, "step") == "pdf" && tsv.get(r, "check") == "text.sha256")
            .unwrap();
        assert_eq!(tsv.get(row, "mac-x64"), "n/c:spoofed t8");
        assert!(tsv.get(row, "note").contains("n/c:spoofed on mac-x64"), "{}", tsv.get(row, "note"));
        let save = tsv
            .rows
            .iter()
            .find(|r| tsv.get(r, "doc") == "a.docx" && tsv.get(r, "step") == "save" && tsv.get(r, "check") == "text.sha256")
            .unwrap();
        assert_eq!(tsv.get(save, "mac-arm64"), "n/c:auto-system-fonts save-text");
        assert_eq!(tsv.get(save, "linux-arm64"), "<missing>");
        assert!(md.contains("| FAIL | 2 |"), "{}", md);
        assert!(env.contains("spoofed: Carlito regular system"), "{}", env);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_expected_platform_without_results_is_infra() {
        let root = scratch("infra");
        let (missing, tsv, md, env) = run_matrix(
            &root,
            vec![("linux-x64", Fake::default()), ("linux-arm64", Fake::default())],
            &["win-x64", "linux-x64", "linux-arm64"],
        );
        assert_eq!(missing, 1);
        // Loaded platforms first, the one without results at the end.
        assert_eq!(&tsv.header[5..8], ["linux-x64", "linux-arm64", "win-x64"]);
        assert!(tsv.rows.iter().all(|r| tsv.get(r, "win-x64") == "<infra>"));
        assert_eq!(verdict_of(&tsv, "run", "a.docx", "open", "status"), "same");
        assert!(md.contains("no results: win-x64"), "{}", md);
        assert!(env.contains("| x2t family | 9.4 | 9.4 | no results |"), "{}", env);
        assert!(env.contains("- win-x64: no results (no results folder given)"), "{}", env);

        // A folder without the run's tables counts the same.
        let broken = root.join("broken/run1");
        std::fs::create_dir_all(&broken).unwrap();
        let args = MatrixArgs { platforms: vec![("broken".into(), broken)], expect: Vec::new(), out: root.join("m2"), known: None };
        assert_eq!(matrix(&args).unwrap().infra, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    // Whatever the results carry ends up in the run's Markdown summary: it
    // must not open a heading, a link, HTML or a code span there.
    #[test]
    fn markdown_from_the_results_keeps_plain_characters_only() {
        let root = scratch("markdown");
        let payload = "x\n## Owned\n[link](http://evil.example)\n<img src=x onerror=alert(1)>\t|`code`@user";
        let dir = write(&root, "win-x64", Fake::default());
        let escaped = crate::tsv::escape(payload);
        let env = std::fs::read_to_string(dir.join("env.tsv")).unwrap();
        std::fs::write(dir.join("env.tsv"), format!("{}sidecar.arch\t{}\nfonts.total\t{}\n", env, escaped, escaped)).unwrap();
        let report = std::fs::read_to_string(dir.join("report.tsv")).unwrap();
        std::fs::write(dir.join("report.tsv"), report.replace("\tt1\n", &format!("\t{}\n", escaped))).unwrap();
        std::fs::write(
            dir.join("fonts/AllFonts.js"),
            format!("window[\"__fonts_files\"] = [];\nwindow[\"__fonts_infos\"] = [[\"{}\",0,0,-1,0,-1,0,-1,0]];\n", payload.replace('\n', "\\n").replace('"', "'")),
        )
        .unwrap();
        let parent = dir.parent().unwrap();
        std::fs::write(parent.join("platform.txt"), format!("ImageOS: {}\n{}\n", payload, payload)).unwrap();
        std::fs::write(parent.join("package.txt"), format!("Installer: {}\n", payload)).unwrap();
        let other = write(&root, "linux-x64", Fake::default());
        let args = MatrixArgs {
            platforms: vec![("win-x64".into(), dir), ("linux-x64".into(), other)],
            expect: Vec::new(),
            out: root.join("matrix"),
            known: None,
        };
        matrix(&args).unwrap();
        for name in ["matrix.md", "environment.md"] {
            let text = std::fs::read_to_string(root.join("matrix").join(name)).unwrap();
            for bad in ["## Owned", "[link]", "](http", "<img", "`code`", "@user", "\t"] {
                assert!(!text.contains(bad), "{} holds {:?}:\n{}", name, bad, text);
            }
        }
        assert_eq!(clean("a\nb|c<d>`e` é.-_:/+()"), "a?b?c?d??e? ?.-_:/+()");
        let _ = std::fs::remove_dir_all(&root);
    }

    // Emulation is decided on the values as recorded; only the shown text is
    // cleaned.
    #[test]
    fn a_universal_x2t_is_native_on_either_host() {
        let root = scratch("arch");
        let mut platforms = Vec::new();
        for (id, host, x2t) in [
            ("mac-x64", "x86_64", "universal:x86_64,aarch64"),
            ("mac-arm64", "aarch64", "universal:x86_64,aarch64"),
            ("win-arm64", "aarch64", "x86_64"),
        ] {
            let dir = write(&root, id, Fake::default());
            let env = std::fs::read_to_string(dir.join("env.tsv")).unwrap().replace("host.arch\tx86_64\n", "");
            std::fs::write(dir.join("env.tsv"), format!("{}host.arch\t{}\nsidecar.arch\t{}\npdf_x2t.arch\t{}\n", env, host, x2t, x2t)).unwrap();
            platforms.push((id.to_string(), dir));
        }
        matrix(&MatrixArgs { platforms, expect: Vec::new(), out: root.join("matrix"), known: None }).unwrap();
        let env = std::fs::read_to_string(root.join("matrix/environment.md")).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let row = env.lines().find(|l| l.starts_with("| Architecture")).unwrap();
        assert_eq!(
            row,
            "| Architecture (host / sidecar / PDF x2t) | x86_64 / universal:x86_64,aarch64 / universal:x86_64,aarch64 \
             | aarch64 / universal:x86_64,aarch64 / universal:x86_64,aarch64 | aarch64 / x86_64 / x86_64 (emulated) |"
        );
    }

    fn verdict_and_note(tsv: &Table, source: &str, doc: &str, step: &str, check: &str) -> (String, String) {
        let row = tsv
            .rows
            .iter()
            .find(|r| tsv.get(r, "source") == source && tsv.get(r, "doc") == doc && tsv.get(r, "step") == step && tsv.get(r, "check") == check)
            .unwrap_or_else(|| panic!("no row {} {} {} {}", source, doc, step, check));
        (tsv.get(row, "verdict").to_string(), tsv.get(row, "note").to_string())
    }

    fn nine_four(pdf_text: &'static str) -> Fake {
        Fake { pdf_text, ..Fake::default() }
    }

    // An entry for the two Windows platforms explains a difference they share,
    // never one a third platform shows.
    #[test]
    fn a_known_entry_explains_only_the_platforms_it_names() {
        let platforms = || vec![("win-x64", nine_four("t2")), ("win-arm64", nine_four("t2")), ("linux-arm64", nine_four("t1"))];
        let root = scratch("known-two");
        let windows = "a.docx\tplain\tpdf\ttext.sha256\twin-x64,win-arm64\tenv\tWindows lays the page out\t\n";
        let (outcome, tsv, md, _) = run_matrix_known(&root, platforms(), &[], Some(windows));
        let (verdict, note) = verdict_and_note(&tsv, "report", "a.docx", "pdf", "text.sha256");
        assert_eq!(verdict, "known");
        assert!(note.contains("known.tsv line 2 (env): Windows lays the page out"), "{}", note);
        assert_eq!(outcome, Outcome::default());
        assert!(md.contains("| known | 1 |"), "{}", md);

        let one = "a.docx\tplain\tpdf\ttext.sha256\twin-x64\tenv\tonly one\t\n";
        let (outcome, tsv, _, _) = run_matrix_known(&root, platforms(), &[], Some(one));
        assert_eq!(verdict_and_note(&tsv, "report", "a.docx", "pdf", "text.sha256").0, "diff-in-family");
        assert_eq!(outcome, Outcome { infra: 0, unexplained: 1 });

        // Without known.tsv the difference is reported, not failed.
        let (outcome, _, _, _) = run_matrix_known(&root, platforms(), &[], None);
        assert_eq!(outcome, Outcome::default());
        let _ = std::fs::remove_dir_all(&root);
    }

    // A family entry applies to the platforms that show that family in the run.
    #[test]
    fn a_family_entry_follows_the_x2t_each_platform_runs() {
        let root = scratch("known-family");
        let entry = "a.docx\tplain\tsave\t*\t8.2\tx2t\t8.2 does not write it\t\n";
        let no_save = |version| Fake { version, with_save: false, ..Fake::default() };
        let (outcome, tsv, _, _) =
            run_matrix_known(&root, vec![("win-x64", Fake::default()), ("mac-x64", no_save(V82)), ("linux-x64", no_save(V82))], &[], Some(entry));
        assert_eq!(verdict_and_note(&tsv, "report", "a.docx", "save", "text.sha256").0, "known");
        // An absent status stays out of the failures once an entry explains it.
        assert_eq!(verdict_and_note(&tsv, "run", "a.docx", "save", "status").0, "known");
        assert_eq!(outcome, Outcome::default());

        // The same gap on a 9.4 platform is not what the entry explains.
        let nine_four_no_save = Fake { with_save: false, ..Fake::default() };
        let (outcome, tsv, _, _) =
            run_matrix_known(&root, vec![("win-x64", Fake::default()), ("mac-x64", no_save(V82)), ("linux-arm64", nine_four_no_save)], &[], Some(entry));
        assert_eq!(verdict_and_note(&tsv, "report", "a.docx", "save", "text.sha256").0, "missing");
        assert!(outcome.unexplained > 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    // A crash or a script PDF without its font is never explained by an env
    // or x2t entry, nor by a bug entry with a wildcard: only by a bug entry
    // that names the document and the check.
    #[test]
    fn a_failure_needs_a_bug_entry_that_names_it() {
        let root = scratch("known-fail");
        let platforms = || {
            vec![
                ("win-x64", Fake { pdf_status: "signal", script: "bundled-only", ..Fake::default() }),
                ("win-arm64", Fake { script: "ok", ..Fake::default() }),
                ("linux-arm64", Fake { script: "ok", ..Fake::default() }),
            ]
        };
        for broad in [
            "a.docx\tplain\tpdf\t*\tall\tenv\tany difference\t\ns.docx\tplain\tpdf\t*\tall\tx2t\tany difference\t\n",
            "a.docx\tplain\tpdf\tstat*\twin-x64\tbug\ttoo wide\t#9\ns.docx\t*\tpdf\tfonts.script\twin-x64\tenv\tnot a bug entry\t\n",
        ] {
            let (_, tsv, _, _) = run_matrix_known(&root, platforms(), &[], Some(broad));
            assert_eq!(verdict_and_note(&tsv, "run", "a.docx", "pdf", "status").0, "FAIL", "{}", broad);
            assert_eq!(verdict_and_note(&tsv, "report", "s.docx", "pdf", "fonts.script").0, "FAIL", "{}", broad);
        }
        let named = "a.docx\tplain\tpdf\tstatus\twin-x64\tbug\tcrash being fixed\t#9\ns.docx\tplain\tpdf\tfonts.script\twin-x64\tbug\tWindows font being fixed\t#10\n";
        let (_, tsv, md, _) = run_matrix_known(&root, platforms(), &[], Some(named));
        let (verdict, note) = verdict_and_note(&tsv, "run", "a.docx", "pdf", "status");
        assert_eq!(verdict, "known-bug");
        assert!(note.contains("#9"), "{}", note);
        assert_eq!(verdict_and_note(&tsv, "report", "s.docx", "pdf", "fonts.script").0, "known-bug");
        assert!(md.contains("| known-bug | 2 |"), "{}", md);
        let _ = std::fs::remove_dir_all(&root);
    }

    // A bug entry for one platform's failure leaves another platform's absent
    // value unexplained: the row stays FAIL and fails the corpus.
    #[test]
    fn a_fail_row_left_with_a_missing_value_is_unexplained() {
        let root = scratch("known-fail-missing");
        let mut platforms = Vec::new();
        for (id, expected) in [("win-x64", "missing:Hello"), ("win-arm64", "ok"), ("linux-arm64", "ok")] {
            platforms.push((id.to_string(), write(&root, id, Fake { pdf_expected: expected, ..Fake::default() })));
        }
        // linux-arm64 has no expected row for the PDF.
        let report = platforms[2].1.join("report.tsv");
        let text = std::fs::read_to_string(&report).unwrap();
        std::fs::write(&report, text.replace("a.docx\tplain\tpdf\texpected\tok\n", "")).unwrap();
        let bug = "a.docx\tplain\tpdf\texpected\twin-x64\tbug\ttext being fixed\t#9\n";
        let known = crate::known::parse(&format!("{}\n{}", crate::known::COLUMNS.join("\t"), bug)).unwrap();
        let args = MatrixArgs { platforms, expect: Vec::new(), out: root.join("matrix"), known: Some(known) };
        let outcome = matrix(&args).unwrap();
        let tsv = Table::read(&root.join("matrix/matrix.tsv")).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(verdict_of(&tsv, "report", "a.docx", "pdf", "expected"), "FAIL");
        assert_eq!(outcome, Outcome { infra: 0, unexplained: 1 });
    }

    #[test]
    fn entries_that_explain_nothing_are_listed_and_do_not_fail() {
        let root = scratch("known-unused");
        let gone = "gone.docx\t*\t*\t*\tall\tenv\tfixed since\t\n";
        let (outcome, _, md, _) = run_matrix_known(&root, vec![("win-x64", Fake::default()), ("linux-arm64", Fake::default())], &[], Some(gone));
        assert_eq!(outcome, Outcome::default());
        assert!(md.contains("Known entries that explained no row in this run:\n\n- line 2: `gone.docx * * *`: fixed since"), "{}", md);
        let _ = std::fs::remove_dir_all(&root);
    }

    // What report fails on fails the row too, whatever class excludes it.
    #[test]
    fn the_script_font_report_errors_and_txt_round_trips_are_hard_failures() {
        let key = |doc: &str, step: &str, check: &str| -> Key { ("report".into(), doc.into(), "plain".into(), step.into(), check.into()) };
        let v = |s: &str| Cell::Value(s.into());
        let script = key("arabic-rtl.docx", "pdf", "fonts.script");
        assert_eq!(verdict(&script, &[("9.4", v("ok")), ("8.2", v("ok"))]), "same");
        assert_eq!(verdict(&script, &[("9.4", v("ok")), ("8.2", Cell::Excluded("system-fonts", "bundled-only".into()))]), "FAIL");
        assert_eq!(verdict(&script, &[("9.4", v("other-font:Arial")), ("9.4", v("other-font:Arial"))]), "FAIL");
        let txt = key("utf16be.txt", "save", "roundtrip.bytes");
        assert_eq!(verdict(&txt, &[("9.4", v("diff:3")), ("8.2", v("equal"))]), "FAIL");
        let csv = key("empty-lines.csv", "save", "roundtrip.bytes");
        assert_eq!(verdict(&csv, &[("9.4", v("diff:8")), ("8.2", v("equal"))]), "same-in-family");
        assert_eq!(verdict(&key("a.docx", "save", "report.error"), &[("9.4", v("unzip failed")), ("8.2", v("unzip failed"))]), "FAIL");
    }

    #[test]
    fn verdicts_from_cells() {
        let key = |check: &str| -> Key { ("report".into(), "a.docx".into(), "plain".into(), "pdf".into(), check.into()) };
        let v = |s: &str| Cell::Value(s.into());
        assert_eq!(verdict(&key("pages"), &[("9.4", v("1")), ("8.2", v("1"))]), "same");
        assert_eq!(verdict(&key("pages"), &[("9.4", v("1")), ("9.4", v("1")), ("8.2", v("2"))]), "same-in-family");
        assert_eq!(verdict(&key("pages"), &[("9.4", v("1")), ("9.4", v("2")), ("8.2", v("2"))]), "diff-in-family");
        assert_eq!(verdict(&key("pages"), &[("9.4", v("1")), ("8.2", Cell::Excluded("spoofed", "2".into()))]), "n/c");
        assert_eq!(verdict(&key("pages"), &[("9.4", v("1")), ("8.2", Cell::NotRun), ("8.2", Cell::Infra)]), "n/c");
        assert_eq!(verdict(&key("pages"), &[("9.4", v("1")), ("9.4", v("1")), ("8.2", Cell::Absent)]), "missing");
        assert_eq!(verdict(&key("status"), &[("9.4", v("ok")), ("8.2", v("expected-fail")), ("8.2", v("expected-fail"))]), "same-in-family");
        assert_eq!(verdict(&key("status"), &[("9.4", v("ok")), ("8.2", v("skipped"))]), "FAIL");
        assert_eq!(verdict(&key("expected"), &[("9.4", v("ok")), ("8.2", Cell::Excluded("system-fonts", "missing:Hi".into()))]), "FAIL");
    }
}
