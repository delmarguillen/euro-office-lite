// `corpus run`: converts the corpus with an installed build's x2t, through the
// functions the app calls, and writes what happened to the --out folder.

use crate::layout::{self, Layout, Os};
use crate::manifest::{self, Entry, Source};
use crate::tsv::Table;
use crate::{allfonts, sha256, text_import, x2t_task};
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
#[derive(Debug)]
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

    let entries = manifest::read(&args.corpus).map_err(Infra)?;
    // report reads the expected text and the classes from this copy.
    std::fs::copy(args.corpus.join("manifest.tsv"), args.out.join("manifest.tsv"))
        .map_err(|e| Infra(format!("cannot copy the manifest: {}", e)))?;
    let mut table = Table::new(&RUN_COLUMNS);
    for (n, entry) in entries.iter().enumerate() {
        if let Some(only) = &args.only {
            if only != &entry.doc && only != &entry.id() {
                continue;
            }
        }
        for variant in &entry.variants {
            let doc = DocRun {
                args,
                prepared: &prepared,
                log: &log,
                entry,
                variant,
                n: n + 1,
            };
            for row in doc.run() {
                log.line("[RUN]", &format!("{} {} {} {}", entry.doc, variant, row.step, row.status));
                table.push(row.cells(&entry.doc, variant));
            }
        }
    }
    table.write(&args.out.join("run.tsv")).map_err(Infra)?;
    Ok(())
}

pub const RUN_COLUMNS: [&str; 11] = [
    "doc", "variant", "step", "status", "exit", "signal", "size", "sha256", "output", "ms", "message",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepRow {
    pub step: String,
    pub status: String,
    pub exit: String,
    pub signal: String,
    pub size: String,
    pub sha256: String,
    // Relative to --out, with forward slashes.
    pub output: String,
    pub ms: u128,
    pub message: String,
}

impl StepRow {
    fn cells(&self, doc: &str, variant: &str) -> Vec<String> {
        vec![
            doc.to_string(),
            variant.to_string(),
            self.step.clone(),
            self.status.clone(),
            self.exit.clone(),
            self.signal.clone(),
            self.size.clone(),
            self.sha256.clone(),
            self.output.clone(),
            self.ms.to_string(),
            self.message.clone(),
        ]
    }
}

// The steps of one document in the order file_ops runs them: open first,
// then save, each save as, and the PDF export.
pub fn plan(entry: &Entry) -> Vec<String> {
    let mut steps = vec!["open".to_string()];
    if entry.has_step("save") {
        steps.push("save".to_string());
    }
    for ext in &entry.save_as {
        steps.push(format!("save_as.{}", ext));
    }
    if entry.has_step("pdf") {
        steps.push("pdf".to_string());
    }
    steps
}

// Runs `execute` for each planned step. Nothing runs after a failed open,
// since there is no Editor.bin. Only the exit code the manifest names, in the
// step it names, is expected-fail; a signal, a spawn or setup error, another
// code or an empty output stay failures, and a success is unexpected-ok.
pub fn run_steps(entry: &Entry, mut execute: impl FnMut(&str) -> StepRow) -> Vec<StepRow> {
    let mut rows = Vec::new();
    let mut opened = true;
    for step in plan(entry) {
        if !opened {
            rows.push(StepRow {
                step,
                status: "skipped".into(),
                message: "the open failed".into(),
                ..StepRow::default()
            });
            continue;
        }
        let mut row = execute(&step);
        row.step = step.clone();
        if step == "open" && row.status != "ok" {
            opened = false;
        }
        if let Some(failure) = entry.expect_fail.as_ref().filter(|f| f.step == step) {
            if row.status == "ok" {
                row.status = "unexpected-ok".into();
            } else if row.status == "exit" && row.exit == failure.exit.to_string() {
                row.status = "expected-fail".into();
            }
        }
        rows.push(row);
    }
    rows
}

// Windows has no signals: a process that crashes ends with an NTSTATUS
// error code (0xC0000005 for an access violation) as its exit code. The
// reserved bit must be clear, so -1 and other small negatives stay exits.
pub fn windows_crash(code: i32) -> Option<String> {
    let code = code as u32;
    (code & 0xD000_0000 == 0xC000_0000).then(|| format!("0x{:08X}", code))
}

// How an x2t run ended: exit code, signal (or Windows crash code), and
// whether it wrote anything. A crash keeps no exit code, as on Unix.
pub fn classify(success: bool, code: Option<i32>, signal: Option<String>, output_size: u64) -> (String, String, String) {
    let exit = if signal.is_some() { String::new() } else { code.map(|c| c.to_string()).unwrap_or_default() };
    let signal_text = signal.clone().unwrap_or_default();
    let status = if signal.is_some() {
        "signal"
    } else if !success {
        "exit"
    } else if output_size == 0 {
        "empty"
    } else {
        "ok"
    };
    (status.to_string(), exit, signal_text)
}

// convert_to_pdf reports how x2t ended only through its log lines and error
// text, so the outcome is read back from them.
pub fn classify_pdf(result: &Result<String, String>, lines: &[String], output_size: u64) -> (String, String, String) {
    classify_pdf_on(cfg!(windows), result, lines, output_size)
}

// -999 is convert_to_pdf's "no exit code", never a Windows crash code.
fn classify_pdf_on(windows: bool, result: &Result<String, String>, lines: &[String], output_size: u64) -> (String, String, String) {
    let logged = lines
        .iter()
        .find_map(|l| l.strip_prefix("failed exit="))
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<i32>().ok());
    let code = logged.filter(|c| *c != -999);
    let signal = lines
        .iter()
        .find_map(|l| l.strip_prefix("x2t killed by signal "))
        .map(|s| s.trim().to_string())
        .filter(|s| s.parse::<i32>().is_ok())
        .or_else(|| code.filter(|_| windows).and_then(windows_crash));
    match result {
        Ok(_) => classify(true, Some(0), None, output_size),
        Err(message) if message.starts_with("Failed to spawn x2t") => ("spawn".into(), String::new(), String::new()),
        Err(_) if signal.is_some() || logged.is_some() => classify(false, code, signal, output_size),
        // Setup failed before x2t ran: font cache, editor files, work folder.
        Err(_) => ("error".into(), String::new(), String::new()),
    }
}

fn truncate(message: &str) -> String {
    let message = message.trim();
    match message.char_indices().nth(500) {
        Some((i, _)) => format!("{}...", &message[..i]),
        None => message.to_string(),
    }
}

fn relative(out: &Path, path: &Path) -> String {
    path.strip_prefix(out)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn error_row(message: impl std::fmt::Display) -> StepRow {
    StepRow {
        status: "error".into(),
        message: truncate(&message.to_string()),
        ..StepRow::default()
    }
}

struct DocRun<'a> {
    args: &'a RunArgs,
    prepared: &'a Prepared,
    log: &'a Log,
    entry: &'a Entry,
    variant: &'a str,
    n: usize,
}

// Where one document and variant lives while it runs.
struct Paths {
    // The file the app would open: a copy of the fixture, the installed
    // template, or a 0-byte file.
    source: PathBuf,
    // Where a plain Save writes: the opened file, as current_file in the app,
    // or a file next to it for a template (create_new leaves no current file).
    save_target: PathBuf,
    temp_dir: PathBuf,
    out_dir: PathBuf,
}

impl DocRun<'_> {
    fn paths(&self) -> Result<Paths, String> {
        let os = Os::current();
        let work = &self.args.work;
        let in_dir = work.join(format!("in-{}-{}", self.n, self.variant));
        let temp_dir = if self.variant == "special-temp" {
            work.join(layout::special_temp_dir(os)).join(format!("doc-{}", self.n))
        } else {
            work.join(format!("doc-{}-{}", self.n, self.variant))
        };
        let out_dir = self.args.out.join("files").join(self.entry.id()).join(self.variant);
        for dir in [&in_dir, &temp_dir, &out_dir] {
            // A fresh temp_dir per document is what switch_document and
            // clear_document_temp give the app.
            let _ = std::fs::remove_dir_all(dir);
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
        }
        let name = self.entry.file_name().to_string();
        let local = if self.variant == "special-name" {
            let (dir, file) = layout::special_names(os, &self.entry.extension());
            in_dir.join(dir).join(file)
        } else {
            in_dir.join(&name)
        };
        if let Some(parent) = local.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
        }
        let original = out_dir.join(format!("original.{}", self.entry.extension()));
        let (source, save_target) = match &self.entry.source {
            Source::Fixture(file) => {
                let fixture = self.args.corpus.join(file);
                std::fs::copy(&fixture, &local).map_err(|e| format!("cannot copy {}: {}", fixture.display(), e))?;
                std::fs::copy(&fixture, &original).map_err(|e| format!("cannot copy {}: {}", fixture.display(), e))?;
                (local.clone(), local)
            }
            Source::Installed(path) => {
                let installed = self.prepared.layout.resource_dir.join(path);
                std::fs::copy(&installed, &original)
                    .map_err(|e| format!("cannot copy {}: {}", installed.display(), e))?;
                (installed, local)
            }
            Source::Empty(_) => {
                std::fs::write(&local, b"").map_err(|e| format!("cannot create {}: {}", local.display(), e))?;
                (local.clone(), local)
            }
        };
        Ok(Paths {
            source,
            save_target,
            temp_dir,
            out_dir,
        })
    }

    fn run(&self) -> Vec<StepRow> {
        let paths = match self.paths() {
            Ok(paths) => paths,
            Err(message) => return run_steps(self.entry, |_| error_row(&message)),
        };
        let mut text_options = None;
        run_steps(self.entry, |step| {
            let started = std::time::Instant::now();
            let mut row = match step {
                "open" => self.open(&paths, &mut text_options),
                "save" => self.save(&paths, &paths.save_target, "save", text_options),
                "pdf" => self.pdf(&paths),
                other => {
                    let ext = other.trim_start_matches("save_as.");
                    let dest = paths.save_target.with_extension(ext);
                    self.save(&paths, &dest, other, text_options)
                }
            };
            row.ms = started.elapsed().as_millis();
            row
        })
    }

    fn binaries(&self) -> &Path {
        &self.prepared.layout.binaries_dir
    }

    // The sidecar call converter::convert_file makes for every conversion
    // that is not a PDF.
    fn convert(
        &self,
        input: &Path,
        output: &Path,
        format_to: i32,
        temp_dir: &Path,
        options: Option<text_import::TextOptions>,
    ) -> Result<std::process::Output, String> {
        let params = x2t_task::params_path(temp_dir, "convert");
        let xml = x2t_task::convert_params_xml(
            &input.to_string_lossy(),
            &output.to_string_lossy(),
            format_to,
            &temp_dir.to_string_lossy(),
            &options.map(|o| o.xml_elements()).unwrap_or_default(),
        );
        x2t_task::write_file(&params, &xml)?;
        let mut command = std::process::Command::new(&self.prepared.layout.sidecar);
        command
            .current_dir(self.binaries())
            .arg(params.to_string_lossy().as_ref())
            .envs(x2t_task::convert_env(self.binaries()))
            .stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW, as the app's sidecar call.
            command.creation_flags(0x0800_0000);
        }
        let result = command.output().map_err(|e| format!("Failed to spawn x2t: {}", e));
        let _ = std::fs::remove_file(&params);
        result
    }

    fn outcome(&self, result: Result<std::process::Output, String>, output: &Path) -> StepRow {
        let output_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        let process = match result {
            Ok(process) => process,
            Err(message) => {
                return StepRow {
                    status: "spawn".into(),
                    message: truncate(&message),
                    ..StepRow::default()
                }
            }
        };
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&process.status).map(|s| s.to_string());
        #[cfg(windows)]
        let signal = process.status.code().and_then(windows_crash);
        #[cfg(not(any(unix, windows)))]
        let signal: Option<String> = None;
        let (status, exit, signal) = classify(process.status.success(), process.status.code(), signal, output_size);
        let message = if status == "ok" {
            String::new()
        } else {
            truncate(&format!(
                "{} {}",
                String::from_utf8_lossy(&process.stderr),
                String::from_utf8_lossy(&process.stdout)
            ))
        };
        let mut row = StepRow {
            status,
            exit,
            signal,
            message,
            ..StepRow::default()
        };
        if output_size > 0 {
            row.size = output_size.to_string();
            row.sha256 = sha256::file_hex(output).unwrap_or_default();
        }
        row
    }

    // open_file_inner: a 0-byte file opens the blank template, CSV and TXT
    // are detected and maybe read from a copy, and Editor.bin must not be empty.
    fn open(&self, paths: &Paths, text_options: &mut Option<text_import::TextOptions>) -> StepRow {
        let mut source = paths.source.clone();
        if x2t_task::is_empty_file(&source) {
            if let Some(template) = x2t_task::blank_template_for(&source) {
                source = self.prepared.layout.resource_dir.join(template);
            }
        }
        let kind = text_import::TextKind::for_path(&source);
        let options = match kind.map(|kind| text_import::detect_file(&source, kind)).transpose() {
            Ok(options) => options,
            Err(e) => return error_row(format!("detect: {}", e)),
        };
        let read_copy = match text_import::prepare_read_copy(&source, &paths.temp_dir, kind, options) {
            Ok(copy) => copy,
            Err(e) => return error_row(format!("read copy: {}", e)),
        };
        let editor_bin = paths.temp_dir.join("Editor.bin");
        let result = self.convert(
            read_copy.as_deref().unwrap_or(&source),
            &editor_bin,
            8192,
            &paths.temp_dir,
            options,
        );
        if let Some(copy) = &read_copy {
            let _ = std::fs::remove_file(copy);
        }
        *text_options = options;
        self.outcome(result, &editor_bin)
    }

    // save_file and save_file_as: the original's CSV/TXT options when both
    // ends are text, then finish_saved_text puts the original's shape back.
    fn save(
        &self,
        paths: &Paths,
        dest: &Path,
        step: &str,
        text_options: Option<text_import::TextOptions>,
    ) -> StepRow {
        let kind = text_import::TextKind::for_path(dest);
        let options = kind.and_then(|kind| text_options.map(|o| o.for_kind(kind)));
        let editor_bin = paths.temp_dir.join("Editor.bin");
        let format_to = x2t_task::detect_format(dest);
        let result = self.convert(&editor_bin, dest, format_to, &paths.temp_dir, options);
        let mut finish_error = None;
        if let (Some(kind), Some(options)) = (kind, options) {
            if dest.exists() {
                if let Err(e) = text_import::finish_saved_text(dest, kind, options) {
                    finish_error = Some(format!("finish_saved_text: {}", e));
                }
            }
        }
        let mut row = self.outcome(result, dest);
        if let Some(error) = finish_error {
            row.message = truncate(&format!("{} {}", error, row.message));
        }
        let kept = if step == "save" { format!("save.{}", extension(dest)) } else { step.to_string() };
        self.keep(&mut row, dest, &paths.out_dir.join(kept));
        row
    }

    fn pdf(&self, paths: &Paths) -> StepRow {
        let dest = paths.save_target.with_extension("pdf");
        let lines = std::cell::RefCell::new(Vec::new());
        let prefix = format!("[PDF] {}/{}", self.entry.doc, self.variant);
        let result = x2t_task::convert_to_pdf(
            self.binaries(),
            Some(&self.prepared.layout.exe_dir_plain),
            &paths.temp_dir.join("Editor.bin").to_string_lossy(),
            &dest.to_string_lossy(),
            &paths.temp_dir,
            &self.prepared.fontdata,
            &|line| {
                self.log.line(&prefix, line);
                lines.borrow_mut().push(line.to_string());
            },
        );
        let output_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        let (status, exit, signal) = classify_pdf(&result, &lines.borrow(), output_size);
        let mut row = StepRow {
            status,
            exit,
            signal,
            message: result.err().map(|e| truncate(&e)).unwrap_or_default(),
            ..StepRow::default()
        };
        if output_size > 0 {
            row.size = output_size.to_string();
            row.sha256 = sha256::file_hex(&dest).unwrap_or_default();
        }
        self.keep(&mut row, &dest, &paths.out_dir.join("export.pdf"));
        row
    }

    // Copies an output into --out under a plain name: names with < or > cannot
    // go into the uploaded artifact. The file must exist under its own name
    // first, which is what the special-name variant checks.
    fn keep(&self, row: &mut StepRow, written: &Path, kept: &Path) {
        if !written.is_file() {
            return;
        }
        match std::fs::copy(written, kept) {
            Ok(_) => row.output = relative(&self.args.out, kept),
            Err(e) => row.message = truncate(&format!("cannot keep the output: {} {}", e, row.message)),
        }
    }
}

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
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

    fn entry(steps: &str, save_as: &str, expect: &str) -> Entry {
        let text = format!(
            "file\tclass\tsteps\texpect\tvariants\tsave_as\tnote\na.ods\tcomparable\t{}\t{}\t\t{}\t\n",
            steps, expect, save_as
        );
        manifest::parse(&text).unwrap().remove(0)
    }

    fn ok() -> StepRow {
        StepRow { status: "ok".into(), ..StepRow::default() }
    }

    #[test]
    fn steps_follow_the_app_order() {
        assert_eq!(plan(&entry("pdf,save,open", "xlsx,csv", "x")), vec!["open", "save", "save_as.xlsx", "save_as.csv", "pdf"]);
        assert_eq!(plan(&entry("open", "", "")), vec!["open"]);
    }

    #[test]
    fn a_failed_open_skips_the_rest_but_a_failed_save_does_not() {
        let e = entry("open,save,pdf", "csv", "x");
        let rows = run_steps(&e, |step| if step == "open" { StepRow { status: "exit".into(), ..StepRow::default() } } else { panic!("{} ran", step) });
        let statuses: Vec<&str> = rows.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(statuses, vec!["exit", "skipped", "skipped", "skipped"]);
        assert_eq!(rows[3].step, "pdf");

        let rows = run_steps(&e, |step| if step == "save" { StepRow { status: "signal".into(), ..StepRow::default() } } else { ok() });
        let statuses: Vec<&str> = rows.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(statuses, vec!["ok", "signal", "ok", "ok"]);
    }

    #[test]
    fn an_expected_failure_is_marked_and_a_surprise_success_too() {
        let e = entry("open", "", "fail:open:88");
        let failed = run_steps(&e, |_| StepRow { status: "exit".into(), exit: "88".into(), ..StepRow::default() });
        assert_eq!(failed[0].status, "expected-fail");
        assert_eq!(failed[0].exit, "88");
        assert_eq!(run_steps(&e, |_| ok())[0].status, "unexpected-ok");
    }

    // A crash or any other failure must not pass for the expected one.
    #[test]
    fn only_the_named_exit_code_is_the_expected_failure() {
        let e = entry("open", "", "fail:open:88");
        let row = |status: &str, exit: &str, signal: &str| StepRow {
            status: status.into(),
            exit: exit.into(),
            signal: signal.into(),
            ..StepRow::default()
        };
        for (given, kept) in [
            (row("signal", "", "11"), "signal"),
            (row("spawn", "", ""), "spawn"),
            (row("error", "", ""), "error"),
            (row("exit", "89", ""), "exit"),
            (row("empty", "0", ""), "empty"),
        ] {
            let got = run_steps(&e, |_| given.clone());
            assert_eq!(got[0].status, kept, "{:?}", given);
        }
    }

    // The expectation belongs to one step: the others report as usual.
    #[test]
    fn the_expected_failure_applies_to_its_own_step_only() {
        let e = entry("open,save", "", "fail:save:88");
        let rows = run_steps(&e, |step| {
            if step == "open" {
                ok()
            } else {
                StepRow { status: "exit".into(), exit: "88".into(), ..StepRow::default() }
            }
        });
        assert_eq!(rows[0].status, "ok");
        assert_eq!(rows[1].status, "expected-fail");
    }

    #[test]
    fn x2t_outcomes_are_classified() {
        let c = |s, code, sig, size| classify(s, code, sig, size).0;
        assert_eq!(c(true, Some(0), None, 10), "ok");
        assert_eq!(c(true, Some(0), None, 0), "empty");
        assert_eq!(c(false, Some(88), None, 0), "exit");
        assert_eq!(classify(false, None, Some("11".into()), 0), ("signal".into(), String::new(), "11".into()));
    }

    #[test]
    fn a_windows_crash_code_is_a_crash_and_ordinary_codes_are_not() {
        assert_eq!(windows_crash(-1073741819), Some("0xC0000005".into()));
        assert_eq!(windows_crash(0xC000_0409_u32 as i32), Some("0xC0000409".into()));
        assert_eq!(windows_crash(0xE06D_7363_u32 as i32), Some("0xE06D7363".into()));
        for code in [0, 1, 88, -1, -999] {
            assert_eq!(windows_crash(code), None, "{}", code);
        }
        let crash = windows_crash(-1073741819);
        assert_eq!(classify(false, Some(-1073741819), crash, 0), ("signal".into(), String::new(), "0xC0000005".into()));
        assert_eq!(classify(false, Some(88), windows_crash(88), 0), ("exit".into(), "88".into(), String::new()));
    }

    // The access violation convert_to_pdf logs as a negative exit code.
    #[test]
    fn a_pdf_crash_on_windows_is_read_as_a_crash() {
        let lines = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let failed = Err("x2t conversion failed".to_string());
        let crash = lines(&["failed exit=-1073741819 pdf_exists=false pdf_size=0"]);
        assert_eq!(classify_pdf_on(true, &failed, &crash, 0), ("signal".into(), String::new(), "0xC0000005".into()));
        assert_eq!(classify_pdf_on(false, &failed, &crash, 0), ("exit".into(), "-1073741819".into(), String::new()));
        for (code, windows) in [("88", true), ("1", true), ("-1", true), ("88", false)] {
            let line = lines(&[&format!("failed exit={} pdf_exists=false pdf_size=0", code)]);
            assert_eq!(classify_pdf_on(windows, &failed, &line, 0), ("exit".into(), code.into(), String::new()));
        }
        let no_code = lines(&["failed exit=-999 pdf_exists=false pdf_size=0"]);
        for windows in [true, false] {
            assert_eq!(classify_pdf_on(windows, &failed, &no_code, 0), ("exit".into(), String::new(), String::new()));
        }
    }

    // The expected failure is an exit code: it stays one on Windows.
    #[test]
    fn the_expected_exit_code_is_not_taken_for_a_crash() {
        let e = entry("open", "", "fail:open:88");
        let (status, exit, signal) = classify(false, Some(88), windows_crash(88), 0);
        let rows = run_steps(&e, |_| StepRow { status: status.clone(), exit: exit.clone(), signal: signal.clone(), ..StepRow::default() });
        assert_eq!(rows[0].status, "expected-fail");
    }

    #[test]
    fn pdf_outcomes_are_read_from_the_log() {
        let lines = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(classify_pdf(&Ok("ok".into()), &[], 500).0, "ok");
        assert_eq!(classify_pdf(&Ok("ok".into()), &[], 0).0, "empty");
        let failed = classify_pdf(&Err("x2t conversion failed (exit code 89): ".into()), &lines(&["failed exit=89 pdf_exists=false pdf_size=0"]), 0);
        assert_eq!(failed, ("exit".into(), "89".into(), String::new()));
        let killed = classify_pdf(
            &Err("x2t conversion failed (exit code -999): ".into()),
            &lines(&["failed exit=-999 pdf_exists=false pdf_size=0", "x2t killed by signal 11"]),
            0,
        );
        assert_eq!(killed, ("signal".into(), String::new(), "11".into()));
        assert_eq!(classify_pdf(&Err("Failed to spawn x2t: nope".into()), &[], 0).0, "spawn");
        assert_eq!(classify_pdf(&Err("font-cache-unavailable: x".into()), &lines(&["setup failed: x"]), 0).0, "error");
    }

    #[test]
    fn long_messages_are_cut_on_a_character() {
        let long = "é".repeat(600);
        assert_eq!(truncate(&long).chars().count(), 503);
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
