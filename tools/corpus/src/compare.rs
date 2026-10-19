// `corpus compare`: lines up two results (env.tsv, run.tsv and report.tsv)
// key by key. Two runs of the same build must come out the same; whatever
// differs is a volatile value the normalization still misses.

use crate::markdown::clean;
use crate::run::Infra;
use crate::tsv::Table;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

// source (env, run or report), doc, variant, step, check.
pub type Key = (String, String, String, String, String);

// Where each run happens, not what it produced.
const IGNORED_ENV: [&str; 6] = ["install_root", "exe", "resource_dir", "binaries_dir", "sidecar.path", "pdf_x2t.path"];
// Packages and PDFs carry dates and ids in their bytes: report.tsv compares
// their normalized parts and text instead.
const BYTES_NOT_COMPARED: [&str; 7] = ["docx", "xlsx", "pptx", "odt", "ods", "odp", "pdf"];

// x2t's Editor.bin from a CSV or TXT differs between two runs of the same
// input (same size, other bytes), so opening one is judged by its size and by
// the saved file, which the round trip compares byte for byte.
fn is_text_document(doc: &str) -> bool {
    let doc = doc.to_lowercase();
    doc.ends_with(".csv") || doc.ends_with(".txt")
}

// The Editor.bin size of a CSV or TXT open, compared within 1%.
pub fn is_text_open_size(key: &Key) -> bool {
    let (source, doc, _, step, check) = key;
    step == "open"
        && is_text_document(doc)
        && ((source == "run" && check == "size") || (source == "report" && check == "editor.size"))
}

pub fn within_one_percent(a: &str, b: &str) -> bool {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(a), Ok(b)) => a.abs_diff(b) * 100 <= a.max(b),
        _ => a == b,
    }
}

fn key(source: &str, doc: &str, variant: &str, step: &str, check: &str) -> Key {
    (source.into(), doc.into(), variant.into(), step.into(), check.into())
}

pub fn values(env: &Table, run: &Table, report: &Table) -> BTreeMap<Key, String> {
    let mut map = BTreeMap::new();
    for row in &env.rows {
        let name = env.get(row, "key");
        if !IGNORED_ENV.contains(&name) {
            map.insert(key("env", "", "", "", name), env.get(row, "value").to_string());
        }
    }
    for row in &run.rows {
        let (doc, variant, step) = (run.get(row, "doc"), run.get(row, "variant"), run.get(row, "step"));
        let output = run.get(row, "output");
        let ext = output.rsplit('.').next().unwrap_or("").to_lowercase();
        let mut fields = vec!["status", "exit", "signal", "output"];
        if output.is_empty() || !BYTES_NOT_COMPARED.contains(&ext.as_str()) {
            fields.push("size");
            if !(step == "open" && is_text_document(doc)) {
                fields.push("sha256");
            }
        }
        for field in fields {
            map.insert(key("run", doc, variant, step, field), run.get(row, field).to_string());
        }
    }
    for row in &report.rows {
        let check = report.get(row, "check");
        // What x2t writes as the application is recorded, not compared (#41).
        if check.starts_with("app.") {
            continue;
        }
        if check == "editor.sha256" && is_text_document(report.get(row, "doc")) {
            continue;
        }
        map.insert(
            key("report", report.get(row, "doc"), report.get(row, "variant"), report.get(row, "step"), check),
            report.get(row, "value").to_string(),
        );
    }
    map
}

#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    pub key: Key,
    pub a: Option<String>,
    pub b: Option<String>,
}

impl Line {
    pub fn verdict(&self) -> &'static str {
        match (&self.a, &self.b) {
            (Some(a), Some(b)) if a == b => "same",
            (Some(a), Some(b)) if is_text_open_size(&self.key) && within_one_percent(a, b) => "same",
            (Some(_), Some(_)) => "diff",
            _ => "missing",
        }
    }
}

pub fn lines(a: &BTreeMap<Key, String>, b: &BTreeMap<Key, String>) -> Vec<Line> {
    let keys: BTreeSet<&Key> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .map(|k| Line { key: k.clone(), a: a.get(k).cloned(), b: b.get(k).cloned() })
        .collect()
}

fn load(out: &Path) -> Result<BTreeMap<Key, String>, Infra> {
    let read = |name: &str| Table::read(&out.join(name)).map_err(Infra);
    Ok(values(&read("env.tsv")?, &read("run.tsv")?, &read("report.tsv")?))
}

// Returns how many keys differ or are missing on one side.
pub fn compare(a: &Path, b: &Path, summary: Option<&Path>) -> Result<usize, Infra> {
    let lines = lines(&load(a)?, &load(b)?);
    let mut table = Table::new(&["source", "doc", "variant", "step", "check", "a", "b", "verdict"]);
    for line in &lines {
        let (s, d, v, st, c) = &line.key;
        table.push(vec![
            s.clone(),
            d.clone(),
            v.clone(),
            st.clone(),
            c.clone(),
            line.a.clone().unwrap_or_default(),
            line.b.clone().unwrap_or_default(),
            line.verdict().to_string(),
        ]);
    }
    let folder = a.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    table.write(&folder.join("compare.tsv")).map_err(Infra)?;
    let wrong: Vec<&Line> = lines.iter().filter(|l| l.verdict() != "same").collect();
    if let Some(path) = summary {
        let mut s = format!(
            "## Determinism: {} against {}\n\n{} values compared, {} differ or are missing.\n",
            a.display(),
            b.display(),
            lines.len(),
            wrong.len()
        );
        if !wrong.is_empty() {
            s.push_str("\n| Source | Doc | Variant | Step | Check | A | B | Verdict |\n|---|---|---|---|---|---|---|---|\n");
            for line in wrong.iter().take(200) {
                let (src, d, v, st, c) = &line.key;
                let cut = |x: &Option<String>| clean(&x.clone().unwrap_or_default().chars().take(80).collect::<String>());
                s.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                    clean(src),
                    clean(d),
                    clean(v),
                    clean(st),
                    clean(c),
                    cut(&line.a),
                    cut(&line.b),
                    line.verdict()
                ));
            }
        }
        let sizes: Vec<&Line> = lines.iter().filter(|l| l.key.0 == "run" && is_text_open_size(&l.key)).collect();
        if !sizes.is_empty() {
            s.push_str("\nEditor.bin size when opening CSV and TXT (compared within 1%):\n\n| Doc | Variant | A | B | |\n|---|---|---|---|---|\n");
            for line in sizes {
                let flag = if line.verdict() == "same" { "" } else { "differs by more than 1%" };
                s.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    clean(&line.key.1),
                    clean(&line.key.2),
                    clean(&line.a.clone().unwrap_or_default()),
                    clean(&line.b.clone().unwrap_or_default()),
                    flag
                ));
            }
        }
        std::fs::write(path, s).map_err(|e| Infra(format!("cannot write {}: {}", path.display(), e)))?;
    }
    Ok(wrong.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        Table::parse(text)
    }

    const ENV: &str = "key\tvalue\nexe\t/w1/usr/bin/euro-office-lite\nsidecar.sha256\tabc\n";
    const RUN: &str = "doc\tvariant\tstep\tstatus\texit\tsignal\tsize\tsha256\toutput\tms\tmessage\n\
a.docx\tplain\topen\tok\t0\t\t10\taaa\t\t120\t\n\
a.docx\tplain\tsave\tok\t0\t\t900\tzip1\tfiles/a.docx/plain/save.docx\t300\t\n\
b.csv\tplain\tsave\tok\t0\t\t5\tcsv1\tfiles/b.csv/plain/save.csv\t40\t/w1/doc-2\n";
    const REPORT: &str = "doc\tvariant\tstep\tcheck\tvalue\n\
a.docx\tplain\tsave\tpart:word/document.xml\th1\n\
a.docx\tplain\tsave\tapp.application\tx2t 8.2\n";

    #[test]
    fn identical_runs_are_the_same_despite_times_paths_and_zip_bytes() {
        let a = values(&table(ENV), &table(RUN), &table(REPORT));
        let run_b = RUN.replace("\t120\t", "\t99\t").replace("zip1", "zip2").replace("/w1/doc-2", "/w2/doc-2");
        let env_b = ENV.replace("/w1/", "/w2/");
        let report_b = REPORT.replace("x2t 8.2", "x2t 9.4");
        let b = values(&table(&env_b), &table(&run_b), &table(&report_b));
        let lines = lines(&a, &b);
        assert!(lines.iter().all(|l| l.verdict() == "same"), "{:?}", lines.iter().filter(|l| l.verdict() != "same").collect::<Vec<_>>());
    }

    const TEXT_RUN: &str = "doc\tvariant\tstep\tstatus\texit\tsignal\tsize\tsha256\toutput\tms\tmessage\n\
c.csv\tplain\topen\tok\t0\t\t1600\tbin1\t\t10\t\n\
d.docx\tplain\topen\tok\t0\t\t4000\tbin1\t\t10\t\n";
    const TEXT_REPORT: &str = "doc\tvariant\tstep\tcheck\tvalue\n\
c.csv\tplain\topen\teditor.size\t1600\n\
c.csv\tplain\topen\teditor.sha256\tbin1\n";

    fn text_lines(size_b: &str) -> Vec<Line> {
        let a = values(&table(ENV), &table(TEXT_RUN), &table(TEXT_REPORT));
        let run_b = TEXT_RUN.replace("1600\tbin1", &format!("{}\tbin2", size_b));
        let report_b = TEXT_REPORT.replace("1600", size_b).replace("bin1", "bin2");
        let b = values(&table(ENV), &table(&run_b), &table(&report_b));
        lines(&a, &b)
    }

    fn not_same(lines: &[Line]) -> Vec<String> {
        lines.iter().filter(|l| l.verdict() != "same").map(|l| format!("{}/{}/{}", l.key.0, l.key.1, l.key.4)).collect()
    }

    // The Editor.bin of a CSV open changes bytes between identical runs.
    #[test]
    fn a_csv_editor_bin_is_judged_by_its_size_within_one_percent() {
        assert!(not_same(&text_lines("1600")).is_empty(), "{:?}", not_same(&text_lines("1600")));
        assert!(not_same(&text_lines("1615")).is_empty(), "{:?}", not_same(&text_lines("1615")));
        assert_eq!(not_same(&text_lines("1620")), vec!["report/c.csv/editor.size", "run/c.csv/size"]);
    }

    #[test]
    fn other_documents_still_compare_the_editor_bin_bytes() {
        let a = values(&table(ENV), &table(TEXT_RUN), &table(TEXT_REPORT));
        let run_b = TEXT_RUN.replace("4000\tbin1", "4000\tbin9");
        let b = values(&table(ENV), &table(&run_b), &table(TEXT_REPORT));
        assert_eq!(not_same(&lines(&a, &b)), vec!["run/d.docx/sha256"]);
    }

    #[test]
    fn one_percent_is_measured_on_the_larger_size() {
        assert!(within_one_percent("1000", "1010"));
        assert!(!within_one_percent("1000", "1011"));
        assert!(within_one_percent("0", "0"));
        assert!(!within_one_percent("0", "5"));
        assert!(!within_one_percent("", "5"));
    }

    #[test]
    fn a_changed_value_or_a_missing_row_is_reported() {
        let a = values(&table(ENV), &table(RUN), &table(REPORT));
        let run_b = RUN.replace("csv1", "csv2").replace("a.docx\tplain\topen\tok", "a.docx\tplain\topen\texit");
        let report_b = "doc\tvariant\tstep\tcheck\tvalue\n";
        let b = values(&table(ENV), &table(&run_b), &table(report_b));
        let wrong: Vec<(String, &str)> = lines(&a, &b)
            .iter()
            .filter(|l| l.verdict() != "same")
            .map(|l| (format!("{}/{}/{}/{}", l.key.0, l.key.1, l.key.3, l.key.4), l.verdict()))
            .collect();
        assert_eq!(
            wrong,
            vec![
                ("report/a.docx/save/part:word/document.xml".to_string(), "missing"),
                ("run/a.docx/open/status".to_string(), "diff"),
                ("run/b.csv/save/sha256".to_string(), "diff"),
            ]
        );
    }

    // determinism.md goes into the run's Markdown summary, and its values
    // come from the results.
    #[test]
    fn the_summary_keeps_values_from_the_results_plain() {
        let root = std::env::temp_dir().join(format!("corpus-compare-md-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let payload = crate::tsv::escape("x\n## Owned\n[link](http://e.x) <b>");
        for (run, version) in [("run1", "8.2"), ("run2", payload.as_str())] {
            let dir = root.join(run);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("env.tsv"), format!("{}sidecar.version\t{}\n", ENV, version)).unwrap();
            std::fs::write(dir.join("run.tsv"), RUN).unwrap();
            std::fs::write(dir.join("report.tsv"), REPORT).unwrap();
        }
        let summary = root.join("determinism.md");
        assert_eq!(compare(&root.join("run1"), &root.join("run2"), Some(&summary)).unwrap(), 1);
        let text = std::fs::read_to_string(&summary).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        for bad in ["\n## Owned", "[link]", "](http", "<b>"] {
            assert!(!text.contains(bad), "{:?} in:\n{}", bad, text);
        }
        assert!(text.contains("| env |  |  |  | sidecar.version | 8.2 | x?"), "{}", text);
    }

    #[test]
    fn compare_writes_the_table_and_the_summary() {
        let root = std::env::temp_dir().join(format!("corpus-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (run, sha) in [("run1", "csv1"), ("run2", "csv2")] {
            let dir = root.join(run);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("env.tsv"), ENV).unwrap();
            // A CSV open whose Editor.bin bytes differ per run, as x2t's do.
            let csv_open = format!("c.csv\tplain\topen\tok\t0\t\t1600\tbin-{}\t\t10\t\n", run);
            std::fs::write(dir.join("run.tsv"), RUN.replace("csv1", sha) + &csv_open).unwrap();
            std::fs::write(dir.join("report.tsv"), REPORT).unwrap();
        }
        let summary = root.join("determinism.md");
        assert_eq!(compare(&root.join("run1"), &root.join("run2"), Some(&summary)).unwrap(), 1);
        let text = std::fs::read_to_string(&summary).unwrap();
        assert!(text.contains("1 differ or are missing"), "{}", text);
        assert!(text.contains("| run | b.csv | plain | save | sha256 | csv1 | csv2 | diff |"), "{}", text);
        assert!(root.join("compare.tsv").is_file());
        assert!(text.contains("Editor.bin size when opening CSV and TXT"), "{}", text);
        let _ = std::fs::remove_dir_all(&root);
    }
}
