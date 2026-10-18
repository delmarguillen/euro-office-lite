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

mod layout;

const USAGE: &str = "\
usage:
  corpus run --install-root <dir> --corpus <dir> --out <dir> [--work <dir>] [--only <doc>] [--label <k=v>]
  corpus report --out <dir>
  corpus compare <out_a> <out_b> [--summary <file>]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => println!("{}", USAGE),
        _ => {
            eprintln!("{}", USAGE);
            std::process::exit(2);
        }
    }
}
