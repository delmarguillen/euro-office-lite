// `corpus report`: reads what `run` wrote and checks each output with
// poppler-utils and unzip, writing report.tsv, the extracted text and
// summary.md. A crash, a failed exit, an empty output or a PDF without its
// expected text fails the report; every other finding is informative.

use crate::manifest::{self, Entry};
use crate::markdown::clean;
use crate::run::Infra;
use crate::tsv::Table;
use crate::{allfonts, arch, normalize, pdf, sha256};
use std::collections::BTreeMap;
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
}

impl Report<'_> {
    fn push(&mut self, doc: &str, variant: &str, step: &str, check: &str, value: impl Into<String>) {
        self.table.push(vec![doc.into(), variant.into(), step.into(), check.into(), value.into()]);
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

pub fn report(out: &Path, trusted_manifest: &Path) -> Result<Vec<String>, Infra> {
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
    let mut report = Report { out, table: Table::new(&REPORT_COLUMNS), failures: Vec::new() };

    for row in &runs.rows {
        let get = |name: &str| runs.get(row, name).to_string();
        let (doc, variant, step, status) = (get("doc"), get("variant"), get("step"), get("status"));
        let Some(entry) = entries.iter().find(|e| e.doc == doc) else {
            return Err(Infra(format!("{} is in run.tsv but not in the manifest", doc)));
        };
        report.push(&doc, &variant, &step, "status", status.clone());
        if status != "ok" && status != "expected-fail" {
            report.failures.push(format!("{} {} {}: {} {}", doc, variant, step, status, get("message")));
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
            check_pdf(&mut report, entry, &variant, &step, &file, spoofed)
        } else if PACKAGES.contains(&ext.as_str()) {
            check_package(&mut report, entry, &variant, &step, &file, &ext, &original, &scratch, spoofed)
        } else {
            check_text_file(&mut report, entry, &variant, &step, &file, &original)
        };
        if let Err(message) = result {
            report.push(&doc, &variant, &step, "report.error", message.clone());
            report.failures.push(format!("{} {} {}: cannot read the output: {}", doc, variant, step, message));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    report.table.write(&out.join("report.tsv")).map_err(Infra)?;
    let summary = summary(&report.table, &entries, &env_value, &report.failures, out);
    std::fs::write(out.join("summary.md"), summary).map_err(|e| Infra(format!("summary.md: {}", e)))?;
    Ok(report.failures)
}

fn check_pdf(report: &mut Report, entry: &Entry, variant: &str, step: &str, file: &Path, spoofed: bool) -> Result<(), String> {
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
    report.push(doc, variant, step, "class", class_value(entry, &[], spoofed));
    Ok(())
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
        report.failures.push(format!("{} {} {}: PDF without its text ({})", doc, variant, step, value));
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
            report.push(doc, variant, step, "roundtrip.bytes", byte_diff(&original_bytes, &bytes));
        }
    }
    report.push(doc, variant, step, "class", entry.class.clone());
    Ok(())
}

// The cell of one step in the summary table.
fn cell(checks: &BTreeMap<(&str, &str), &str>, step: &str) -> String {
    let Some(status) = checks.get(&(step, "status")) else { return String::new() };
    match *status {
        "ok" => {}
        "expected-fail" => return "expected fail".to_string(),
        other => return format!("FAIL {}", other),
    }
    if checks.get(&(step, "expected")).is_some_and(|v| v.starts_with("missing:")) && step == "pdf" {
        return "FAIL text missing".to_string();
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
fn summary(table: &Table, entries: &[Entry], env: &dyn Fn(&str) -> String, failures: &[String], out: &Path) -> String {
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
        let mut report = Report { out: &out, table: Table::new(&REPORT_COLUMNS), failures: Vec::new() };
        pdf_expectation(&mut report, &e, "plain", "pdf", "Hello\nworld\n");
        assert!(report.failures.is_empty());
        pdf_expectation(&mut report, &e, "plain", "pdf", "\x0c\n");
        assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
        assert!(report.failures[0].contains("missing:Hello"));
        let values: Vec<&str> = report.table.rows.iter().map(|r| r[4].as_str()).collect();
        assert_eq!(values, vec!["ok", "missing:Hello"]);
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
        let text = summary(&table, &[entry("comparable")], &env, &failures, &out);
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
        let result = report(out, &trusted(out));
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
        let result = report(&out, &trusted(&out));
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
    }
}
