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
mod layout;
mod manifest;
mod run;
mod sha256;
mod tsv;

use std::collections::HashMap;
use std::path::PathBuf;

const USAGE: &str = "\
usage:
  corpus run --install-root <dir> --corpus <dir> --out <dir> [--work <dir>] [--only <doc>] [--label <k=v>]...
  corpus report --out <dir>
  corpus compare <out_a> <out_b> [--summary <file>]";

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
}
