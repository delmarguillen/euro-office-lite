// `corpus baseline`: the values each platform must keep, committed as
// tests/corpus/baseline/<platform>.tsv. Every run writes a candidate per
// platform and checks the committed one: a new value, a value gone or a value
// changed fails it, so a change on one platform shows even when the others
// agree. Updating is copying a reviewed candidate over the committed file.
//
// Pinned: what compare compares between two runs, except what depends on
// the runner image (the system font list, the font cache bytes, the locale,
// the labels), and, for outputs drawn with the runner's fonts (system-fonts,
// auto-system-fonts, spoofed, or a known env entry), everything but how x2t
// ended (status, exit, signal).

use crate::compare::{self, Key};
use crate::known::{Kind, KnownEntry};
use crate::markdown::clean;
use crate::matrix::{self, Loaded};
use crate::run::Infra;
use crate::tsv::Table;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const COLUMNS: [&str; 6] = ["source", "doc", "variant", "step", "check", "value"];

// The env values that name the x2t under test and the bundled fonts.
const ENV_PINNED: [&str; 10] = [
    "host.arch",
    "sidecar.version",
    "sidecar.sha256",
    "sidecar.arch",
    "pdf_x2t.version",
    "pdf_x2t.sha256",
    "pdf_x2t.arch",
    "x2t_same_file",
    "fonts.bundled",
    "fonts.check",
];
// macOS builds sign x2t with a timestamp, so its hash changes on every build
// even when the ONLYOFFICE binary is the same; its version still pins it.
const ENV_RESIGNED: [&str; 2] = ["sidecar.sha256", "pdf_x2t.sha256"];
const OUTCOME: [&str; 3] = ["status", "exit", "signal"];

type StepKey = (String, String, String);

// The values one platform's baseline holds, from one run.
pub fn pinned(platform: &Loaded, known: &[KnownEntry]) -> BTreeMap<Key, String> {
    let mut out = BTreeMap::new();
    for (key, value) in &platform.values {
        let (source, doc, variant, step, check) = key;
        if source == "env" {
            let resigned = platform.id.starts_with("mac-") && ENV_RESIGNED.contains(&check.as_str());
            if ENV_PINNED.contains(&check.as_str()) && !resigned {
                out.insert(key.clone(), value.clone());
            }
            continue;
        }
        if platform.flaky.contains_key(&(doc.clone(), variant.clone(), step.clone())) {
            continue;
        }
        let runner_drawn = matrix::exclusion(platform, key).is_some()
            || known.iter().any(|e| {
                e.kind == Kind::Env && e.matches(doc, variant, step, check) && e.covers(&platform.id, &platform.family)
            });
        if !runner_drawn || OUTCOME.contains(&check.as_str()) {
            out.insert(key.clone(), value.clone());
        }
    }
    out
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub added: Vec<(Key, String)>,
    pub removed: Vec<(Key, String)>,
    pub changed: Vec<(Key, String, String)>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

// The baseline against this run's values. The steps report let pass as
// flaky are left out on both sides.
pub fn diff(baseline: &BTreeMap<Key, String>, now: &BTreeMap<Key, String>, flaky: &BTreeMap<StepKey, String>) -> Diff {
    let skip = |key: &Key| flaky.contains_key(&(key.1.clone(), key.2.clone(), key.3.clone()));
    let mut out = Diff::default();
    for (key, value) in now.iter().filter(|(k, _)| !skip(k)) {
        match baseline.get(key) {
            None => out.added.push((key.clone(), value.clone())),
            Some(old) if old == value => {}
            Some(old) if compare::is_text_open_size(key) && compare::within_one_percent(old, value) => {}
            Some(old) => out.changed.push((key.clone(), old.clone(), value.clone())),
        }
    }
    for (key, value) in baseline.iter().filter(|(k, _)| !skip(k)) {
        if !now.contains_key(key) {
            out.removed.push((key.clone(), value.clone()));
        }
    }
    out
}

pub fn to_table(values: &BTreeMap<Key, String>) -> Table {
    let mut table = Table::new(&COLUMNS);
    for ((source, doc, variant, step, check), value) in values {
        table.push(vec![source.clone(), doc.clone(), variant.clone(), step.clone(), check.clone(), value.clone()]);
    }
    table
}

pub fn from_table(table: &Table) -> Result<BTreeMap<Key, String>, String> {
    if table.header != COLUMNS {
        return Err(format!("a baseline has the columns {}", COLUMNS.join(" ")));
    }
    Ok(table
        .rows
        .iter()
        .map(|row| {
            let at = |c: &str| table.get(row, c).to_string();
            ((at("source"), at("doc"), at("variant"), at("step"), at("check")), at("value"))
        })
        .collect())
}

pub struct BaselineArgs {
    // id and the folder of run1 of that platform; its run2 is read next to
    // it only to name its flaky steps.
    pub platforms: Vec<(String, PathBuf)>,
    // The committed baselines (tests/corpus/baseline).
    pub baseline: PathBuf,
    // Where the candidates and baseline-diff.md go.
    pub out: PathBuf,
    pub known: Vec<KnownEntry>,
    // A platform without a committed baseline passes (until the first one is
    // committed).
    pub missing_ok: bool,
}

const MD_ROWS: usize = 200;

fn line(key: &Key) -> String {
    let (source, doc, variant, step, check) = key;
    format!("{} {} {} {} {}", clean(source), clean(doc), clean(variant), clean(step), clean(check))
}

fn short(value: &str) -> String {
    let cut: String = value.chars().take(70).collect();
    clean(&if cut.len() < value.len() { format!("{}...", cut) } else { cut })
}

fn flaky_lines(id: &str, run: &str, flaky: &BTreeMap<StepKey, String>) -> Vec<String> {
    flaky
        .iter()
        .map(|((doc, variant, step), what)| format!("- flaky on {} in {}: {} {} {}: {}\n", id, run, clean(doc), clean(variant), clean(step), clean(what)))
        .collect()
}

// Returns how many platforms do not match their committed baseline.
pub fn baseline(args: &BaselineArgs) -> Result<usize, Infra> {
    let candidates = args.out.join("candidates");
    std::fs::create_dir_all(&candidates).map_err(|e| Infra(format!("cannot create {}: {}", candidates.display(), e)))?;
    let mut md = String::from("## Baselines\n\n");
    let mut failing = 0;
    for (id, dir) in &args.platforms {
        md.push_str(&format!("### {}\n\n", id));
        let platform = match matrix::load(id, dir) {
            Ok(platform) => platform,
            Err(reason) => {
                failing += 1;
                md.push_str(&format!("No results to check: {}\n\n", clean(&reason)));
                continue;
            }
        };
        // The flaky steps of both runs are named; run2 is only read for that.
        md.extend(flaky_lines(id, "run1", &platform.flaky));
        let run2 = dir.parent().map(|p| p.join("run2").join("report.tsv"));
        if let Some(table) = run2.filter(|_| dir.file_name().is_some_and(|n| n == "run1")).and_then(|p| Table::read(&p).ok()) {
            md.extend(flaky_lines(id, "run2", &compare::flaky_steps(&table)));
        }
        let now = pinned(&platform, &args.known);
        // A run with a flaky step lacks that step's values: it is no baseline.
        if platform.flaky.is_empty() {
            let path = candidates.join(format!("{}.tsv", id));
            to_table(&now).write(&path).map_err(Infra)?;
        } else {
            md.push_str("No candidate written: run1 has a flaky step.\n");
        }
        let committed = args.baseline.join(format!("{}.tsv", id));
        if !committed.is_file() {
            if !args.missing_ok {
                failing += 1;
            }
            md.push_str(&format!("\nNo committed baseline ({}).\n\n", if args.missing_ok { "allowed for now" } else { "fails" }));
            continue;
        }
        let old = Table::read(&committed).and_then(|t| from_table(&t)).map_err(|e| Infra(format!("{}: {}", committed.display(), e)))?;
        let found = diff(&old, &now, &platform.flaky);
        if found.is_empty() {
            md.push_str(&format!("\nMatches its baseline ({} values).\n\n", old.len()));
            continue;
        }
        failing += 1;
        md.push_str(&format!(
            "\nDiffers from its baseline: {} added, {} removed, {} changed.\n\n| Change | Value | Baseline | Now |\n|---|---|---|---|\n",
            found.added.len(),
            found.removed.len(),
            found.changed.len()
        ));
        let rows = found
            .changed
            .iter()
            .map(|(k, old, new)| format!("| changed | {} | {} | {} |\n", line(k), short(old), short(new)))
            .chain(found.added.iter().map(|(k, v)| format!("| added | {} | | {} |\n", line(k), short(v))))
            .chain(found.removed.iter().map(|(k, v)| format!("| removed | {} | {} | |\n", line(k), short(v))));
        let rows: Vec<String> = rows.collect();
        for row in rows.iter().take(MD_ROWS) {
            md.push_str(row);
        }
        if rows.len() > MD_ROWS {
            md.push_str(&format!("\n{} more changes in the candidate's diff.\n", rows.len() - MD_ROWS));
        }
        md.push('\n');
    }
    let path = args.out.join("baseline-diff.md");
    std::fs::write(&path, md).map_err(|e| Infra(format!("cannot write {}: {}", path.display(), e)))?;
    Ok(failing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("corpus-baseline-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MANIFEST: &str = "file\tclass\tsteps\texpect\tvariants\tsave_as\tnote\n\
a.docx\tcomparable\topen,save,pdf\tHello\t\t\t\n\
b.csv\tcomparable\topen,save\t\t\t\t\n\
s.docx\tsystem-fonts\topen,pdf\t\t\t\t\n";

    // One platform's run1, with the values a test then changes.
    fn write_run(root: &Path, id: &str, pdf_text: &str, csv_size: &str, extra_report: &str) -> PathBuf {
        let dir = root.join(id).join("run1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("env.tsv"),
            "key\tvalue\nhost.arch\tx86_64\nexe\t/w/euro-office-lite\npdf_x2t.version\tVersion: 9.4.0.129\n\
pdf_x2t.sha256\tdeb8\nfonts.check\tok\nfonts.total\t163\nallfonts.sha256\ta4b2\ncorpus.git\tabc\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("run.tsv"),
            format!(
                "doc\tvariant\tstep\tstatus\texit\tsignal\tsize\tsha256\toutput\tms\tmessage\n\
a.docx\tplain\topen\tok\t0\t\t10\tbin\t\t5\t\n\
a.docx\tplain\tpdf\tok\t0\t\t900\tpdf\tfiles/a.docx/plain/export.pdf\t5\t\n\
b.csv\tplain\topen\tok\t0\t\t{}\tcsvbin\t\t5\t\n\
s.docx\tplain\tpdf\tok\t0\t\t900\tpdf\tfiles/s.docx/plain/export.pdf\t5\t\n",
                csv_size
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("report.tsv"),
            format!(
                "doc\tvariant\tstep\tcheck\tvalue\n\
a.docx\tplain\tpdf\ttext.sha256\t{}\na.docx\tplain\tpdf\tclass\tcomparable\n\
s.docx\tplain\tpdf\ttext.sha256\ts1\ns.docx\tplain\tpdf\tclass\tsystem-fonts\n{}",
                pdf_text, extra_report
            ),
        )
        .unwrap();
        std::fs::write(dir.join("manifest.tsv"), MANIFEST).unwrap();
        dir
    }

    fn known(rows: &str) -> Vec<KnownEntry> {
        crate::known::parse(&format!("{}\n{}", crate::known::COLUMNS.join("\t"), rows)).unwrap()
    }

    fn checks(values: &BTreeMap<Key, String>) -> Vec<String> {
        values.keys().map(|(s, d, _, st, c)| format!("{} {} {} {}", s, d, st, c)).collect()
    }

    #[test]
    fn a_baseline_pins_the_x2t_and_the_outputs_drawn_with_bundled_fonts() {
        let root = scratch("pinned");
        let dir = write_run(&root, "win-x64", "t1", "1600", "");
        let platform = matrix::load("win-x64", &dir).unwrap();
        let values = checks(&pinned(&platform, &[]));
        // The x2t and the font check stay; the image's font list and the
        // labels go.
        assert!(values.contains(&"env   pdf_x2t.sha256".to_string()), "{:?}", values);
        assert!(values.contains(&"env   fonts.check".to_string()));
        for gone in ["env   fonts.total", "env   allfonts.sha256", "env   corpus.git", "env   exe"] {
            assert!(!values.contains(&gone.to_string()), "{}", gone);
        }
        assert!(values.contains(&"report a.docx pdf text.sha256".to_string()));
        // A system-fonts document keeps only how x2t ended.
        assert!(values.contains(&"run s.docx pdf status".to_string()));
        assert!(!values.contains(&"report s.docx pdf text.sha256".to_string()));
        assert!(!values.contains(&"run s.docx pdf size".to_string()));

        // A known env entry for this platform drops what depends on the
        // runner; an x2t entry pins it as usual.
        let env_entry = known("a.docx\tplain\tpdf\ttext.sha256\twin-x64\tenv\tlayout from the system fonts\t\n");
        assert!(!checks(&pinned(&platform, &env_entry)).contains(&"report a.docx pdf text.sha256".to_string()));
        let x2t_entry = known("a.docx\tplain\tpdf\ttext.sha256\tall\tx2t\tthe 9.4 layout\t\n");
        assert!(checks(&pinned(&platform, &x2t_entry)).contains(&"report a.docx pdf text.sha256".to_string()));
        let elsewhere = known("a.docx\tplain\tpdf\ttext.sha256\tlinux-x64\tenv\tnot this platform\t\n");
        assert!(checks(&pinned(&platform, &elsewhere)).contains(&"report a.docx pdf text.sha256".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }

    // The same run as a mac and as a Windows platform: only the mac one
    // leaves out the re-signed x2t hashes, and both keep the versions.
    fn run_with_sidecar(root: &Path, id: &str) -> Loaded {
        let dir = write_run(root, id, "t1", "1600", "");
        let env = std::fs::read_to_string(dir.join("env.tsv")).unwrap();
        std::fs::write(
            dir.join("env.tsv"),
            format!("{}sidecar.version\tVersion: 9.4.0.129\nsidecar.sha256\t5a1e\nx2t_same_file\tyes\n", env),
        )
        .unwrap();
        matrix::load(id, &dir).unwrap()
    }

    #[test]
    fn a_mac_baseline_leaves_out_the_re_signed_x2t_hashes() {
        let root = scratch("mac-resigned");
        let values = checks(&pinned(&run_with_sidecar(&root, "mac-x64"), &[]));
        for kept in ["env   sidecar.version", "env   pdf_x2t.version", "env   x2t_same_file"] {
            assert!(values.contains(&kept.to_string()), "{} {:?}", kept, values);
        }
        for gone in ["env   sidecar.sha256", "env   pdf_x2t.sha256"] {
            assert!(!values.contains(&gone.to_string()), "{} {:?}", gone, values);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_windows_baseline_still_pins_the_x2t_hashes() {
        let root = scratch("win-hashes");
        let values = checks(&pinned(&run_with_sidecar(&root, "win-x64"), &[]));
        for kept in ["env   sidecar.sha256", "env   pdf_x2t.sha256", "env   sidecar.version"] {
            assert!(values.contains(&kept.to_string()), "{} {:?}", kept, values);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_changed_added_or_removed_value_is_a_difference() {
        let key = |check: &str| -> Key { ("report".into(), "a.docx".into(), "plain".into(), "pdf".into(), check.into()) };
        let mut old = BTreeMap::new();
        old.insert(key("text.sha256"), "t1".to_string());
        old.insert(key("pages"), "1".to_string());
        let mut now = old.clone();
        assert!(diff(&old, &now, &BTreeMap::new()).is_empty());
        now.insert(key("text.sha256"), "t2".into());
        now.remove(&key("pages"));
        now.insert(key("fonts"), "Carlito".into());
        let found = diff(&old, &now, &BTreeMap::new());
        assert_eq!(found.changed, vec![(key("text.sha256"), "t1".into(), "t2".into())]);
        assert_eq!(found.removed, vec![(key("pages"), "1".into())]);
        assert_eq!(found.added, vec![(key("fonts"), "Carlito".into())]);
        // A flaky step is left out on both sides.
        let mut flaky = BTreeMap::new();
        flaky.insert(("a.docx".to_string(), "plain".to_string(), "pdf".to_string()), "signal 6".to_string());
        assert!(diff(&old, &now, &flaky).is_empty());
    }

    // The Editor.bin size of a CSV open changes a little between identical
    // runs: within 1% it matches, as in compare.
    #[test]
    fn a_csv_open_size_matches_within_one_percent() {
        let key: Key = ("run".into(), "b.csv".into(), "plain".into(), "open".into(), "size".into());
        let mut old = BTreeMap::new();
        old.insert(key.clone(), "1600".to_string());
        let mut now = BTreeMap::new();
        now.insert(key.clone(), "1612".to_string());
        assert!(diff(&old, &now, &BTreeMap::new()).is_empty());
        now.insert(key, "1700".to_string());
        assert_eq!(diff(&old, &now, &BTreeMap::new()).changed.len(), 1);
    }

    fn args(root: &Path, platforms: Vec<(String, PathBuf)>, missing_ok: bool) -> BaselineArgs {
        BaselineArgs { platforms, baseline: root.join("committed"), out: root.join("out"), known: Vec::new(), missing_ok }
    }

    #[test]
    fn a_run_writes_its_candidate_and_checks_the_committed_baseline() {
        let root = scratch("command");
        let dir = write_run(&root, "linux-arm64", "t1", "1600", "");
        let platforms = vec![("linux-arm64".to_string(), dir.clone())];

        // No baseline yet: fails unless allowed, and writes the candidate.
        assert_eq!(baseline(&args(&root, platforms.clone(), false)).unwrap(), 1);
        assert_eq!(baseline(&args(&root, platforms.clone(), true)).unwrap(), 0);
        let candidate = root.join("out/candidates/linux-arm64.tsv");
        assert!(candidate.is_file());
        let md = std::fs::read_to_string(root.join("out/baseline-diff.md")).unwrap();
        assert!(md.contains("No committed baseline (allowed for now)"), "{}", md);

        // Committing the candidate makes the same run match.
        std::fs::create_dir_all(root.join("committed")).unwrap();
        std::fs::copy(&candidate, root.join("committed/linux-arm64.tsv")).unwrap();
        assert_eq!(baseline(&args(&root, platforms.clone(), false)).unwrap(), 0);

        // A difference injected on purpose fails it and says what changed.
        let changed = write_run(&root, "linux-arm64", "t9", "1600", "");
        assert_eq!(baseline(&args(&root, vec![("linux-arm64".to_string(), changed)], false)).unwrap(), 1);
        let md = std::fs::read_to_string(root.join("out/baseline-diff.md")).unwrap();
        assert!(md.contains("| changed | report a.docx plain pdf text.sha256 | t1 | t9 |"), "{}", md);
        let _ = std::fs::remove_dir_all(&root);
    }

    // A flaky step is named with its run, platform and signal, is not
    // compared, and keeps a candidate from being written.
    #[test]
    fn a_flaky_step_is_named_and_not_compared() {
        let root = scratch("flaky");
        let dir = write_run(&root, "linux-arm64", "t1", "1600", "");
        let platforms = vec![("linux-arm64".to_string(), dir)];
        baseline(&args(&root, platforms, true)).unwrap();
        std::fs::create_dir_all(root.join("committed")).unwrap();
        std::fs::copy(root.join("out/candidates/linux-arm64.tsv"), root.join("committed/linux-arm64.tsv")).unwrap();
        std::fs::remove_dir_all(root.join("out")).unwrap();

        let flaky_row = "a.docx\tplain\tpdf\tflaky\tsignal 6 on linux-arm64 in run1 (the other run: ok)\n";
        let crashed = write_run(&root, "linux-arm64", "", "1600", flaky_row);
        assert_eq!(baseline(&args(&root, vec![("linux-arm64".to_string(), crashed)], false)).unwrap(), 0);
        let md = std::fs::read_to_string(root.join("out/baseline-diff.md")).unwrap();
        assert!(md.contains("- flaky on linux-arm64 in run1: a.docx plain pdf: signal 6 on linux-arm64 in run1"), "{}", md);
        assert!(md.contains("No candidate written: run1 has a flaky step."), "{}", md);
        assert!(!root.join("out/candidates/linux-arm64.tsv").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_platform_without_results_does_not_match() {
        let root = scratch("none");
        let platforms = vec![("win-x64".to_string(), root.join("win-x64/run1"))];
        assert_eq!(baseline(&args(&root, platforms, true)).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    // The committed baselines the workflow checks: one per platform of its
    // table, each one readable.
    #[test]
    fn the_committed_baselines_are_readable() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/baseline");
        for id in crate::known::PLATFORM_IDS {
            let table = Table::read(&dir.join(format!("{}.tsv", id))).unwrap();
            assert_eq!(table.header, COLUMNS, "{}", id);
            let values = from_table(&table).unwrap();
            assert!(!values.is_empty(), "{}", id);
            assert_eq!(values.len(), table.rows.len(), "{}: a key twice", id);
        }
    }
}
