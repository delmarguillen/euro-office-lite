// `corpus compare`: lines up two results (env.tsv, run.tsv and report.tsv)
// key by key. Two runs of the same build must come out the same; whatever
// differs is a volatile value the normalization still misses.

use crate::run::Infra;
use crate::tsv::Table;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

type Key = (String, String, String, String, String);

// Where each run happens, not what it produced.
const IGNORED_ENV: [&str; 6] = ["install_root", "exe", "resource_dir", "binaries_dir", "sidecar.path", "pdf_x2t.path"];
// Packages and PDFs carry dates and ids in their bytes: report.tsv compares
// their normalized parts and text instead.
const BYTES_NOT_COMPARED: [&str; 7] = ["docx", "xlsx", "pptx", "odt", "ods", "odp", "pdf"];

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
            fields.extend(["size", "sha256"]);
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
                let cut = |x: &Option<String>| x.clone().unwrap_or_default().chars().take(80).collect::<String>().replace('|', "/");
                s.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                    src, d, v, st, c, cut(&line.a), cut(&line.b), line.verdict()
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

    #[test]
    fn compare_writes_the_table_and_the_summary() {
        let root = std::env::temp_dir().join(format!("corpus-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (run, sha) in [("run1", "csv1"), ("run2", "csv2")] {
            let dir = root.join(run);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("env.tsv"), ENV).unwrap();
            std::fs::write(dir.join("run.tsv"), RUN.replace("csv1", sha)).unwrap();
            std::fs::write(dir.join("report.tsv"), REPORT).unwrap();
        }
        let summary = root.join("determinism.md");
        assert_eq!(compare(&root.join("run1"), &root.join("run2"), Some(&summary)).unwrap(), 1);
        let text = std::fs::read_to_string(&summary).unwrap();
        assert!(text.contains("1 differ or are missing"), "{}", text);
        assert!(text.contains("| run | b.csv | plain | save | sha256 | csv1 | csv2 | diff |"), "{}", text);
        assert!(root.join("compare.tsv").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
