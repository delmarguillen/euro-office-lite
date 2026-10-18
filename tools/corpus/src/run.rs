// `corpus run`: converts the corpus with an installed build's x2t, through the
// functions the app calls, and writes what happened to the --out folder.

use crate::layout::{self, Layout, Os};
use crate::tsv::Table;
use crate::{allfonts, sha256, x2t_task};
use std::path::{Path, PathBuf};

pub struct RunArgs {
    pub install_root: PathBuf,
    pub corpus: PathBuf,
    pub out: PathBuf,
    pub work: PathBuf,
    pub only: Option<String>,
    pub labels: Vec<(String, String)>,
}

// Appends every line the app's code logs to out/log.txt, with a prefix that
// says which phase or document it belongs to.
pub struct Log {
    file: std::cell::RefCell<std::fs::File>,
}

impl Log {
    pub fn create(path: &Path) -> Result<Log, String> {
        std::fs::File::create(path)
            .map(|file| Log { file: std::cell::RefCell::new(file) })
            .map_err(|e| format!("cannot create {}: {}", path.display(), e))
    }

    pub fn line(&self, prefix: &str, line: &str) {
        use std::io::Write;
        let _ = writeln!(self.file.borrow_mut(), "{} {}", prefix, line);
    }
}

// An infrastructure failure: the install or the font cache is not usable, so
// no conversion result would mean anything. The binary exits 2.
pub struct Infra(pub String);

pub struct X2tInfo {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub version: String,
}

fn x2t_info(path: &Path, binaries_dir: &Path) -> Result<X2tInfo, Infra> {
    let size = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| Infra(format!("cannot read {}: {}", path.display(), e)))?;
    let sha256 = sha256::file_hex(path).map_err(|e| Infra(format!("cannot hash {}: {}", path.display(), e)))?;
    Ok(X2tInfo {
        path: path.to_path_buf(),
        size,
        sha256,
        version: x2t_version(path, binaries_dir),
    })
}

// x2t run without arguments prints "OOX/binary file converter. Version: ..."
// and exits with an error; the line is all that is kept.
fn x2t_version(x2t: &Path, binaries_dir: &Path) -> String {
    let output = std::process::Command::new(x2t)
        .current_dir(binaries_dir)
        .envs(x2t_task::convert_env(binaries_dir))
        .stdin(std::process::Stdio::null())
        .output();
    match output {
        Ok(output) => version_line(&String::from_utf8_lossy(&output.stdout))
            .or_else(|| version_line(&String::from_utf8_lossy(&output.stderr)))
            .unwrap_or_else(|| "unknown".to_string()),
        Err(_) => "unknown".to_string(),
    }
}

pub fn version_line(text: &str) -> Option<String> {
    text.lines()
        .find(|line| line.contains("Version:"))
        .map(|line| line.trim().to_string())
}

pub struct Prepared {
    pub layout: Layout,
    pub fontdata: PathBuf,
    pub env: Vec<(String, String)>,
}

fn file_facts(path: &Path) -> Result<(u64, String), Infra> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err(Infra(format!("{} is missing or empty", path.display())));
    }
    let sha = sha256::file_hex(path).map_err(|e| Infra(format!("cannot hash {}: {}", path.display(), e)))?;
    Ok((size, sha))
}

fn copy_into(source: &Path, dir: &Path) -> Result<(), Infra> {
    std::fs::create_dir_all(dir).map_err(|e| Infra(format!("cannot create {}: {}", dir.display(), e)))?;
    let name = source.file_name().unwrap_or_default();
    std::fs::copy(source, dir.join(name))
        .map(|_| ())
        .map_err(|e| Infra(format!("cannot copy {}: {}", source.display(), e)))
}

// Once per run: the install layout, both x2t, and the font cache built by the
// app's own scan, with the list of fonts it found.
pub fn prepare(args: &RunArgs, log: &Log) -> Result<Prepared, Infra> {
    let os = Os::current();
    let exe = layout::expected_exe(os, &args.install_root).map_err(Infra)?;
    let layout = layout::layout_from_exe(os, &exe).map_err(Infra)?;

    let sidecar = x2t_info(&layout.sidecar, &layout.binaries_dir)?;
    let pdf_x2t_path = x2t_task::find_x2t(&layout.binaries_dir, Some(&layout.exe_dir_plain))
        .map_err(|e| Infra(format!("{} under {}", e, layout.binaries_dir.display())))?;
    let pdf_x2t = x2t_info(&pdf_x2t_path, &layout.binaries_dir)?;

    let fontdata = args.work.join("fontdata");
    let _ = std::fs::remove_dir_all(&fontdata);
    std::fs::create_dir_all(&fontdata)
        .map_err(|e| Infra(format!("cannot create {}: {}", fontdata.display(), e)))?;
    let generated = x2t_task::generate_fonts(&fontdata, &layout.binaries_dir, Some(&layout.exe_dir_plain), &|line| {
        log.line("[FONTS]", line)
    });
    if !generated {
        return Err(Infra("the font scan failed (see log.txt)".to_string()));
    }
    let allfonts_path = fontdata.join("AllFonts.js");
    let selection_path = fontdata.join("font_selection.bin");
    let (allfonts_size, allfonts_sha) = file_facts(&allfonts_path)?;
    let (selection_size, selection_sha) = file_facts(&selection_path)?;
    let allfonts_text = std::fs::read_to_string(&allfonts_path)
        .map_err(|e| Infra(format!("cannot read {}: {}", allfonts_path.display(), e)))?;
    if os == Os::Windows {
        let selection = std::fs::read(&selection_path).unwrap_or_default();
        let lossy = String::from_utf8_lossy(&selection);
        for text in [allfonts_text.as_str(), lossy.as_ref()] {
            if text.contains("\"/?/") || text.contains(r"\\?\") {
                return Err(Infra("the font cache lists paths with the extended prefix".to_string()));
            }
        }
    }
    let fonts_out = args.out.join("fonts");
    copy_into(&allfonts_path, &fonts_out)?;
    copy_into(&selection_path, &fonts_out)?;

    let list = allfonts::parse(&allfonts_text).map_err(|e| Infra(format!("AllFonts.js: {}", e)))?;
    let binaries = layout.binaries_dir.to_string_lossy().to_string();
    let mut fonts = Table::new(&["index", "path", "origin"]);
    for (index, path) in list.files.iter().enumerate() {
        fonts.push(vec![index.to_string(), path.clone(), allfonts::origin(path, &binaries).to_string()]);
    }
    fonts.write(&args.out.join("fonts.tsv")).map_err(Infra)?;
    let checks = allfonts::check(&list, &binaries);
    let mut check_table = Table::new(&["family", "style", "path", "origin"]);
    for row in &checks {
        check_table.push(vec![row.family.clone(), row.style.clone(), row.path.clone(), row.origin.clone()]);
    }
    check_table.write(&args.out.join("fonts-check.tsv")).map_err(Infra)?;
    let bundled = fonts.rows.iter().filter(|row| row[2] == "bundled").count();

    let path = |p: &Path| p.to_string_lossy().to_string();
    let mut env: Vec<(String, String)> = vec![
        ("os".into(), std::env::consts::OS.into()),
        ("arch".into(), std::env::consts::ARCH.into()),
        ("install_root".into(), path(&args.install_root)),
        ("exe".into(), path(&layout.exe)),
        ("resource_dir".into(), path(&layout.resource_dir)),
        ("binaries_dir".into(), path(&layout.binaries_dir)),
    ];
    for (name, info) in [("sidecar", &sidecar), ("pdf_x2t", &pdf_x2t)] {
        env.push((format!("{}.path", name), path(&info.path)));
        env.push((format!("{}.size", name), info.size.to_string()));
        env.push((format!("{}.sha256", name), info.sha256.clone()));
        env.push((format!("{}.version", name), info.version.clone()));
    }
    let same = if sidecar.sha256 == pdf_x2t.sha256 { "yes" } else { "no" };
    env.push(("x2t_same_file".into(), same.into()));
    env.push(("allfonts.size".into(), allfonts_size.to_string()));
    env.push(("allfonts.sha256".into(), allfonts_sha));
    env.push(("font_selection.size".into(), selection_size.to_string()));
    env.push(("font_selection.sha256".into(), selection_sha));
    env.push(("fonts.total".into(), list.files.len().to_string()));
    env.push(("fonts.bundled".into(), bundled.to_string()));
    env.push(("fonts.system".into(), (list.files.len() - bundled).to_string()));
    env.push(("fonts.check".into(), allfonts::check_summary(&checks)));
    for var in ["LANG", "LC_ALL", "TZ"] {
        env.push((var.into(), std::env::var(var).unwrap_or_default()));
    }
    for (key, value) in &args.labels {
        env.push((format!("corpus.{}", key), value.clone()));
    }
    Ok(Prepared { layout, fontdata, env })
}

pub fn write_env(out: &Path, env: &[(String, String)]) -> Result<(), String> {
    let mut table = Table::new(&["key", "value"]);
    for (key, value) in env {
        table.push(vec![key.clone(), value.clone()]);
    }
    table.write(&out.join("env.tsv"))
}

pub fn run(args: &RunArgs) -> Result<(), Infra> {
    std::fs::create_dir_all(&args.out).map_err(|e| Infra(format!("cannot create {}: {}", args.out.display(), e)))?;
    std::fs::create_dir_all(&args.work).map_err(|e| Infra(format!("cannot create {}: {}", args.work.display(), e)))?;
    let log = Log::create(&args.out.join("log.txt")).map_err(Infra)?;
    let prepared = prepare(args, &log)?;
    write_env(&args.out, &prepared.env).map_err(Infra)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line_is_found_or_left_unknown() {
        let usage = "\nOOX/binary file converter. Version: 8.2.0.147\nUsage: x2t ...\n";
        assert_eq!(version_line(usage), Some("OOX/binary file converter. Version: 8.2.0.147".into()));
        assert_eq!(version_line("Usage: x2t\n"), None);
    }

    #[test]
    fn an_install_without_x2t_is_an_infrastructure_failure() {
        let root = std::env::temp_dir().join(format!("corpus-run-noinstall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let args = RunArgs {
            install_root: root.clone(),
            corpus: root.join("corpus"),
            out: root.join("out"),
            work: root.join("work"),
            only: None,
            labels: Vec::new(),
        };
        match run(&args) {
            Err(Infra(message)) => assert!(message.starts_with("executable not found"), "{}", message),
            Ok(()) => panic!("an empty root must not run"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
