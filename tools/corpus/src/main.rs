// Conversion corpus: converts tests/corpus with the x2t of an installed build,
// through the same functions the app calls, and reports per document and step.

// The app's own x2t code, compiled into this crate unchanged: every conversion
// here goes through the functions the app calls.
#[path = "../../../src-tauri/src/x2t_task.rs"]
#[allow(dead_code)]
mod x2t_task;
#[path = "../../../src-tauri/src/text_import.rs"]
#[allow(dead_code)]
mod text_import;

mod allfonts;
mod arch;
mod compare;
mod known;
mod layout;
mod manifest;
mod markdown;
mod matrix;
mod normalize;
mod pdf;
mod report;
mod run;
mod sha256;
mod tsv;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
usage:
  corpus run --install-root <dir> --corpus <dir> --out <dir> [--work <dir>] [--only <doc>] [--label <k=v>]...
  corpus report --out <dir> --manifest <trusted manifest.tsv> [--known <known.tsv> --platform <id> --other <other run dir>]
  corpus compare <out_a> <out_b> [--summary <file>]
  corpus matrix --platform <id>=<run dir>... [--expect <id,id,...>] [--known <known.tsv>] --out <dir>";

// Positional arguments and --flag value pairs; --label may repeat.
struct Args {
    positional: Vec<String>,
    flags: HashMap<String, Vec<String>>,
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, String> {
        let mut positional = Vec::new();
        let mut flags: HashMap<String, Vec<String>> = HashMap::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if let Some(name) = arg.strip_prefix("--") {
                let value = iter.next().ok_or_else(|| format!("--{} needs a value", name))?;
                flags.entry(name.to_string()).or_default().push(value.clone());
            } else {
                positional.push(arg.clone());
            }
        }
        Ok(Args { positional, flags })
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.flags.get(name).and_then(|values| values.last()).map(String::as_str)
    }

    fn required(&self, name: &str) -> Result<PathBuf, String> {
        self.one(name)
            .map(PathBuf::from)
            .ok_or_else(|| format!("--{} is required", name))
    }

    fn only_known(&self, known: &[&str]) -> Result<(), String> {
        match self.flags.keys().find(|name| !known.contains(&name.as_str())) {
            Some(name) => Err(format!("unknown option --{}", name)),
            None => Ok(()),
        }
    }
}

fn run_args(args: &Args) -> Result<run::RunArgs, String> {
    args.only_known(&["install-root", "corpus", "out", "work", "only", "label"])?;
    let out = args.required("out")?;
    let labels = args
        .flags
        .get("label")
        .into_iter()
        .flatten()
        .map(|label| {
            label
                .split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| format!("--label {} is not key=value", label))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(run::RunArgs {
        install_root: args.required("install-root")?,
        corpus: args.required("corpus")?,
        work: args.one("work").map(PathBuf::from).unwrap_or_else(|| out.join("work")),
        out,
        only: args.one("only").map(str::to_string),
        labels,
    })
}

type ReportArgs =(PathBuf, PathBuf, Vec<known::KnownEntry>, Option<String>, Option<PathBuf>);

// --known needs --platform and --other: report settles a crash in one run
// against the other run and the platform's entries.
fn report_args(args: &Args) -> Result<ReportArgs, String> {
    args.only_known(&["out", "manifest", "known", "platform", "other"])?;
    if args.one("known").is_some() && (args.one("platform").is_none() || args.one("other").is_none()) {
        return Err("--known needs --platform and --other".to_string());
    }
    let known = args.one("known").map(|path| known::read(Path::new(path))).transpose()?.unwrap_or_default();
    let platform = args.one("platform").map(str::to_string);
    if let Some(id) = platform.as_deref().filter(|id| !matrix::valid_id(id)) {
        return Err(format!("--platform {:?} is not a lowercase platform id", id));
    }
    let other = args.one("other").map(PathBuf::from);
    Ok((args.required("out")?, args.required("manifest")?, known, platform, other))
}

fn matrix_args(args: &Args) -> Result<matrix::MatrixArgs, String> {
    args.only_known(&["platform", "expect", "out", "known"])?;
    if !args.positional.is_empty() {
        return Err("matrix takes no positional arguments".to_string());
    }
    let mut platforms: Vec<(String, PathBuf)> = Vec::new();
    for value in args.flags.get("platform").into_iter().flatten() {
        let (id, dir) = value
            .split_once('=')
            .filter(|(id, dir)| matrix::valid_id(id) && !dir.is_empty())
            .ok_or_else(|| format!("--platform {} is not <id>=<dir> with a lowercase id", value))?;
        if platforms.iter().any(|(p, _)| p == id) {
            return Err(format!("--platform {} is given twice", id));
        }
        platforms.push((id.to_string(), PathBuf::from(dir)));
    }
    let expect: Vec<String> = match args.one("expect") {
        Some(list) => list.split(',').map(str::to_string).collect(),
        None => Vec::new(),
    };
    if let Some(bad) = expect.iter().find(|id| !matrix::valid_id(id)) {
        return Err(format!("--expect holds a bad platform id: {:?}", bad));
    }
    if platforms.is_empty() && expect.is_empty() {
        return Err("matrix needs --platform or --expect".to_string());
    }
    // A known.tsv that cannot be read or is malformed is the run's problem,
    // not a usage error, but it stops the command the same way (exit 2).
    let known = args.one("known").map(|path| known::read(Path::new(path))).transpose()?;
    Ok(matrix::MatrixArgs { platforms, expect, out: args.required("out")?, known })
}

fn usage_error(message: &str) -> ! {
    eprintln!("corpus: {}\n{}", message, USAGE);
    std::process::exit(2);
}

fn main() {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = all.first().cloned() else {
        usage_error("no command");
    };
    let args = Args::parse(&all[1..]).unwrap_or_else(|e| usage_error(&e));
    let code = match command.as_str() {
        "run" => {
            let run_args = run_args(&args).unwrap_or_else(|e| usage_error(&e));
            match run::run(&run_args) {
                Ok(()) => 0,
                Err(run::Infra(message)) => {
                    eprintln!("corpus run: {}", message);
                    2
                }
            }
        }
        "report" => {
            let (out, manifest, known, platform, other) = report_args(&args).unwrap_or_else(|e| usage_error(&e));
            let options = report::Options { known: &known, platform: platform.as_deref(), other: other.as_deref() };
            match report::report(&out, &manifest, &options) {
                Ok(failures) if failures.is_empty() => 0,
                Ok(failures) => {
                    for failure in &failures {
                        eprintln!("FAIL {}", failure);
                    }
                    1
                }
                Err(run::Infra(message)) => {
                    eprintln!("corpus report: {}", message);
                    2
                }
            }
        }
        "compare" => {
            args.only_known(&["summary"]).unwrap_or_else(|e| usage_error(&e));
            let [a, b] = args.positional.as_slice() else {
                usage_error("compare takes two result folders");
            };
            let summary = args.one("summary").map(PathBuf::from);
            match compare::compare(Path::new(a), Path::new(b), summary.as_deref()) {
                Ok(0) => 0,
                Ok(count) => {
                    eprintln!("corpus compare: {} values differ or are missing", count);
                    1
                }
                Err(run::Infra(message)) => {
                    eprintln!("corpus compare: {}", message);
                    2
                }
            }
        }
        "matrix" => {
            let matrix_args = matrix_args(&args).unwrap_or_else(|e| usage_error(&e));
            match matrix::matrix(&matrix_args) {
                Ok(outcome) if outcome == matrix::Outcome::default() => 0,
                Ok(outcome) => {
                    if outcome.infra > 0 {
                        eprintln!("corpus matrix: {} expected platforms have no results", outcome.infra);
                    }
                    if outcome.unexplained > 0 {
                        eprintln!("corpus matrix: {} rows differ within a family, miss a value or fail, with no known entry", outcome.unexplained);
                    }
                    1
                }
                Err(run::Infra(message)) => {
                    eprintln!("corpus matrix: {}", message);
                    2
                }
            }
        }
        "-h" | "--help" => {
            println!("{}", USAGE);
            0
        }
        other => usage_error(&format!("unknown command {}", other)),
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn run_options_are_read_and_unknown_ones_refused() {
        let args = Args::parse(&strings(&[
            "--install-root", "/r", "--corpus", "c", "--out", "o", "--label", "git=abc", "--label", "x=y=z",
        ]))
        .unwrap();
        let run = run_args(&args).unwrap();
        assert_eq!(run.work, PathBuf::from("o").join("work"));
        assert_eq!(run.labels, vec![("git".into(), "abc".into()), ("x".into(), "y=z".into())]);

        let args = Args::parse(&strings(&["--out", "o", "--bogus", "1"])).unwrap();
        assert!(run_args(&args).is_err());
        assert!(Args::parse(&strings(&["--out"])).is_err());
    }

    #[test]
    fn report_options_are_checked() {
        let parse = |a: &[&str]| report_args(&Args::parse(&strings(a)).unwrap());
        let (out, manifest, known, platform, other) =
            parse(&["--out", "r/run1", "--manifest", "m.tsv", "--platform", "linux-arm64", "--other", "r/run2"]).unwrap();
        assert_eq!((out, manifest), (PathBuf::from("r/run1"), PathBuf::from("m.tsv")));
        assert!(known.is_empty());
        assert_eq!((platform.as_deref(), other), (Some("linux-arm64"), Some(PathBuf::from("r/run2"))));
        for bad in [
            &["--out", "o", "--manifest", "m", "--platform", "Linux"][..],
            &["--out", "o", "--manifest", "m", "--known", "no/such/known.tsv"],
            &["--out", "o", "--manifest", "m", "--bogus", "1"],
            &["--out", "o"],
        ] {
            assert!(parse(bad).is_err(), "{:?}", bad);
        }
    }

    // Without the platform and the other run, known.tsv could settle nothing
    // on purpose: --known alone is a usage error.
    #[test]
    fn report_known_needs_the_platform_and_the_other_run() {
        let dir = std::env::temp_dir().join(format!("corpus-main-known-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("known.tsv");
        std::fs::write(&path, format!("{}\n", known::COLUMNS.join("\t"))).unwrap();
        let known = path.to_string_lossy().to_string();
        let parse = |a: &[&str]| report_args(&Args::parse(&strings(a)).unwrap());
        let base = ["--out", "r/run1", "--manifest", "m.tsv", "--known", known.as_str()];
        let results: Vec<bool> = [
            &[][..],
            &["--platform", "linux-arm64"],
            &["--other", "r/run2"],
            &["--platform", "linux-arm64", "--other", "r/run2"],
        ]
        .iter()
        .map(|extra| parse(&[&base[..], extra].concat()).is_ok())
        .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(results, vec![false, false, false, true]);
    }

    #[test]
    fn matrix_options_are_checked() {
        let parse = |a: &[&str]| matrix_args(&Args::parse(&strings(a)).unwrap());
        let ok = parse(&["--platform", "win-x64=r/win/run1", "--platform", "mac-arm64=r/mac/run1", "--expect", "win-x64,mac-arm64", "--out", "m"]).unwrap();
        assert_eq!(ok.platforms, vec![("win-x64".to_string(), PathBuf::from("r/win/run1")), ("mac-arm64".to_string(), PathBuf::from("r/mac/run1"))]);
        assert_eq!(ok.expect, vec!["win-x64", "mac-arm64"]);
        for bad in [
            &["--platform", "win-x64", "--out", "m"][..],
            &["--platform", "=dir", "--out", "m"],
            &["--platform", "Win=dir", "--out", "m"],
            &["--platform", "a/b=dir", "--out", "m"],
            &["--platform", "a=", "--out", "m"],
            &["--platform", "a=x", "--platform", "a=y", "--out", "m"],
            &["--platform", "a=x", "--expect", "a,,b", "--out", "m"],
            &["--platform", "a=x"],
            &["--out", "m"],
            &["--platform", "a=x", "--out", "m", "extra"],
            &["--platform", "a=x", "--out", "m", "--bogus", "1"],
            &["--platform", "a=x", "--out", "m", "--known", "no/such/known.tsv"],
        ] {
            assert!(parse(bad).is_err(), "{:?}", bad);
        }
    }
}
