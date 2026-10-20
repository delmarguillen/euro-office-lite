// `corpus report`: reads what `run` wrote and checks each output with
// poppler-utils and unzip, writing report.tsv, the extracted text and
// summary.md. A crash, a failed exit, an empty output, a PDF without its
// expected text or a script PDF with the bundled fonts only fails the report;
// every other finding is informative.

use crate::known::{Kind, KnownEntry};
use crate::manifest::{self, Entry};
use crate::markdown::clean;
use crate::run::Infra;
use crate::tsv::Table;
use crate::{allfonts, arch, compare, normalize, pdf, sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

pub const REPORT_COLUMNS: [&str; 5] = ["doc", "variant", "step", "check", "value"];
const PACKAGES: [&str; 6] = ["docx", "xlsx", "pptx", "odt", "ods", "odp"];
const TOOLS: [&str; 4] = ["pdftotext", "pdfinfo", "pdffonts", "unzip"];

fn tool_output(tool: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(tool)
        .args(args)
        .output()
        .map_err(|e| format!("{} could not run: {}", tool, e))?;
    if !output.status.success() {
        return Err(format!("{} failed: {}", tool, String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// Every file of an unpacked package, keyed by its path inside the package.
fn unpack(package: &Path, dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    tool_output("unzip", &["-q", "-o", &path_arg(package), "-d", &path_arg(dir)])?;
    let mut parts = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                parts.insert(name, std::fs::read(&path).map_err(|e| e.to_string())?);
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    Ok(parts)
}

fn numbered(parts: &BTreeMap<String, Vec<u8>>, prefix: &str) -> Vec<(u32, String)> {
    parts
        .iter()
        .filter_map(|(name, bytes)| {
            let n = name.strip_prefix(prefix)?.strip_suffix(".xml")?.parse().ok()?;
            Some((n, String::from_utf8_lossy(bytes).into_owned()))
        })
        .collect()
}

fn part_text(parts: &BTreeMap<String, Vec<u8>>, name: &str) -> Option<String> {
    parts.get(name).map(|bytes| String::from_utf8_lossy(bytes).into_owned())
}

// The document text of a package, for the round-trip check.
pub fn package_text(ext: &str, parts: &BTreeMap<String, Vec<u8>>) -> String {
    match ext {
        "docx" => normalize::docx_text(&part_text(parts, "word/document.xml").unwrap_or_default()),
        "pptx" => normalize::pptx_text(&numbered(parts, "ppt/slides/slide")),
        "xlsx" => normalize::xlsx_text(
            part_text(parts, "xl/sharedStrings.xml").as_deref(),
            &numbered(parts, "xl/worksheets/sheet"),
        ),
        _ => normalize::odf_text(&part_text(parts, "content.xml").unwrap_or_default()),
    }
}

fn is_xml(name: &str) -> bool {
    name.ends_with(".xml") || name.ends_with(".rels")
}

// The fonts an OOXML package names that the comparable documents may not,
// and how many theme script entries the check skipped.
pub fn foreign_fonts(parts: &BTreeMap<String, Vec<u8>>) -> (Vec<String>, usize) {
    let mut names = Vec::new();
    let mut scripts_ignored = 0;
    for (_, bytes) in parts.iter().filter(|(name, _)| is_xml(name)) {
        let found = normalize::ooxml_font_names(&String::from_utf8_lossy(bytes));
        scripts_ignored += found.scripts_ignored;
        names.extend(found.names.into_iter().filter(|name| !allfonts::CHECKED_FAMILIES.contains(&name.as_str())));
    }
    names.sort();
    names.dedup();
    (names, scripts_ignored)
}

struct Report<'a> {
    out: &'a Path,
    table: Table,
    failures: Vec<String>,
    // The (doc, variant, step, check) of each failure, in the same order.
    failure_keys: Vec<FailKey>,
    // What the runner cannot test, which is not the document's failure.
    infra: Vec<String>,
}

impl Report<'_> {
    fn new(out: &Path) -> Report<'_> {
        Report { out, table: Table::new(&REPORT_COLUMNS), failures: Vec::new(), failure_keys: Vec::new(), infra: Vec::new() }
    }

    fn push(&mut self, doc: &str, variant: &str, step: &str, check: &str, value: impl Into<String>) {
        self.table.push(vec![doc.into(), variant.into(), step.into(), check.into(), value.into()]);
    }

    fn fail(&mut self, doc: &str, variant: &str, step: &str, check: &str, message: String) {
        self.failures.push(message);
        self.failure_keys.push((doc.into(), variant.into(), step.into(), check.into()));
    }

    fn write_text(&self, entry: &Entry, variant: &str, step: &str, text: &str) -> Result<(), String> {
        let path = self.out.join("text").join(entry.id()).join(variant).join(format!("{}.txt", step));
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {}", path.display(), e))
    }
}

fn class_value(entry: &Entry, foreign: &[String], spoofed: bool) -> String {
    if entry.class != "comparable" {
        entry.class.clone()
    } else if !foreign.is_empty() {
        // The asked-for comparable output names other fonts: it leaves the
        // comparison instead of comparing against whatever each OS has.
        "auto-system-fonts".to_string()
    } else if spoofed {
        "spoofed".to_string()
    } else {
        "comparable".to_string()
    }
}

// An output as `run` records it: a relative path with forward slashes and
// plain names only, so joining it to the results folder stays inside it.
pub fn relative_output(output: &str) -> Result<(), String> {
    let plain = |part: &str| !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', ':']);
    if output.split('/').all(plain) {
        Ok(())
    } else {
        Err(format!("output {:?} is not a plain path inside the results folder", output))
    }
}

// The results may come from another job's artifact, so every row is checked
// against what `run` writes before any of its values reaches a path.
fn check_rows(runs: &Table, entries: &[Entry]) -> Result<(), Infra> {
    for row in &runs.rows {
        let (doc, variant, step) = (runs.get(row, "doc"), runs.get(row, "variant"), runs.get(row, "step"));
        let Some(entry) = entries.iter().find(|e| e.doc == doc) else {
            return Err(Infra(format!("{} is in run.tsv but not in the manifest", doc)));
        };
        // The id names the document's folders under text/ and files/.
        if relative_output(&entry.id()).is_err() {
            return Err(Infra(format!("{}: the document id is not a plain folder name", doc)));
        }
        if !entry.variants.iter().any(|v| v == variant) {
            return Err(Infra(format!("{}: run.tsv names variant {:?}, which run never writes", doc, variant)));
        }
        // The step names a file under text/: one plain name, never a path.
        if step.contains('/') || relative_output(step).is_err() || !crate::run::plan(entry).iter().any(|s| s == step) {
            return Err(Infra(format!("{}: run.tsv names step {:?}, which run never writes", doc, step)));
        }
        let output = runs.get(row, "output");
        if !output.is_empty() {
            relative_output(output).map_err(|e| Infra(format!("{} {} {}: {}", doc, variant, step, e)))?;
        }
    }
    Ok(())
}

fn remove(path: &Path, how: impl Fn(&Path) -> std::io::Result<()>) -> Result<(), Infra> {
    match how(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Infra(format!("cannot remove {}: {}", path.display(), e))),
        _ => Ok(()),
    }
}

// The results carry their own copy of the manifest; only the trusted one
// (the checkout's) is parsed, and the copy must match it byte for byte.
fn trusted_entries(out: &Path, trusted_manifest: &Path) -> Result<Vec<Entry>, Infra> {
    let read = |path: &Path| std::fs::read(path).map_err(|e| Infra(format!("cannot read manifest {}: {}", path.display(), e)));
    let trusted = read(trusted_manifest)?;
    if read(&out.join("manifest.tsv"))? != trusted {
        return Err(Infra(format!(
            "the manifest in {} differs from the trusted manifest {}",
            out.display(),
            trusted_manifest.display()
        )));
    }
    manifest::parse(&String::from_utf8_lossy(&trusted)).map_err(|e| Infra(format!("trusted manifest: {}", e)))
}

// What report needs from outside the results to settle a failure.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options<'a> {
    // known.tsv: flaky crashes and bugs named on purpose.
    pub known: &'a [KnownEntry],
    // The platform id the workflow ran these results on.
    pub platform: Option<&'a str>,
    // The other run of the same build, to tell a crash in one run from one
    // in both.
    pub other: Option<&'a Path>,
}

pub fn report(out: &Path, trusted_manifest: &Path, options: &Options) -> Result<Vec<String>, Infra> {
    // Whatever these names held came with the results, not from this report:
    // gone first, so a refused report leaves nothing to be read as its own.
    for file in ["report.tsv", "summary.md"] {
        remove(&out.join(file), |p| std::fs::remove_file(p))?;
    }
    for dir in ["text", ".unpack"] {
        remove(&out.join(dir), |p| std::fs::remove_dir_all(p))?;
    }
    let runs = Table::read(&out.join("run.tsv")).map_err(Infra)?;
    let env = Table::read(&out.join("env.tsv")).map_err(Infra)?;
    let entries = trusted_entries(out, trusted_manifest)?;
    check_rows(&runs, &entries)?;
    let other = match options.other {
        Some(dir) => Table::read(&dir.join("run.tsv")).map_err(Infra)?,
        None => Table::new(&crate::run::RUN_COLUMNS),
    };
    for tool in TOOLS {
        if Command::new(tool).arg("-v").output().is_err() {
            return Err(Infra(format!("{} is not installed (poppler-utils and unzip are needed)", tool)));
        }
    }
    let env_value = |key: &str| {
        env.rows.iter().find(|r| r.first().map(String::as_str) == Some(key)).and_then(|r| r.get(1)).cloned().unwrap_or_default()
    };
    let spoofed = env_value("fonts.check") != "ok";
    let scratch = out.join(".unpack");
    let runner_families = runner_families(out);
    let mut report = Report::new(out);

    for row in &runs.rows {
        let get = |name: &str| runs.get(row, name).to_string();
        let (doc, variant, step, status) = (get("doc"), get("variant"), get("step"), get("status"));
        let Some(entry) = entries.iter().find(|e| e.doc == doc) else {
            return Err(Infra(format!("{} is in run.tsv but not in the manifest", doc)));
        };
        report.push(&doc, &variant, &step, "status", status.clone());
        if status != "ok" && status != "expected-fail" {
            report.fail(&doc, &variant, &step, "status", format!("{} {} {}: {} {}", doc, variant, step, status, get("message")));
        }
        if step == "open" && status == "ok" {
            report.push(&doc, &variant, &step, "editor.size", get("size"));
            report.push(&doc, &variant, &step, "editor.sha256", get("sha256"));
        }
        let output = get("output");
        if output.is_empty() {
            continue;
        }
        let file = out.join(&output);
        let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let original = file.with_file_name(format!("original.{}", entry.extension()));
        let result = if ext == "pdf" {
            check_pdf(&mut report, entry, &variant, &step, &file, spoofed, &runner_families)
        } else if PACKAGES.contains(&ext.as_str()) {
            check_package(&mut report, entry, &variant, &step, &file, &ext, &original, &scratch, spoofed)
        } else {
            check_text_file(&mut report, entry, &variant, &step, &file, &original)
        };
        if let Err(message) = result {
            report.push(&doc, &variant, &step, "report.error", message.clone());
            report.fail(&doc, &variant, &step, "report.error", format!("{} {} {}: cannot read the output: {}", doc, variant, step, message));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    let scope = Scope {
        known: options.known,
        platform: options.platform.unwrap_or(""),
        family: crate::matrix::family(&env_value("pdf_x2t.version")),
        run: out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    let failures: Vec<(FailKey, String)> = report.failure_keys.drain(..).zip(report.failures.drain(..)).collect();
    let (kept, settled) = settle_run(&runs, &other, &failures, &scope);
    let notes: Vec<String> = settled.iter().map(|r| format!("{} {} {}: {}: {}", r[0], r[1], r[2], r[3], r[4])).collect();
    for row in settled {
        report.table.push(row.to_vec());
    }
    report.table.write(&out.join("report.tsv")).map_err(Infra)?;
    let summary = summary(&report.table, &entries, &env_value, &kept, &notes, out);
    std::fs::write(out.join("summary.md"), summary).map_err(|e| Infra(format!("summary.md: {}", e)))?;
    if !report.infra.is_empty() {
        return Err(Infra(report.infra.join("; ")));
    }
    Ok(kept)
}

// Who the known entries apply to in this report.
#[derive(Clone, Debug, Default)]
pub struct Scope<'a> {
    pub known: &'a [KnownEntry],
    // The platform id the workflow ran these results on, and its x2t family.
    pub platform: &'a str,
    pub family: String,
    // This results folder's name (run1, run2), to say where a crash was.
    pub run: String,
}

type StepKey = (String, String, String);

fn step_key(table: &Table, row: &[String]) -> StepKey {
    (table.get(row, "doc").to_string(), table.get(row, "variant").to_string(), table.get(row, "step").to_string())
}

// The steps of this run that crashed and may pass as flaky, with what
// happened. A crash passes only if known.tsv declares it as flaky for this
// document, variant, step and platform, and the other run of the same build
// ran that step to its end (ok or expected-fail): a crash in both runs is a
// hard failure, and so is any crash nobody declared or one the other run
// cannot vouch for (skipped, exit, empty). The steps a flaky open skipped go
// with it.
pub fn flaky(runs: &Table, other: &Table, scope: &Scope) -> BTreeMap<StepKey, String> {
    let other_status: BTreeMap<StepKey, String> =
        other.rows.iter().map(|row| (step_key(other, row), other.get(row, "status").to_string())).collect();
    let mut out = BTreeMap::new();
    for row in runs.rows.iter().filter(|row| runs.get(row, "status") == "signal") {
        let step = step_key(runs, row);
        let declared = scope.known.iter().any(|e| {
            e.kind == Kind::Flaky && e.matches(&step.0, &step.1, &step.2, "status") && e.covers(scope.platform, &scope.family)
        });
        let Some(other) = other_status.get(&step).filter(|s| *s == "ok" || *s == "expected-fail") else { continue };
        if declared {
            let what = format!("signal {} on {} in {} (the other run: {})", runs.get(row, "signal"), scope.platform, scope.run, other);
            out.insert(step, what);
        }
    }
    let flaky_opens: Vec<(String, String)> = out.keys().filter(|k| k.2 == "open").map(|k| (k.0.clone(), k.1.clone())).collect();
    for row in runs.rows.iter().filter(|row| runs.get(row, "status") == "skipped") {
        let step = step_key(runs, row);
        if flaky_opens.contains(&(step.0.clone(), step.1.clone())) {
            out.insert(step, "skipped after the flaky open".to_string());
        }
    }
    out
}

// (doc, variant, step, check) of one hard failure.
pub type FailKey = (String, String, String, String);

// The steps that crashed in this run and in the other run of the same build.
pub fn crashed_in_both(runs: &Table, other: &Table) -> BTreeSet<StepKey> {
    let crashed = |table: &Table| -> BTreeSet<StepKey> {
        table.rows.iter().filter(|row| table.get(row, "status") == "signal").map(|row| step_key(table, row)).collect()
    };
    crashed(runs).intersection(&crashed(other)).cloned().collect()
}

// What report settles for one run: its flaky steps against the other run,
// then every failure against them, the crashes of both runs and known.tsv.
pub fn settle_run(runs: &Table, other: &Table, failures: &[(FailKey, String)], scope: &Scope) -> (Vec<String>, Vec<[String; 5]>) {
    settle(failures, &flaky(runs, other, scope), &crashed_in_both(runs, other), scope)
}

// Splits the hard failures into the ones that stay and the report rows that
// record the others: the crash of a flaky step (or the skip a flaky open
// caused), or a failure a bug entry names exactly (document and check, no
// wildcard) for this platform. Nothing else leaves: a crash in both runs, a
// check the partial output of a flaky crash fails, a script PDF without its
// font, a lost TXT or a crash nobody declared stays a failure whatever env,
// x2t or flaky entry matches it.
pub fn settle(
    failures: &[(FailKey, String)],
    flaky: &BTreeMap<StepKey, String>,
    crashed_in_both: &BTreeSet<StepKey>,
    scope: &Scope,
) -> (Vec<String>, Vec<[String; 5]>) {
    let mut kept = Vec::new();
    let mut rows: Vec<[String; 5]> = Vec::new();
    for ((doc, variant, step, check), message) in failures {
        let at = (doc.clone(), variant.clone(), step.clone());
        if check == "status" && crashed_in_both.contains(&at) {
            kept.push(message.clone());
            continue;
        }
        if let Some(what) = flaky.get(&at).filter(|_| check == "status") {
            let row = [doc.clone(), variant.clone(), step.clone(), compare::FLAKY.to_string(), what.clone()];
            if !rows.contains(&row) {
                rows.push(row);
            }
            continue;
        }
        let bug = scope.known.iter().find(|e| {
            e.covers_failure(doc, check) && e.matches(doc, variant, step, check) && e.covers(scope.platform, &scope.family)
        });
        match bug {
            Some(entry) => rows.push([
                doc.clone(),
                variant.clone(),
                step.clone(),
                compare::KNOWN_BUG.to_string(),
                format!("{}: {}", entry.describe(), message),
            ]),
            None => kept.push(message.clone()),
        }
    }
    // Flaky steps whose crash left no failure of their own (none today) are
    // still recorded, so compare and matrix see every one.
    for ((doc, variant, step), what) in flaky {
        let row = [doc.clone(), variant.clone(), step.clone(), compare::FLAKY.to_string(), what.clone()];
        if !rows.contains(&row) {
            rows.push(row);
        }
    }
    (kept, rows)
}

// The font families of the runner's font cache, which run copies next to the
// results.
fn runner_families(out: &Path) -> Vec<String> {
    std::fs::read_to_string(out.join("fonts").join("AllFonts.js"))
        .ok()
        .and_then(|text| allfonts::parse(&text).ok())
        .map(|list| list.infos.into_iter().map(|info| info.name).collect())
        .unwrap_or_default()
}

fn check_pdf(
    report: &mut Report,
    entry: &Entry,
    variant: &str,
    step: &str,
    file: &Path,
    spoofed: bool,
    runner_families: &[String],
) -> Result<(), String> {
    let doc = entry.doc.as_str();
    let info = tool_output("pdfinfo", &[&path_arg(file)])?;
    report.push(doc, variant, step, "pages", pdf::pages(&info).map(|p| p.to_string()).unwrap_or_default());
    let text = normalize::tidy(&tool_output("pdftotext", &["-enc", "UTF-8", &path_arg(file), "-"])?);
    report.write_text(entry, variant, step, &text)?;
    report.push(doc, variant, step, "text.sha256", sha256::hex(text.as_bytes()));
    pdf_expectation(report, entry, variant, step, &text);
    let fonts = pdf::fonts(&tool_output("pdffonts", &[&path_arg(file)])?);
    let listed: Vec<String> = fonts.iter().map(pdf::Font::describe).collect();
    report.push(doc, variant, step, "fonts", listed.join(", "));
    let not_embedded: Vec<&str> = fonts.iter().filter(|f| f.emb != "yes").map(|f| f.name.as_str()).collect();
    report.push(doc, variant, step, "fonts.embedded", if not_embedded.is_empty() { "all".to_string() } else { not_embedded.join(", ") });
    if entry.class == "comparable" {
        let mut foreign: Vec<&str> = fonts.iter().filter(|f| !pdf::allowed(&f.name)).map(|f| f.name.as_str()).collect();
        foreign.dedup();
        report.push(doc, variant, step, "fonts.allowed", if foreign.is_empty() { "ok".to_string() } else { foreign.join(", ") });
    }
    script_font(report, entry, variant, step, &fonts, runner_families);
    report.push(doc, variant, step, "class", class_value(entry, &[], spoofed));
    Ok(())
}

// A script the bundled fonts cannot draw (Arabic, Hebrew) still leaves its
// Latin phrase in the PDF, so the expected text alone misses a PDF drawn with
// Liberation and Carlito only. pdftotext gives Arabic in visual order with
// its ligatures, so the fonts embedded are checked instead of the text: one
// of the declared families, embedded. Families compare by pdf::family.
fn script_font(report: &mut Report, entry: &Entry, variant: &str, step: &str, fonts: &[pdf::Font], runner_families: &[String]) {
    if entry.script_fonts.is_empty() {
        return;
    }
    let doc = entry.doc.as_str();
    let declared: Vec<String> = entry.script_fonts.iter().map(|f| pdf::family(f)).collect();
    let on_runner = runner_families.iter().any(|f| declared.contains(&pdf::family(f)));
    if !on_runner {
        report.infra.push(format!(
            "{} {} {}: the runner has none of the fonts for its script ({})",
            doc,
            variant,
            step,
            entry.script_fonts.join(", ")
        ));
        report.push(doc, variant, step, "fonts.script", "no-script-font-on-runner");
        return;
    }
    let is_declared = |f: &&pdf::Font| declared.contains(&pdf::family(&f.name));
    let names = |list: Vec<&pdf::Font>| list.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ");
    let value = if fonts.iter().filter(is_declared).any(|f| f.emb == "yes") {
        "ok".to_string()
    } else {
        let declared_fonts: Vec<&pdf::Font> = fonts.iter().filter(is_declared).collect();
        let others: Vec<&pdf::Font> = fonts.iter().filter(|f| !pdf::allowed(&f.name)).collect();
        let value = if !declared_fonts.is_empty() {
            format!("not-embedded:{}", names(declared_fonts))
        } else if !others.is_empty() {
            format!("other-font:{}", names(others))
        } else {
            "bundled-only".to_string()
        };
        let embedded: Vec<&pdf::Font> = fonts.iter().filter(|f| f.emb == "yes").collect();
        let embedded = if embedded.is_empty() { "nothing".to_string() } else { names(embedded) };
        // Naming both sides tells a runner with another valid font apart from
        // a broken export: the first only needs that family in the manifest.
        let message = format!(
            "{} {} {}: PDF without an embedded font for its script ({}): embeds {}, expected one of {}",
            doc,
            variant,
            step,
            value,
            embedded,
            entry.script_fonts.join("|")
        );
        report.fail(doc, variant, step, "fonts.script", message);
        value
    };
    report.push(doc, variant, step, "fonts.script", value);
}

// A PDF that misses any expected phrase fails the report: x2t can exit 0 and
// still write a PDF without its text when the font cache is wrong.
fn pdf_expectation(report: &mut Report, entry: &Entry, variant: &str, step: &str, text: &str) {
    if entry.expect.is_empty() {
        return;
    }
    let doc = entry.doc.as_str();
    let missing = pdf::missing_phrases(text, &entry.expect);
    if missing.is_empty() {
        report.push(doc, variant, step, "expected", "ok");
    } else {
        let value = format!("missing:{}", missing.join("|"));
        report.fail(doc, variant, step, "expected", format!("{} {} {}: PDF without its text ({})", doc, variant, step, value));
        report.push(doc, variant, step, "expected", value);
    }
}

#[allow(clippy::too_many_arguments)]
fn check_package(
    report: &mut Report,
    entry: &Entry,
    variant: &str,
    step: &str,
    file: &Path,
    ext: &str,
    original: &Path,
    scratch: &Path,
    spoofed: bool,
) -> Result<(), String> {
    let doc = entry.doc.as_str();
    let parts = unpack(file, &scratch.join("out"))?;
    for (name, bytes) in &parts {
        let hash = if is_xml(name) {
            sha256::hex(normalize::normalize_part(name, &String::from_utf8_lossy(bytes)).as_bytes())
        } else {
            sha256::hex(bytes)
        };
        report.push(doc, variant, step, &format!("part:{}", name), hash);
    }
    let text = package_text(ext, &parts);
    report.write_text(entry, variant, step, &text)?;
    report.push(doc, variant, step, "text.sha256", sha256::hex(text.as_bytes()));
    if !entry.expect.is_empty() {
        let missing = pdf::missing_phrases(&text, &entry.expect);
        report.push(doc, variant, step, "expected", if missing.is_empty() { "ok".to_string() } else { format!("missing:{}", missing.join("|")) });
    }
    // The round trip compares the document text with the original's: x2t
    // rewrites styles and structure, so the parts themselves always differ.
    if step == "save" && original.is_file() && std::fs::metadata(original).map(|m| m.len()).unwrap_or(0) > 0 {
        let original_parts = unpack(original, &scratch.join("original"))?;
        let same = package_text(ext, &original_parts) == text;
        report.push(doc, variant, step, "roundtrip.text", if same { "equal" } else { "diff" });
    }
    // Kept apart from the parts (#41): what x2t writes as the application.
    let (application, version) = match part_text(&parts, "docProps/app.xml") {
        Some(app) => (normalize::element_text(&app, "Application"), normalize::element_text(&app, "AppVersion")),
        None => (part_text(&parts, "meta.xml").and_then(|m| normalize::element_text(&m, "meta:generator")), None),
    };
    report.push(doc, variant, step, "app.application", application.unwrap_or_default());
    report.push(doc, variant, step, "app.version", version.unwrap_or_default());
    let (foreign, scripts_ignored) = if ext == "xlsx" || ext == "pptx" { foreign_fonts(&parts) } else { (Vec::new(), 0) };
    if ext == "xlsx" || ext == "pptx" {
        report.push(doc, variant, step, "fonts.named", if foreign.is_empty() { "ok".to_string() } else { foreign.join(", ") });
        report.push(doc, variant, step, "fonts.script_ignored", scripts_ignored.to_string());
    }
    report.push(doc, variant, step, "class", class_value(entry, &foreign, spoofed));
    Ok(())
}

// The first differing byte, or equal.
pub fn byte_diff(a: &[u8], b: &[u8]) -> String {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => format!("diff:{}", i),
        None if a.len() == b.len() => "equal".to_string(),
        None => format!("diff:{}", a.len().min(b.len())),
    }
}

fn check_text_file(report: &mut Report, entry: &Entry, variant: &str, step: &str, file: &Path, original: &Path) -> Result<(), String> {
    let doc = entry.doc.as_str();
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    report.push(doc, variant, step, "bytes.sha256", sha256::hex(&bytes));
    let same_format = file.extension() == original.extension();
    if step == "save" && same_format {
        if let Ok(original_bytes) = std::fs::read(original) {
            let diff = byte_diff(&original_bytes, &bytes);
            // A TXT has nothing a save may rewrite: the app puts its encoding,
            // BOM and line breaks back, so any other byte is lost text.
            let txt = file.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("txt"));
            if txt && diff != "equal" {
                let message = format!("{} {} {}: the TXT changed in an open and save ({})", doc, variant, step, diff);
                report.fail(doc, variant, step, "roundtrip.bytes", message);
            }
            report.push(doc, variant, step, "roundtrip.bytes", diff);
        }
    }
    report.push(doc, variant, step, "class", entry.class.clone());
    Ok(())
}

// The cell of one step in the summary table.
fn cell(checks: &BTreeMap<(&str, &str), &str>, step: &str) -> String {
    let Some(status) = checks.get(&(step, "status")) else { return String::new() };
    if checks.contains_key(&(step, compare::FLAKY)) {
        return "flaky".to_string();
    }
    if checks.contains_key(&(step, compare::KNOWN_BUG)) {
        return "known bug".to_string();
    }
    match *status {
        "ok" => {}
        "expected-fail" => return "expected fail".to_string(),
        other => return format!("FAIL {}", other),
    }
    if checks.get(&(step, "expected")).is_some_and(|v| v.starts_with("missing:")) && step == "pdf" {
        return "FAIL text missing".to_string();
    }
    if checks.get(&(step, "fonts.script")).is_some_and(|v| *v != "ok" && *v != "no-script-font-on-runner") {
        return "FAIL script font".to_string();
    }
    let differs = ["roundtrip.text", "roundtrip.bytes"]
        .iter()
        .any(|c| checks.get(&(step, *c)).is_some_and(|v| *v != "equal"));
    let mut text = if differs { "diff".to_string() } else { "ok".to_string() };
    if let Some(class) = checks.get(&(step, "class")).filter(|c| **c == "auto-system-fonts" || **c == "spoofed") {
        text.push_str(&format!(" ({})", class));
    }
    text
}

pub fn architecture_line(env: &dyn Fn(&str) -> String) -> String {
    let host = env("host.arch");
    let (sidecar, pdf_x2t) = (env("sidecar.arch"), env("pdf_x2t.arch"));
    let emulated = arch::emulated(&sidecar, &host) || arch::emulated(&pdf_x2t, &host);
    format!(
        "- Architecture: host {}, sidecar x2t {}, PDF x2t {}{}\n",
        clean(&host),
        clean(&sidecar),
        clean(&pdf_x2t),
        if emulated { " (emulated)" } else { "" }
    )
}

// What summary.md shows from env.tsv, package.txt, run.tsv and x2t's own
// messages goes through clean(): the file ends up in the run's summary.
fn summary(table: &Table, entries: &[Entry], env: &dyn Fn(&str) -> String, failures: &[String], notes: &[String], out: &Path) -> String {
    let mut s = String::from("## Conversion corpus\n\n");
    let package = std::fs::read_to_string(out.join("..").join("package.txt")).unwrap_or_default();
    if !package.trim().is_empty() {
        s.push_str(&format!("- Package: {}\n", clean(&package.split_whitespace().collect::<Vec<_>>().join(" "))));
    }
    let shown = |key: &str| clean(&env(key));
    s.push_str(&format!("- Sidecar x2t: {} ({})\n", shown("sidecar.version"), shown("sidecar.path")));
    s.push_str(&format!("- PDF x2t: {} ({}), same file: {}\n", shown("pdf_x2t.version"), shown("pdf_x2t.path"), shown("x2t_same_file")));
    s.push_str(&architecture_line(env));
    s.push_str(&format!(
        "- Font cache: {} fonts, {} bundled, {} system; Liberation and Carlito: {}\n",
        shown("fonts.total"),
        shown("fonts.bundled"),
        shown("fonts.system"),
        shown("fonts.check")
    ));
    s.push_str("- Editor.bin comes from x2t, not from the editor: save and export here never go through sdkjs.\n");
    s.push_str(&format!("- Hard failures: {}\n\n", failures.len()));
    for failure in failures {
        s.push_str(&format!("  - {}\n", clean(failure)));
    }
    if !failures.is_empty() {
        s.push('\n');
    }
    // Failures known.tsv settles: a flaky crash in one run, a bug named on
    // purpose. They do not fail the report, and they are always shown.
    if !notes.is_empty() {
        s.push_str(&format!("- Settled by known.tsv: {}\n\n", notes.len()));
        for note in notes {
            s.push_str(&format!("  - {}\n", clean(note)));
        }
        s.push('\n');
    }
    s.push_str("| Document | Variant | Class | Open | Save | PDF | Save as |\n|---|---|---|---|---|---|---|\n");
    let mut grouped: BTreeMap<(usize, String), BTreeMap<(&str, &str), &str>> = BTreeMap::new();
    for row in &table.rows {
        let position = entries.iter().position(|e| e.doc == row[0]).unwrap_or(usize::MAX);
        grouped
            .entry((position, row[1].clone()))
            .or_default()
            .insert((row[2].as_str(), row[3].as_str()), row[4].as_str());
    }
    for ((position, variant), checks) in &grouped {
        let Some(entry) = entries.get(*position) else { continue };
        let save_as: Vec<String> = entry
            .save_as
            .iter()
            .map(|ext| format!("{} {}", ext, clean(&cell(checks, &format!("save_as.{}", ext)))))
            .collect();
        // The document, variant and class are the trusted manifest's own.
        s.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} |\n",
            entry.doc,
            variant,
            entry.class,
            clean(&cell(checks, "open")),
            clean(&cell(checks, "save")),
            clean(&cell(checks, "pdf")),
            save_as.join(", ")
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(class: &str) -> Entry {
        let text = format!(
            "file\tclass\tsteps\texpect\tvariants\tsave_as\tnote\nsheet.ods\t{}\topen,save,pdf\tHello\t\txlsx\t\n",
            class
        );
        manifest::parse(&text).unwrap().remove(0)
    }

    #[test]
    fn an_xlsx_naming_other_fonts_leaves_the_comparison() {
        let mut parts = BTreeMap::new();
        parts.insert("xl/styles.xml".to_string(), b"<styleSheet><fonts><font><name val=\"Liberation Sans\"/></font><font><name val=\"Calibri\"/></font></fonts></styleSheet>".to_vec());
        parts.insert("xl/theme/theme1.xml".to_string(), b"<a:theme><a:latin typeface=\"Arial\"/><a:font script=\"Jpan\" typeface=\"MS Gothic\"/></a:theme>".to_vec());
        parts.insert("xl/media/image1.png".to_string(), b"typeface=\"Nope\"".to_vec());
        let (foreign, scripts_ignored) = foreign_fonts(&parts);
        assert_eq!(foreign, vec!["Arial", "Calibri"]);
        assert_eq!(scripts_ignored, 1);
        assert_eq!(class_value(&entry("comparable"), &foreign, false), "auto-system-fonts");
        assert_eq!(class_value(&entry("comparable"), &[], false), "comparable");
        assert_eq!(class_value(&entry("comparable"), &[], true), "spoofed");
        assert_eq!(class_value(&entry("system-fonts"), &foreign, true), "system-fonts");
    }

    #[test]
    fn an_xlsx_with_only_bundled_fonts_stays_comparable() {
        let mut parts = BTreeMap::new();
        parts.insert("xl/styles.xml".to_string(), b"<styleSheet><fonts><font><name val=\"Liberation Sans\"/></font><font><name val=\"Carlito\"/></font></fonts></styleSheet>".to_vec());
        parts.insert("xl/theme/theme1.xml".to_string(), b"<a:theme><a:latin typeface=\"Liberation Sans\"/><a:ea typeface=\"\"/></a:theme>".to_vec());
        assert_eq!(foreign_fonts(&parts), (Vec::new(), 0));
    }

    #[test]
    fn a_pdf_missing_its_expected_text_is_a_hard_failure() {
        let out = std::env::temp_dir();
        let e = entry("comparable");
        let mut report = Report::new(&out);
        pdf_expectation(&mut report, &e, "plain", "pdf", "Hello\nworld\n");
        assert!(report.failures.is_empty());
        pdf_expectation(&mut report, &e, "plain", "pdf", "\x0c\n");
        assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
        assert!(report.failures[0].contains("missing:Hello"));
        let values: Vec<&str> = report.table.rows.iter().map(|r| r[4].as_str()).collect();
        assert_eq!(values, vec!["ok", "missing:Hello"]);
    }

    fn font(name: &str) -> pdf::Font {
        pdf::Font { name: name.into(), kind: "CID TrueType".into(), emb: "yes".into(), sub: "yes".into(), uni: "yes".into() }
    }

    fn font_emb(name: &str, emb: &str) -> pdf::Font {
        pdf::Font { emb: emb.into(), ..font(name) }
    }

    // The fonts.script value, the failures and the infrastructure problems of
    // one Arabic PDF with these fonts on a runner with these families.
    fn script_check(fonts: &[pdf::Font], runner: &[&str]) -> (String, Vec<String>, Vec<String>) {
        let out = std::env::temp_dir();
        let text = "file	class	steps	expect	variants	save_as	note	script_fonts
arabic-rtl.docx	system-fonts	open,pdf	Arabic				DejaVu Sans|Tahoma
";
        let e = manifest::parse(text).unwrap().remove(0);
        let runner: Vec<String> = runner.iter().map(|s| s.to_string()).collect();
        let mut report = Report::new(&out);
        script_font(&mut report, &e, "plain", "pdf", fonts, &runner);
        assert_eq!(report.table.rows.len(), 1);
        assert_eq!(report.table.rows[0][3], "fonts.script");
        (report.table.rows[0][4].clone(), report.failures, report.infra)
    }

    // The fonts the PDF of run 39278051145 embedded for arabic-rtl.docx and
    // hebrew.docx, on the families each runner's font cache had.
    #[test]
    fn the_script_check_on_the_fonts_the_runners_embedded() {
        let linux = ["Carlito", "DejaVu Sans", "Lato", "Liberation Serif"];
        let (value, failures, infra) = script_check(&[font("Carlito"), font("Liberation Serif")], &linux);
        assert_eq!(value, "bundled-only");
        assert_eq!(
            failures,
            vec!["arabic-rtl.docx plain pdf: PDF without an embedded font for its script (bundled-only): embeds Carlito, Liberation Serif, expected one of DejaVu Sans|Tahoma"]
        );
        assert!(infra.is_empty());
        assert_eq!(script_check(&[font("Liberation Serif")], &linux).0, "bundled-only");

        let windows_mac = ["Arial", "Liberation Serif", "Tahoma", "Times New Roman"];
        let (value, failures, infra) = script_check(&[font("Liberation Serif"), font("Tahoma")], &windows_mac);
        assert_eq!((value.as_str(), failures.len(), infra.len()), ("ok", 0, 0));
    }

    #[test]
    fn only_an_embedded_declared_family_passes_the_script_check() {
        let runner = ["DejaVu Sans", "Liberation Serif", "Tahoma"];
        // A subset of DejaVu Sans, as pdffonts names it, and Tahoma bold.
        assert_eq!(script_check(&[font("BAAAAA+DejaVuSans"), font("Liberation Serif")], &runner).0, "ok");
        assert_eq!(script_check(&[font("Tahoma,Bold")], &runner).0, "ok");
        assert_eq!(script_check(&[font("DejaVuSans-Bold")], &runner).0, "ok");
        assert_eq!(script_check(&[font("CAAAAA+DejaVu#20Sans,Bold")], &runner).0, "ok");

        // A font unrelated to the script is not enough.
        let (value, failures, _) = script_check(&[font("Arial"), font("Liberation Serif")], &runner);
        assert_eq!(value, "other-font:Arial");
        assert_eq!(failures.len(), 1);
        assert!(failures[0].ends_with("embeds Arial, Liberation Serif, expected one of DejaVu Sans|Tahoma"), "{}", failures[0]);
        // Nor a declared family that is only referenced, not embedded.
        let (value, failures, _) = script_check(&[font_emb("DejaVu Sans", "no"), font("Arial")], &runner);
        assert_eq!(value, "not-embedded:DejaVu Sans");
        assert_eq!(failures.len(), 1);
        assert!(failures[0].ends_with("embeds Arial, expected one of DejaVu Sans|Tahoma"), "{}", failures[0]);
        let (_, failures, _) = script_check(&[font_emb("Tahoma", "no")], &runner);
        assert!(failures[0].ends_with("embeds nothing, expected one of DejaVu Sans|Tahoma"), "{}", failures[0]);
    }

    // Families that share a first part stay apart: the manifest never
    // declares one with a '-' or ',', so the cut style suffix cannot join them.
    #[test]
    fn a_family_sharing_the_first_part_does_not_pass_the_script_check() {
        let out = std::env::temp_dir();
        let text = "file\tclass\tsteps\texpect\tvariants\tsave_as\tnote\tscript_fonts\n\
a.docx\tsystem-fonts\topen,pdf\tHi\t\t\t\tExampleArabic\n";
        let e = manifest::parse(text).unwrap().remove(0);
        let runner = vec!["ExampleArabic".to_string(), "ExampleLatin".to_string()];
        let mut report = Report::new(&out);
        script_font(&mut report, &e, "plain", "pdf", &[font("ABCDEF+ExampleLatin-Bold")], &runner);
        assert_eq!(report.table.rows[0][4], "other-font:ABCDEF+ExampleLatin-Bold");
        assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    }

    // A runner without any of the listed fonts cannot draw the script: that
    // is the runner's problem, not the document's, whatever the PDF holds.
    #[test]
    fn a_runner_without_the_declared_families_is_infrastructure() {
        let (value, failures, infra) = script_check(&[font("Liberation Serif")], &["Carlito", "Liberation Serif"]);
        assert_eq!(value, "no-script-font-on-runner");
        assert!(failures.is_empty());
        assert_eq!(infra.len(), 1);
        assert!(infra[0].contains("DejaVu Sans, Tahoma"), "{}", infra[0]);
        assert_eq!(script_check(&[font("Arial")], &[]).0, "no-script-font-on-runner");
    }

    #[test]
    fn documents_without_script_fonts_get_no_script_row() {
        let out = std::env::temp_dir();
        let mut report = Report::new(&out);
        script_font(&mut report, &entry("system-fonts"), "plain", "pdf", &[font("Liberation Serif")], &["Tahoma".to_string()]);
        assert!(report.table.rows.is_empty() && report.failures.is_empty() && report.infra.is_empty());
    }

    #[test]
    fn the_runner_font_families_come_from_its_font_cache_copy() {
        let out = std::env::temp_dir().join(format!("corpus-report-families-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        assert!(runner_families(&out).is_empty());
        std::fs::create_dir_all(out.join("fonts")).unwrap();
        std::fs::write(
            out.join("fonts/AllFonts.js"),
            "window[\"__fonts_files\"] = [];\nwindow[\"__fonts_infos\"] = [[\"DejaVu Sans\",0,0,-1,0,-1,0,-1,0],[\"Carlito\",0,0,-1,0,-1,0,-1,0]];\n",
        )
        .unwrap();
        let families = runner_families(&out);
        let _ = std::fs::remove_dir_all(&out);
        assert_eq!(families, vec!["DejaVu Sans", "Carlito"]);
    }

    fn run_table(rows: &[(&str, &str, &str, &str, &str)]) -> Table {
        let mut table = Table::new(&crate::run::RUN_COLUMNS);
        for (doc, variant, step, status, signal) in rows {
            let mut cells = vec![String::new(); crate::run::RUN_COLUMNS.len()];
            cells[0] = doc.to_string();
            cells[1] = variant.to_string();
            cells[2] = step.to_string();
            cells[3] = status.to_string();
            cells[5] = signal.to_string();
            table.push(cells);
        }
        table
    }

    fn known_entries(rows: &str) -> Vec<KnownEntry> {
        crate::known::parse(&format!("{}\n{}", crate::known::COLUMNS.join("\t"), rows)).unwrap()
    }

    fn scope(known: &[KnownEntry]) -> Scope<'_> {
        Scope { known, platform: "linux-arm64", family: "9.4".into(), run: "run1".into() }
    }

    const ARABIC_FLAKY: &str = "arabic-rtl.docx\tplain\tpdf\t*\tlinux-arm64\tflaky\tcrash being measured\t\n";

    // The three cases of the rule: a declared crash in one run passes as
    // flaky; the same crash undeclared, or declared but in both runs, fails.
    #[test]
    fn a_crash_is_flaky_only_when_declared_and_in_one_run() {
        let crashed = run_table(&[("arabic-rtl.docx", "plain", "open", "ok", ""), ("arabic-rtl.docx", "plain", "pdf", "signal", "6")]);
        let fine = run_table(&[("arabic-rtl.docx", "plain", "open", "ok", ""), ("arabic-rtl.docx", "plain", "pdf", "ok", "")]);
        let declared = known_entries(ARABIC_FLAKY);
        let key = ("arabic-rtl.docx".to_string(), "plain".to_string(), "pdf".to_string());

        let flaky_steps = flaky(&crashed, &fine, &scope(&declared));
        assert_eq!(flaky_steps.get(&key).map(String::as_str), Some("signal 6 on linux-arm64 in run1 (the other run: ok)"));
        assert_eq!(flaky_steps.len(), 1);

        assert!(flaky(&crashed, &fine, &scope(&[])).is_empty(), "undeclared");
        assert!(flaky(&crashed, &crashed, &scope(&declared)).is_empty(), "in both runs");
    }

    #[test]
    fn a_flaky_entry_is_bound_to_its_document_step_and_platform() {
        let crashed = run_table(&[("arabic-rtl.docx", "plain", "pdf", "signal", "11")]);
        let fine = run_table(&[("arabic-rtl.docx", "plain", "pdf", "ok", "")]);
        let declared = known_entries(ARABIC_FLAKY);
        let elsewhere = Scope { platform: "linux-x64", family: "8.2".into(), ..scope(&declared) };
        assert!(flaky(&crashed, &fine, &elsewhere).is_empty(), "another platform");
        let other_step = known_entries("arabic-rtl.docx\tplain\tsave\t*\tlinux-arm64\tflaky\tr\t\n");
        assert!(flaky(&crashed, &fine, &scope(&other_step)).is_empty(), "another step");
        let not_flaky = known_entries("arabic-rtl.docx\tplain\tpdf\tstatus\tlinux-arm64\tbug\tr\t#9\n");
        assert!(flaky(&crashed, &fine, &scope(&not_flaky)).is_empty(), "a bug entry is not a flaky one");
        // Only a crash: an exit code in one run is never flaky.
        let exited = run_table(&[("arabic-rtl.docx", "plain", "pdf", "exit", "")]);
        assert!(flaky(&exited, &fine, &scope(&declared)).is_empty(), "an exit");
        // The other run must have run the step.
        assert!(flaky(&crashed, &run_table(&[]), &scope(&declared)).is_empty(), "no other run");
        let family = known_entries("arabic-rtl.docx\tplain\tpdf\t*\t9.4\tflaky\tr\t\n");
        assert_eq!(flaky(&crashed, &fine, &scope(&family)).len(), 1, "by family");
    }

    // A crashed open skips the rest of the document in that run: those
    // skipped steps are part of the same flaky crash.
    #[test]
    fn the_steps_a_flaky_open_skipped_are_flaky_too() {
        let crashed = run_table(&[
            ("arabic-rtl.docx", "plain", "open", "signal", "6"),
            ("arabic-rtl.docx", "plain", "save", "skipped", ""),
            ("arabic-rtl.docx", "plain", "pdf", "skipped", ""),
            ("hebrew.docx", "plain", "save", "skipped", ""),
        ]);
        let fine = run_table(&[("arabic-rtl.docx", "plain", "open", "ok", "")]);
        let declared = known_entries("arabic-rtl.docx\tplain\topen\t*\tall\tflaky\tr\t\n");
        let steps: Vec<String> = flaky(&crashed, &fine, &scope(&declared)).keys().map(|k| format!("{} {}", k.0, k.2)).collect();
        assert_eq!(steps, vec!["arabic-rtl.docx open", "arabic-rtl.docx pdf", "arabic-rtl.docx save"]);
    }

    fn failure(doc: &str, step: &str, check: &str, message: &str) -> (FailKey, String) {
        ((doc.into(), "plain".into(), step.into(), check.into()), message.into())
    }

    // A script PDF without its font is a hard failure that no env, x2t or
    // flaky entry, nor a wildcard bug entry, takes away; only a bug entry that
    // names the document and fonts.script does, and the report says so.
    #[test]
    fn a_script_font_failure_leaves_only_for_a_bug_entry_that_names_it() {
        let failures = [failure("arabic-rtl.docx", "pdf", "fonts.script", "arabic-rtl.docx plain pdf: PDF without an embedded font")];
        for broad in [
            "arabic-rtl.docx\tplain\tpdf\tfonts.script\tall\tenv\tr\t\n",
            "arabic-rtl.docx\tplain\tpdf\tfonts.script\tall\tx2t\tr\t\n",
            "arabic-rtl.docx\tplain\tpdf\t*\tall\tflaky\tr\t\n",
            "arabic-rtl.docx\t*\t*\tfonts*\tall\tbug\tr\t#7\n",
            "*\tplain\tpdf\tfonts.script\tall\tbug\tr\t#7\n",
            "arabic-rtl.docx\tplain\tpdf\tfonts.script\tlinux-x64\tbug\tr\t#7\n",
        ] {
            let known = known_entries(broad);
            let (kept, rows) = settle(&failures, &BTreeMap::new(), &BTreeSet::new(), &scope(&known));
            assert_eq!((kept.len(), rows.len()), (1, 0), "{}", broad);
        }
        let known = known_entries("arabic-rtl.docx\tplain\tpdf\tfonts.script\tlinux-arm64\tbug\tDejaVu missing on the image\t#7\n");
        let (kept, rows) = settle(&failures, &BTreeMap::new(), &BTreeSet::new(), &scope(&known));
        assert!(kept.is_empty());
        assert_eq!(rows[0][3], "known-bug");
        assert!(rows[0][4].starts_with("known.tsv line 2 (bug, #7): DejaVu missing on the image: arabic-rtl.docx plain pdf"), "{}", rows[0][4]);
    }

    // The failures of a flaky step leave, recorded once; any other stays.
    #[test]
    fn the_failures_of_a_flaky_step_leave_and_the_rest_stay() {
        let failures = [
            failure("arabic-rtl.docx", "pdf", "status", "arabic-rtl.docx plain pdf: signal"),
            failure("hebrew.docx", "pdf", "status", "hebrew.docx plain pdf: signal"),
        ];
        let mut flaky_steps = BTreeMap::new();
        flaky_steps.insert(("arabic-rtl.docx".to_string(), "plain".to_string(), "pdf".to_string()), "signal 6 on linux-arm64 in run1 (the other run: ok)".to_string());
        let (kept, rows) = settle(&failures, &flaky_steps, &BTreeSet::new(), &scope(&[]));
        assert_eq!(kept, vec!["hebrew.docx plain pdf: signal"]);
        assert_eq!(rows, vec![[
            "arabic-rtl.docx".to_string(), "plain".into(), "pdf".into(), "flaky".into(), "signal 6 on linux-arm64 in run1 (the other run: ok)".into()
        ]]);
    }

    // The other run vouches for a crash only if it ran that step to its end:
    // here neither run made the PDF, so the crash of run1 stays a failure.
    #[test]
    fn a_crash_is_flaky_only_if_the_other_run_finished_the_step() {
        let run1 = run_table(&[("arabic-rtl.docx", "plain", "open", "ok", ""), ("arabic-rtl.docx", "plain", "pdf", "signal", "6")]);
        let run2 = run_table(&[("arabic-rtl.docx", "plain", "open", "signal", "11"), ("arabic-rtl.docx", "plain", "pdf", "skipped", "")]);
        let declared = known_entries(
            "arabic-rtl.docx\tplain\topen\t*\tlinux-arm64\tflaky\tr\t\narabic-rtl.docx\tplain\tpdf\t*\tlinux-arm64\tflaky\tr\t\n",
        );
        assert!(flaky(&run1, &run2, &scope(&declared)).is_empty(), "the other run skipped the PDF");
        let failures = [failure("arabic-rtl.docx", "pdf", "status", "arabic-rtl.docx plain pdf: signal")];
        let (kept, rows) = settle_run(&run1, &run2, &failures, &scope(&declared));
        assert_eq!((kept.len(), rows.len()), (1, 0));
        // run2's crashed open is vouched for by run1's open, and takes its
        // skipped PDF with it.
        assert_eq!(flaky(&run2, &run1, &Scope { run: "run2".into(), ..scope(&declared) }).len(), 2);
        for other in ["exit", "empty", "skipped", "unexpected-ok"] {
            let other_run = run_table(&[("arabic-rtl.docx", "plain", "open", "ok", ""), ("arabic-rtl.docx", "plain", "pdf", other, "")]);
            assert!(flaky(&run1, &other_run, &scope(&declared)).is_empty(), "{}", other);
        }
        let expected_fail = run_table(&[("arabic-rtl.docx", "plain", "pdf", "expected-fail", "")]);
        assert_eq!(flaky(&run1, &expected_fail, &scope(&declared)).len(), 1);
    }

    // A flaky entry lets the crash pass, not what the crashed step's partial
    // output fails: those checks stay failures unless a bug entry names them.
    #[test]
    fn a_flaky_crash_does_not_take_away_what_its_partial_output_fails() {
        let run1 = run_table(&[("arabic-rtl.docx", "plain", "pdf", "signal", "6"), ("notes.txt", "plain", "save", "signal", "11")]);
        let run2 = run_table(&[("arabic-rtl.docx", "plain", "pdf", "ok", ""), ("notes.txt", "plain", "save", "ok", "")]);
        let flaky_rows = "arabic-rtl.docx\tplain\tpdf\t*\tlinux-arm64\tflaky\tr\t\nnotes.txt\tplain\tsave\t*\tlinux-arm64\tflaky\tr\t\n";
        let failures = [
            failure("arabic-rtl.docx", "pdf", "status", "arabic-rtl.docx plain pdf: signal"),
            failure("arabic-rtl.docx", "pdf", "fonts.script", "arabic-rtl.docx plain pdf: PDF without an embedded font"),
            failure("arabic-rtl.docx", "pdf", "expected", "arabic-rtl.docx plain pdf: PDF without its text"),
            failure("notes.txt", "save", "status", "notes.txt plain save: signal"),
            failure("notes.txt", "save", "roundtrip.bytes", "notes.txt plain save: the TXT changed"),
        ];
        let (kept, rows) = settle_run(&run1, &run2, &failures, &scope(&known_entries(flaky_rows)));
        assert_eq!(
            kept,
            vec![
                "arabic-rtl.docx plain pdf: PDF without an embedded font",
                "arabic-rtl.docx plain pdf: PDF without its text",
                "notes.txt plain save: the TXT changed"
            ]
        );
        let flaky_steps: Vec<(&str, &str)> = rows.iter().map(|r| (r[0].as_str(), r[3].as_str())).collect();
        assert_eq!(flaky_steps, vec![("arabic-rtl.docx", "flaky"), ("notes.txt", "flaky")]);

        // A bug entry that names the document and the check still covers it.
        let with_bug = format!("{}arabic-rtl.docx\tplain\tpdf\tfonts.script\tlinux-arm64\tbug\tr\t#7\n", flaky_rows);
        let (kept, rows) = settle_run(&run1, &run2, &failures, &scope(&known_entries(&with_bug)));
        assert_eq!(kept, vec!["arabic-rtl.docx plain pdf: PDF without its text", "notes.txt plain save: the TXT changed"]);
        assert!(rows.iter().any(|r| r[0] == "arabic-rtl.docx" && r[3] == "known-bug"), "{:?}", rows);
    }

    // A crash in both runs fails even with an exact bug entry for its status;
    // a bug entry still covers a crash in one run, or an exit in both.
    #[test]
    fn a_crash_in_both_runs_fails_whatever_known_tsv_says() {
        let crashed = run_table(&[("arabic-rtl.docx", "plain", "pdf", "signal", "6")]);
        let failures = [failure("arabic-rtl.docx", "pdf", "status", "arabic-rtl.docx plain pdf: signal")];
        let bug = "arabic-rtl.docx\tplain\tpdf\tstatus\tlinux-arm64\tbug\tcrash being fixed\t#9\n";
        let both = known_entries(&format!("{}{}", bug, ARABIC_FLAKY));
        let (kept, rows) = settle_run(&crashed, &crashed, &failures, &scope(&both));
        assert_eq!((kept, rows.len()), (vec!["arabic-rtl.docx plain pdf: signal".to_string()], 0));

        let bug_only = known_entries(bug);
        let fine = run_table(&[("arabic-rtl.docx", "plain", "pdf", "ok", "")]);
        let (kept, rows) = settle_run(&crashed, &fine, &failures, &scope(&bug_only));
        assert!(kept.is_empty());
        assert_eq!(rows[0][3], "known-bug");
        let exited = run_table(&[("arabic-rtl.docx", "plain", "pdf", "exit", "")]);
        let exit_failure = [failure("arabic-rtl.docx", "pdf", "status", "arabic-rtl.docx plain pdf: exit 1")];
        let (kept, rows) = settle_run(&exited, &exited, &exit_failure, &scope(&bug_only));
        assert!(kept.is_empty());
        assert_eq!(rows[0][3], "known-bug");
    }

    fn text_entry(file: &str) -> Entry {
        let text = format!("file\tclass\tsteps\texpect\tvariants\tsave_as\tnote\n{}\tcomparable\topen,save\t\t\t\t\n", file);
        manifest::parse(&text).unwrap().remove(0)
    }

    #[test]
    fn a_txt_that_does_not_come_back_byte_for_byte_is_a_hard_failure() {
        let dir = std::env::temp_dir().join(format!("eo-report-roundtrip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let original_txt = write("original.txt", b"\xFE\xFF\x00H\x00\x0A");
        let original_csv = write("original.csv", b"a;b\n\n");
        let same_txt = write("same.txt", b"\xFE\xFF\x00H\x00\x0A");
        let lost_txt = write("lost.txt", b"\xFE\xFF\x00\x0A");
        let changed_csv = write("changed.csv", b"a;b\n;\n");
        let mut report = Report::new(&dir);

        check_text_file(&mut report, &text_entry("a.txt"), "plain", "save", &same_txt, &original_txt).unwrap();
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        check_text_file(&mut report, &text_entry("a.txt"), "plain", "save", &lost_txt, &original_txt).unwrap();
        assert_eq!(report.failures, vec!["a.txt plain save: the TXT changed in an open and save (diff:3)"]);
        // CSV round trips only inform: x2t 9.4 still changes some of them.
        check_text_file(&mut report, &text_entry("b.csv"), "plain", "save", &changed_csv, &original_csv).unwrap();
        assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
        let roundtrips: Vec<&str> = report.table.rows.iter().filter(|r| r[3] == "roundtrip.bytes").map(|r| r[4].as_str()).collect();
        assert_eq!(roundtrips, vec!["equal", "diff:3", "diff:4"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // summary.md goes into the run's Markdown summary: what it shows from the
    // results (env.tsv, package.txt, x2t's messages, statuses) stays plain.
    #[test]
    fn the_summary_keeps_values_from_the_results_plain() {
        let payload = "x\n## Owned\n[link](http://e.x) <b>";
        let root = std::env::temp_dir().join(format!("corpus-report-md-{}", std::process::id()));
        let out = root.join("run1");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(root.join("package.txt"), payload).unwrap();
        let mut table = Table::new(&REPORT_COLUMNS);
        table.push(vec!["sheet.ods".into(), "plain".into(), "open".into(), "status".into(), payload.into()]);
        let env = |_: &str| payload.to_string();
        let failures = vec![format!("sheet.ods plain open: exit {}", payload)];
        let text = summary(&table, &[entry("comparable")], &env, &failures, &[], &out);
        let _ = std::fs::remove_dir_all(&root);
        for bad in ["\n## Owned", "[link]", "](http", "<b>"] {
            assert!(!text.contains(bad), "{:?} in:\n{}", bad, text);
        }
        assert!(text.starts_with("## Conversion corpus\n"), "{}", text);
    }

    #[test]
    fn the_summary_says_when_x2t_runs_emulated() {
        let env_of = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string()).unwrap_or_default()
        };
        let win_arm = env_of(&[("host.arch", "aarch64"), ("sidecar.arch", "x86_64"), ("pdf_x2t.arch", "x86_64")]);
        assert_eq!(architecture_line(&win_arm), "- Architecture: host aarch64, sidecar x2t x86_64, PDF x2t x86_64 (emulated)\n");
        let native = env_of(&[("host.arch", "aarch64"), ("sidecar.arch", "aarch64"), ("pdf_x2t.arch", "aarch64")]);
        assert!(!architecture_line(&native).contains("emulated"));
    }

    const MANIFEST: &str =
        "file\tclass\tsteps\texpect\tvariants\tsave_as\tnote\nsheet.ods\tcomparable\topen,save,pdf\tHello\t\txlsx\t\n";

    // The trusted manifest kept next to a results folder by these tests.
    fn trusted(out: &Path) -> std::path::PathBuf {
        out.parent().unwrap().join("trusted-manifest.tsv")
    }

    // A results folder whose run.tsv holds one given row for sheet.ods.
    fn results_with_row(name: &str, variant: &str, step: &str, output: &str) -> std::path::PathBuf {
        results_with(name, MANIFEST, variant, step, output)
    }

    // The same, with `manifest` as the copy that travels in the results.
    fn results_with(name: &str, manifest: &str, variant: &str, step: &str, output: &str) -> std::path::PathBuf {
        let out = std::env::temp_dir().join(format!("corpus-report-{}-{}", name, std::process::id())).join("run1");
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(trusted(&out), MANIFEST).unwrap();
        std::fs::write(out.join("manifest.tsv"), manifest).unwrap();
        std::fs::write(out.join("env.tsv"), "key\tvalue\nfonts.check\tok\n").unwrap();
        std::fs::write(
            out.join("run.tsv"),
            format!(
                "{}\nsheet.ods\t{}\t{}\tok\t0\t\t5\tabc\t{}\t1\t\n",
                crate::run::RUN_COLUMNS.join("\t"),
                variant,
                step,
                output
            ),
        )
        .unwrap();
        out
    }

    fn report_error(out: &Path) -> String {
        let result = report(out, &trusted(out), &Options::default());
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
        match result {
            Err(Infra(message)) => message,
            Ok(failures) => panic!("accepted, with failures {:?}", failures),
        }
    }

    // The results come from another job's artifact: a row must not make the
    // report read or write outside its folder.
    #[test]
    fn an_output_outside_the_results_folder_is_refused() {
        for (n, output) in [
            "../escape.pdf",
            "files/../../escape.pdf",
            "/etc/passwd",
            "\\\\server\\share\\x.pdf",
            "C:/Windows/x.pdf",
            "files//x.pdf",
            "files/./x.pdf",
        ]
        .iter()
        .enumerate()
        {
            let out = results_with_row(&format!("out{}", n), "plain", "pdf", output);
            let message = report_error(&out);
            assert!(message.contains("output"), "{}: {}", output, message);
        }
    }

    #[test]
    fn a_variant_or_step_run_never_writes_is_refused() {
        for (n, (variant, step)) in [
            ("../../x", "pdf"),
            ("other", "pdf"),
            ("plain", "../../x"),
            ("plain", "save_as.docx"),
            ("plain", "print"),
        ]
        .iter()
        .enumerate()
        {
            let out = results_with_row(&format!("row{}", n), variant, step, "");
            let message = report_error(&out);
            assert!(message.contains("variant") || message.contains("step"), "{} {}: {}", variant, step, message);
        }
    }

    #[test]
    fn outputs_run_writes_are_accepted() {
        for output in ["files/sheet.ods/plain/export.pdf", "files/sheet.ods/special-name/save_as.xlsx"] {
            assert_eq!(relative_output(output), Ok(()), "{}", output);
        }
        let out = results_with_row("valid", "plain", "save_as.xlsx", "files/sheet.ods/plain/save_as.xlsx");
        let runs = Table::read(&out.join("run.tsv")).unwrap();
        let entries = manifest::parse(&std::fs::read_to_string(out.join("manifest.tsv")).unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
        assert!(check_rows(&runs, &entries).is_ok());
    }

    // Results refused by report must not keep the report.tsv, summary.md
    // and text/ they arrived with, which compare and the summary would read.
    #[test]
    fn a_refused_report_leaves_none_of_its_outputs_behind() {
        let out = results_with("stale", &MANIFEST.replace("Hello", "Hullo"), "plain", "open", "");
        std::fs::write(out.join("report.tsv"), "doc\tvariant\tstep\tcheck\tvalue\nforged\tplain\topen\tstatus\tok\n").unwrap();
        std::fs::write(out.join("summary.md"), "## forged\n").unwrap();
        std::fs::create_dir_all(out.join("text/forged")).unwrap();
        let result = report(&out, &trusted(&out), &Options::default());
        let left: Vec<bool> = ["report.tsv", "summary.md", "text"].iter().map(|name| out.join(name).exists()).collect();
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
        assert!(result.is_err());
        assert_eq!(left, vec![false, false, false]);
    }

    // save_as reaches the step name, which names files under text/.
    #[test]
    fn a_step_that_is_not_one_plain_name_is_refused() {
        let out = results_with_row("step", "plain", "save_as.x/../../escaped.docx", "");
        let runs = Table::read(&out.join("run.tsv")).unwrap();
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
        let mut entries = manifest::parse(MANIFEST).unwrap();
        entries[0].save_as = vec!["x/../../escaped.docx".to_string()];
        match check_rows(&runs, &entries) {
            Err(Infra(message)) => assert!(message.contains("step"), "{}", message),
            Ok(()) => panic!("a step with a path was accepted"),
        }
    }

    // The manifest in the results came with them; only the checkout's own
    // decides which documents, steps and names are valid.
    #[test]
    fn a_results_manifest_that_differs_from_the_trusted_one_is_refused() {
        let hostile = MANIFEST.replace("\txlsx\t", "\tx/../../escaped.docx\t");
        let changed = MANIFEST.replace("Hello", "Hullo");
        let crlf = MANIFEST.replace('\n', "\r\n");
        for (n, (manifest, step)) in [
            (hostile.as_str(), "save_as.x/../../escaped.docx"),
            (changed.as_str(), "open"),
            (crlf.as_str(), "open"),
        ]
        .iter()
        .enumerate()
        {
            let out = results_with(&format!("manifest{}", n), manifest, "plain", step, "");
            let message = report_error(&out);
            assert!(message.contains("manifest"), "{}: {}", n, message);
        }
        let out = results_with("manifest-gone", MANIFEST, "plain", "open", "");
        std::fs::remove_file(trusted(&out)).unwrap();
        assert!(report_error(&out).contains("manifest"));
    }

    #[test]
    fn the_first_differing_byte_is_reported() {
        assert_eq!(byte_diff(b"abc", b"abc"), "equal");
        assert_eq!(byte_diff(b"abc", b"abd"), "diff:2");
        assert_eq!(byte_diff(b"abc", b"abcd"), "diff:3");
        assert_eq!(byte_diff(b"", b"x"), "diff:0");
    }

    #[test]
    fn package_text_picks_the_part_of_each_format() {
        let mut docx = BTreeMap::new();
        docx.insert("word/document.xml".to_string(), b"<w:p><w:r><w:t>Hi</w:t></w:r></w:p>".to_vec());
        assert_eq!(package_text("docx", &docx), "Hi\n");
        let mut odt = BTreeMap::new();
        odt.insert("content.xml".to_string(), b"<text:p>Odt</text:p>".to_vec());
        assert_eq!(package_text("odp", &odt), "Odt\n");
        let mut pptx = BTreeMap::new();
        pptx.insert("ppt/slides/slide1.xml".to_string(), b"<a:p><a:r><a:t>S</a:t></a:r></a:p>".to_vec());
        pptx.insert("ppt/slides/_rels/slide1.xml.rels".to_string(), b"<r/>".to_vec());
        assert_eq!(package_text("pptx", &pptx), "# slide 1\nS\n");
    }

    #[test]
    fn summary_cells_show_failures_missing_text_and_diffs() {
        let mut checks = BTreeMap::new();
        checks.insert(("open", "status"), "ok");
        checks.insert(("save", "status"), "ok");
        checks.insert(("save", "roundtrip.bytes"), "diff:4");
        checks.insert(("pdf", "status"), "ok");
        checks.insert(("pdf", "expected"), "missing:Hello");
        checks.insert(("save_as.xlsx", "status"), "ok");
        checks.insert(("save_as.xlsx", "class"), "auto-system-fonts");
        checks.insert(("save_as.csv", "status"), "exit");
        assert_eq!(cell(&checks, "open"), "ok");
        assert_eq!(cell(&checks, "save"), "diff");
        assert_eq!(cell(&checks, "pdf"), "FAIL text missing");
        assert_eq!(cell(&checks, "save_as.xlsx"), "ok (auto-system-fonts)");
        assert_eq!(cell(&checks, "save_as.csv"), "FAIL exit");
        assert_eq!(cell(&checks, "nothing"), "");

        let mut script = BTreeMap::new();
        script.insert(("pdf", "status"), "ok");
        script.insert(("pdf", "expected"), "ok");
        script.insert(("pdf", "fonts.script"), "bundled-only");
        assert_eq!(cell(&script, "pdf"), "FAIL script font");
        script.insert(("pdf", "fonts.script"), "ok");
        assert_eq!(cell(&script, "pdf"), "ok");

        // A step known.tsv settled says so instead of FAIL.
        let mut settled = BTreeMap::new();
        settled.insert(("pdf", "status"), "signal");
        settled.insert(("pdf", "flaky"), "signal 6 on linux-arm64 in run1 (the other run: ok)");
        assert_eq!(cell(&settled, "pdf"), "flaky");
        settled.remove(&("pdf", "flaky"));
        settled.insert(("pdf", "known-bug"), "known.tsv line 2 (bug, #7): r: x");
        assert_eq!(cell(&settled, "pdf"), "known bug");
    }
}
