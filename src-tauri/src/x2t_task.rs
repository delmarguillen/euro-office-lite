// What the app does to run x2t that needs no Tauri. The file uses only std and
// never names crate:: or tauri, so another crate can include it with #[path]
// and run x2t with exactly the app's code. Log lines go out through a `log`
// parameter, without the prefix the app adds.

// x2t is a console program; without this flag Windows opens a console window for it.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// On Windows, resource_dir() and canonicalize() return \\?\C:\... paths. x2t
// copies whatever spelling it is given into its font cache, and its PDF export
// cannot open fonts listed under that prefix, so paths handed to x2t drop it.
// Only drive and UNC forms have a plain spelling (C:\..., \\server\...); any
// other form, such as \\?\Volume{GUID}\..., is kept as it is.
pub fn strip_extended_prefix(p: &std::path::Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    let Some(rest) = s.strip_prefix(r"\\?\") else {
        return p.to_path_buf();
    };
    let bytes = rest.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        std::path::PathBuf::from(rest)
    } else if let Some(share) = rest
        .get(..4)
        .filter(|head| head.eq_ignore_ascii_case(r"UNC\"))
        .map(|_| &rest[4..])
    {
        std::path::PathBuf::from(format!(r"\\{}", share))
    } else {
        p.to_path_buf()
    }
}

pub fn is_x2t_binary(name: &str) -> bool {
    if cfg!(target_os = "windows") {
        (name.starts_with("x2t-") || name == "x2t.exe") && name.ends_with(".exe")
    } else {
        (name.starts_with("x2t-") && !name.contains('.')) || name == "x2t"
    }
}

// The x2t the PDF export and the font scan run, and on Windows every other
// conversion too: the first match in x2t_search_dirs.
pub fn find_x2t(
    binaries_dir: &std::path::Path,
    exe_dir: Option<&std::path::Path>,
) -> Result<std::path::PathBuf, String> {
    for dir in &x2t_search_dirs(binaries_dir, exe_dir) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if is_x2t_binary(&name) {
                    return Ok(entry.path());
                }
            }
        }
    }
    Err("x2t executable not found".to_string())
}

// Windows: binaries\ only. x2t ships there with its libraries and the Visual
// C++ runtime, and the loader looks in the executable's own folder before
// System32, so an older runtime installed system-wide never wins.
// Elsewhere: binaries/, then its parent, then Contents/MacOS, then `exe_dir`
// (the folder of the app's executable). On Linux and macOS binaries/ holds its
// own copy of x2t, so the PDF export and the font scan never run the sidecar
// that the other conversions use.
fn x2t_search_dirs(
    binaries_dir: &std::path::Path,
    exe_dir: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    if cfg!(target_os = "windows") {
        return vec![binaries_dir.to_path_buf()];
    }
    let mut dirs = vec![
        binaries_dir.to_path_buf(),
        binaries_dir.parent().unwrap_or(binaries_dir).to_path_buf(),
    ];
    let macos_dir = binaries_dir
        .parent()
        .and_then(|resources| resources.parent())
        .map(|contents| contents.join("MacOS"));
    if let Some(macos_dir) = macos_dir.filter(|dir| dir.exists()) {
        dirs.push(macos_dir);
    }
    if let Some(exe_dir) = exe_dir {
        dirs.push(exe_dir.to_path_buf());
    }
    dirs
}

/// Where to write the params file x2t is invoked with. Every path inside the
/// XML is absolute, so the file does not need to sit next to the output; the
/// print path has always written it into temp_dir and works on all three
/// platforms. Keeping it there is what stops a PDF export from leaving
/// `x2t_params.xml` behind in the user's folder (#35), which the cleanup after
/// the run cannot guarantee on its own (a crash or a kill skips it). The name
/// is unique per call so two conversions in the same session cannot clobber
/// each other's params.
pub fn params_path(temp_dir: &std::path::Path, tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    temp_dir.join(format!(
        "x2t_params_{}_{}_{}.xml",
        tag,
        std::process::id(),
        unique
    ))
}

/// Start of the error a PDF export or print returns when there is no font
/// cache, so the frontend can say why instead of showing a generic error.
pub const FONT_CACHE_ERROR: &str = "font-cache-unavailable";

// The font list x2t gets for a PDF. Never the one bundled in binaries/: it has
// no font selection table, and a PDF made with it loses its text.
pub fn pdf_font_list(fontdata_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let allfonts_js = fontdata_dir.join("AllFonts.js");
    if is_nonempty_file(&allfonts_js) {
        Ok(allfonts_js)
    } else {
        Err(format!(
            "{}: {} is missing",
            FONT_CACHE_ERROR,
            allfonts_js.display()
        ))
    }
}

fn path_error(action: &str, path: &std::path::Path, error: std::io::Error) -> String {
    format!("{} {}: {}", action, path.display(), error)
}

fn create_dir(path: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| path_error("Failed to create directory", path, e))
}

fn copy_file(source: &std::path::Path, destination: &std::path::Path) -> Result<u64, String> {
    if source == destination {
        return std::fs::metadata(source)
            .map(|metadata| metadata.len())
            .map_err(|e| path_error("Failed to read source file", source, e));
    }
    if let (Ok(source_path), Ok(destination_path)) = (
        std::fs::canonicalize(source),
        std::fs::canonicalize(destination),
    ) {
        if source_path == destination_path {
            return std::fs::metadata(source)
                .map(|metadata| metadata.len())
                .map_err(|e| path_error("Failed to read source file", source, e));
        }
    }
    if let Some(parent) = destination.parent() {
        create_dir(parent)?;
    }
    std::fs::copy(source, destination).map_err(|e| {
        format!(
            "Failed to copy {} to {}: {}",
            source.display(),
            destination.display(),
            e
        )
    })
}

pub fn write_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        create_dir(parent)?;
    }
    std::fs::write(path, contents).map_err(|e| path_error("Failed to write file", path, e))
}

fn is_nonempty_file(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn symlink_path(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        create_dir(parent)?;
    }
    std::os::unix::fs::symlink(source, destination).map_err(|e| {
        format!(
            "Failed to link {} to {}: {}",
            source.display(),
            destination.display(),
            e
        )
    })
}

// The folder that holds x2t's *.framework bundles: Contents/Frameworks of a
// .app, or binaries/ itself when the frameworks sit next to x2t, as
// get-x2t-ci.sh leaves them. None for an x2t that links plain dylibs.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn x2t_frameworks_dir(binaries_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let bundled = binaries_dir.parent()?.parent()?.join("Frameworks");
    if bundled.is_dir() {
        return Some(bundled);
    }
    let has_framework = std::fs::read_dir(binaries_dir).ok()?.flatten().any(|entry| {
        entry.path().is_dir() && entry.file_name().to_string_lossy().ends_with(".framework")
    });
    has_framework.then(|| binaries_dir.to_path_buf())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reset_work_dir(work_dir: &std::path::Path) -> Result<(), String> {
    if work_dir.exists() {
        std::fs::remove_dir_all(work_dir)
            .map_err(|e| path_error("Failed to reset PDF work directory", work_dir, e))?;
    }
    create_dir(work_dir)
}

const DOCTRENDERER_CONFIG_PARENT: &str = r#"<Settings>
<file>../editors/sdkjs/common/Native/native.js</file>
<file>../editors/sdkjs/common/Native/jquery_native.js</file>
<allfonts>../editors/sdkjs/common/AllFonts.js</allfonts>
<file>../editors/web-apps/vendor/xregexp/xregexp-all-min.js</file>
<sdkjs>../editors/sdkjs</sdkjs>
<dictionaries>../dictionaries</dictionaries>
<DoctSdk>
<file>../editors/sdkjs/word/sdk-all-min.js</file>
<file>../editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>../editors/sdkjs/word/sdk-all.js</file>
</DoctSdk>
<PpttSdk>
<file>../editors/sdkjs/slide/sdk-all-min.js</file>
<file>../editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>../editors/sdkjs/slide/sdk-all.js</file>
</PpttSdk>
<XlstSdk>
<file>../editors/sdkjs/cell/sdk-all-min.js</file>
<file>../editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>../editors/sdkjs/cell/sdk-all.js</file>
</XlstSdk>
</Settings>"#;

const DOCTRENDERER_EDITOR_RESOURCES: [&str; 10] = [
    "sdkjs/common/Native/native.js",
    "sdkjs/common/Native/jquery_native.js",
    "sdkjs/common/libfont/engine/fonts_native.js",
    "sdkjs/word/sdk-all-min.js",
    "sdkjs/word/sdk-all.js",
    "sdkjs/cell/sdk-all-min.js",
    "sdkjs/cell/sdk-all.js",
    "sdkjs/slide/sdk-all-min.js",
    "sdkjs/slide/sdk-all.js",
    "web-apps/vendor/xregexp/xregexp-all-min.js",
];

fn validate_editor_resources(editors_dir: &std::path::Path) -> Result<(), String> {
    for relative_path in DOCTRENDERER_EDITOR_RESOURCES {
        let source = editors_dir.join(relative_path);
        if !is_nonempty_file(&source) {
            return Err(format!(
                "Missing or empty DoctRenderer resource: {}",
                source.display()
            ));
        }
    }
    Ok(())
}

// A path with &, < or > anywhere makes the params XML invalid and x2t exits 88
// without writing anything; x2t decodes the entities back, so the file keeps
// its literal name. Every value goes in element text, never in an attribute.
fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

// x2t loads nothing from the network or from local paths named inside a
// document. Every params XML the app writes ends with this block.
pub fn x2t_options_xml() -> &'static str {
    "<options>\n\
<allowPrivateIP>false</allowPrivateIP>\n\
<allowNetworkRequest>false</allowNetworkRequest>\n\
</options>\n"
}

// Paths are passed raw and escaped here, once. `extra` is already XML: the
// numeric CSV/TXT elements from TextOptions::xml_elements.
pub fn convert_params_xml(
    input: &str,
    output: &str,
    format_to: i32,
    temp_dir: &str,
    extra: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<TaskQueueDataConvert xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
                      xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<m_sFileFrom>{}</m_sFileFrom>
<m_sFileTo>{}</m_sFileTo>
<m_nFormatTo>{}</m_nFormatTo>
<m_sTempDir>{}</m_sTempDir>
{}{}</TaskQueueDataConvert>"#,
        xml_escape(&input.replace('\\', "/")),
        xml_escape(&output.replace('\\', "/")),
        format_to,
        xml_escape(&temp_dir.replace('\\', "/")),
        extra,
        x2t_options_xml(),
    )
}

// x2t 10 writes "<this name>/<x2t version>" into what it saves: <Application>
// in docProps/app.xml, meta:generator in ODF and /Producer and /Creator in PDF.
// It reads the name from the APPLICATION_NAME variable, "ONLYOFFICE" without it.
pub const APPLICATION_NAME: &str = "Euro-Office-Lite";

// The variables every x2t run gets, on every platform.
pub fn x2t_common_env() -> Vec<(&'static str, std::ffi::OsString)> {
    vec![("APPLICATION_NAME", APPLICATION_NAME.into())]
}

// The variables the app sets when it runs x2t for an open, save or export:
// the common ones, and the library folder on Linux. The folder goes through
// to_string_lossy, as the sidecar call always passed it.
pub fn convert_env(binaries_dir: &std::path::Path) -> Vec<(&'static str, std::ffi::OsString)> {
    let mut env = x2t_common_env();
    #[cfg(target_os = "linux")]
    env.push((
        "LD_LIBRARY_PATH",
        binaries_dir.to_string_lossy().into_owned().into(),
    ));
    #[cfg(not(target_os = "linux"))]
    let _ = binaries_dir;
    env
}

fn pdf_params_xml(input: &str, output: &str, fonts_dir: &str, allfonts: &str) -> String {
    pdf_params_xml_with(input, output, fonts_dir, allfonts, "")
}

// x2t reads the editor's changes from changes/ next to the input PDF and
// writes the input with them applied. Its own scratch files go to `scratch`,
// and the changes stay out of the output folder.
fn pdf_changes_params(scratch: &str) -> String {
    format!(
        "<m_nFormatFrom>513</m_nFormatFrom>
<m_bFromChanges>true</m_bFromChanges>
<m_bDontSaveAdditional>true</m_bDontSaveAdditional>
<m_sTempDir>{}</m_sTempDir>
",
        xml_escape(&scratch.replace('\\', "/"))
    )
}

// Paths are passed raw and escaped here, once. `extra` is already XML.
fn pdf_params_xml_with(
    input: &str,
    output: &str,
    fonts_dir: &str,
    allfonts: &str,
    extra: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<TaskQueueDataConvert xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
                      xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<m_sFileFrom>{}</m_sFileFrom>
<m_sFileTo>{}</m_sFileTo>
<m_nFormatTo>513</m_nFormatTo>
<m_bEmbeddedFonts>true</m_bEmbeddedFonts>
<m_sFontDir>{}</m_sFontDir>
<m_sAllFontsPath>{}</m_sAllFontsPath>
{}{}</TaskQueueDataConvert>"#,
        xml_escape(input),
        xml_escape(output),
        xml_escape(&fonts_dir.replace('\\', "/")),
        xml_escape(&allfonts.replace('\\', "/")),
        extra,
        x2t_options_xml(),
    )
}

pub fn convert_to_pdf(
    binaries_dir: &std::path::Path,
    exe_dir: Option<&std::path::Path>,
    input: &str,
    output: &str,
    temp_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
    log: &dyn Fn(&str),
) -> Result<String, String> {
    run_x2t_to_pdf(binaries_dir, exe_dir, input, output, temp_dir, fontdata_dir, log, "")
}

// Writes `source_pdf` with the editor's changes applied to `output`. The
// changes must already be in changes/ next to `source_pdf`.
pub fn apply_pdf_changes(
    binaries_dir: &std::path::Path,
    exe_dir: Option<&std::path::Path>,
    source_pdf: &str,
    output: &str,
    temp_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
    log: &dyn Fn(&str),
) -> Result<String, String> {
    let scratch = temp_dir.join("pdf_changes_tmp");
    reset_dir(&scratch)?;
    let result = run_x2t_to_pdf(
        binaries_dir,
        exe_dir,
        source_pdf,
        output,
        temp_dir,
        fontdata_dir,
        log,
        &pdf_changes_params(&scratch.to_string_lossy()),
    );
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn reset_dir(dir: &std::path::Path) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| path_error("Failed to reset directory", dir, e))?;
    }
    create_dir(dir)
}

#[allow(clippy::too_many_arguments)]
fn run_x2t_to_pdf(
    binaries_dir: &std::path::Path,
    exe_dir: Option<&std::path::Path>,
    input: &str,
    output: &str,
    temp_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
    log: &dyn Fn(&str),
    extra_params: &str,
) -> Result<String, String> {
    let binaries_dir = &strip_extended_prefix(binaries_dir);
    let output_name = std::path::Path::new(output)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown.pdf");
    log(&format!(
        "start platform={} output={}",
        std::env::consts::OS,
        output_name
    ));
    let x2t_exe = find_x2t(binaries_dir, exe_dir).map_err(|error| {
        log(&format!("x2t discovery failed: {}", error));
        error
    })?;
    let fonts_dir = binaries_dir.join("fonts");
    let editors_dir = binaries_dir
        .parent()
        .unwrap_or(binaries_dir)
        .join("editors");

    let allfonts_js = pdf_font_list(fontdata_dir).map_err(|error| {
        log(&format!("setup failed: {}", error));
        error
    })?;

    let runtime_strategy = if cfg!(target_os = "windows") {
        "windows-direct"
    } else if cfg!(target_os = "macos") {
        "macos-workdir"
    } else {
        "linux-workdir"
    };

    validate_editor_resources(&editors_dir).map_err(|error| {
        log(&format!("setup failed: {}", error));
        error
    })?;

    #[cfg(target_os = "linux")]
    let runtime = setup_linux_workdir(
        binaries_dir,
        &x2t_exe,
        &fonts_dir,
        &fontdata_dir,
        &editors_dir,
        &allfonts_js,
        temp_dir,
    );

    #[cfg(target_os = "macos")]
    let runtime = setup_macos_workdir(
        binaries_dir,
        &x2t_exe,
        &fonts_dir,
        &fontdata_dir,
        &editors_dir,
        &allfonts_js,
        temp_dir,
    );

    #[cfg(target_os = "windows")]
    let runtime = setup_windows_direct(
        binaries_dir,
        &x2t_exe,
        &fonts_dir,
        &fontdata_dir,
        &editors_dir,
        &allfonts_js,
    );

    let (run_dir, run_x2t, run_allfonts, run_fonts) = runtime.map_err(|error| {
        log(&format!("setup failed strategy={}: {}", runtime_strategy, error));
        error
    })?;

    let params_xml = params_path(temp_dir, "pdf");

    let fonts_dir_for_xml = std::fs::canonicalize(&run_fonts).unwrap_or_else(|_| run_fonts.clone());
    let allfonts_abs =
        std::fs::canonicalize(&run_allfonts).unwrap_or_else(|_| run_allfonts.clone());

    let xml = pdf_params_xml_with(
        input,
        output,
        &fonts_dir_for_xml.to_string_lossy(),
        &allfonts_abs.to_string_lossy(),
        extra_params,
    );
    write_file(&params_xml, &xml)?;

    let mut cmd = pdf_command(&run_x2t, &run_dir, &params_xml, binaries_dir);
    let result = cmd.output().map_err(|error| {
        let message = format!("Failed to spawn x2t: {}", error);
        log(&message);
        message
    })?;
    let _ = std::fs::remove_file(&params_xml);
    let code = result.status.code().unwrap_or(-999);
    let pdf_exists = std::path::Path::new(output).exists();
    let pdf_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    if result.status.success() {
        log(&format!("success pdf_size={}", pdf_size));
    } else {
        log(&format!(
            "failed exit={} pdf_exists={} pdf_size={}",
            code, pdf_exists, pdf_size
        ));
    }
    if !result.status.success() && !result.stdout.is_empty() {
        log(&format!("x2t stdout: {}", String::from_utf8_lossy(&result.stdout)));
    }
    if !result.status.success() && !result.stderr.is_empty() {
        log(&format!("x2t stderr: {}", String::from_utf8_lossy(&result.stderr)));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = result.status.signal() {
            log(&format!("x2t killed by signal {}", sig));
        }
    }

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!(
            "x2t conversion failed (exit code {}): {}",
            code, stderr
        ))
    }
}

// The x2t run for a PDF export: from `run_dir`, with the libraries of
// `run_dir` first and those of `binaries_dir` after them on Linux.
fn pdf_command(
    run_x2t: &std::path::Path,
    run_dir: &std::path::Path,
    params_xml: &std::path::Path,
    binaries_dir: &std::path::Path,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(run_x2t);
    cmd.current_dir(run_dir)
        .arg(params_xml.to_string_lossy().as_ref())
        .envs(x2t_common_env());
    #[cfg(target_os = "linux")]
    cmd.env(
        "LD_LIBRARY_PATH",
        format!("{}:{}", run_dir.display(), binaries_dir.display()),
    );
    #[cfg(target_os = "macos")]
    cmd.env("DYLD_LIBRARY_PATH", run_dir);
    #[cfg(not(target_os = "linux"))]
    let _ = binaries_dir;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Windows: run x2t directly from binaries_dir (writable, all DLLs present).
/// Write DoctRenderer.config and AllFonts.js (server) in place. DoctRenderer
/// reads its config from x2t's own folder, which is binaries_dir.
#[cfg(target_os = "windows")]
fn setup_windows_direct(
    binaries_dir: &std::path::Path,
    x2t_exe: &std::path::Path,
    fonts_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
    editors_dir: &std::path::Path,
    allfonts_js: &std::path::Path,
) -> Result<
    (
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ),
    String,
> {
    let bin_allfonts = binaries_dir.join("AllFonts.js");
    copy_file(allfonts_js, &bin_allfonts)?;

    let editors_allfonts = editors_dir.join("sdkjs/common/AllFonts.js");
    if editors_allfonts.exists() {
        copy_file(allfonts_js, &editors_allfonts)?;
    }

    let fontsel_src = fontdata_dir.join("font_selection.bin");
    if fontsel_src.exists() {
        copy_file(&fontsel_src, &binaries_dir.join("font_selection.bin"))?;
        copy_file(&fontsel_src, &fonts_dir.join("font_selection.bin"))?;
    }

    write_file(
        &binaries_dir.join("DoctRenderer.config"),
        DOCTRENDERER_CONFIG_PARENT,
    )?;

    Ok((
        binaries_dir.to_path_buf(),
        x2t_exe.to_path_buf(),
        bin_allfonts,
        fonts_dir.to_path_buf(),
    ))
}

/// Linux: build a writable work directory because installed resources are read-only.
#[cfg(target_os = "linux")]
fn setup_linux_workdir(
    binaries_dir: &std::path::Path,
    x2t_exe: &std::path::Path,
    fonts_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
    editors_dir: &std::path::Path,
    allfonts_js: &std::path::Path,
    temp_dir: &std::path::Path,
) -> Result<
    (
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ),
    String,
> {
    let work_dir = temp_dir.join("x2t-workdir");
    reset_work_dir(&work_dir)?;
    let work_binaries = work_dir.join("binaries");
    let work_editors = work_dir.join("editors");
    let work_dictionaries = work_dir.join("dictionaries");

    create_dir(&work_binaries)?;
    create_dir(&work_editors.join("sdkjs/common/Native"))?;
    create_dir(&work_editors.join("sdkjs/common/libfont/engine"))?;
    create_dir(&work_editors.join("sdkjs/word"))?;
    create_dir(&work_editors.join("web-apps/vendor/xregexp"))?;
    create_dir(&work_dictionaries)?;

    let work_x2t = work_binaries.join("x2t");
    symlink_path(x2t_exe, &work_x2t)?;

    let entries = std::fs::read_dir(binaries_dir)
        .map_err(|e| path_error("Failed to read binaries directory", binaries_dir, e))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| path_error("Failed to read binaries entry", binaries_dir, e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".so")
            || name.contains(".so.")
            || name.ends_with(".dat")
            || name == "package.config"
        {
            symlink_path(&entry.path(), &work_binaries.join(&name))?;
        }
    }

    // A real folder, not a link to the installed one: the PDF export looks for
    // font_selection.bin in m_sFontDir, and without it falls back to the bundled
    // fonts only, so Arabic or Hebrew text loses its glyphs.
    let work_fonts = work_binaries.join("fonts");
    create_dir(&work_fonts)?;
    let font_entries = std::fs::read_dir(fonts_dir)
        .map_err(|e| path_error("Failed to read fonts directory", fonts_dir, e))?;
    for entry in font_entries {
        let entry = entry.map_err(|e| path_error("Failed to read fonts entry", fonts_dir, e))?;
        // The cache's copy goes in below; a link here would make that copy
        // write through it into the read-only install.
        if entry.path().is_file() && entry.file_name() != "font_selection.bin" {
            symlink_path(&entry.path(), &work_fonts.join(entry.file_name()))?;
        }
    }

    let fontsel_src = fontdata_dir.join("font_selection.bin");
    if is_nonempty_file(&fontsel_src) {
        copy_file(&fontsel_src, &work_binaries.join("font_selection.bin"))?;
        copy_file(&fontsel_src, &work_fonts.join("font_selection.bin"))?;
    }

    let work_allfonts = work_binaries.join("AllFonts.js");
    copy_file(allfonts_js, &work_allfonts)?;
    copy_file(allfonts_js, &work_editors.join("sdkjs/common/AllFonts.js"))?;
    write_file(
        &work_binaries.join("DoctRenderer.config"),
        DOCTRENDERER_CONFIG_PARENT,
    )?;
    link_editor_resources(editors_dir, &work_editors)?;

    Ok((work_binaries, work_x2t, work_allfonts, work_fonts))
}

/// macOS: never modify the signed .app bundle. Build the complete mutable
/// DoctRenderer layout in the application temp directory.
#[cfg(target_os = "macos")]
fn setup_macos_workdir(
    binaries_dir: &std::path::Path,
    x2t_exe: &std::path::Path,
    fonts_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
    editors_dir: &std::path::Path,
    allfonts_js: &std::path::Path,
    temp_dir: &std::path::Path,
) -> Result<
    (
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ),
    String,
> {
    let work_dir = temp_dir.join("x2t-workdir");
    reset_work_dir(&work_dir)?;
    let work_binaries = work_dir.join("binaries");
    let work_editors = work_dir.join("editors");
    let work_dictionaries = work_dir.join("dictionaries");
    let work_fonts = work_binaries.join("fonts");

    create_dir(&work_binaries)?;
    create_dir(&work_fonts)?;
    create_dir(&work_editors.join("sdkjs/common/Native"))?;
    create_dir(&work_editors.join("sdkjs/common/libfont/engine"))?;
    create_dir(&work_editors.join("sdkjs/word"))?;
    create_dir(&work_editors.join("web-apps/vendor/xregexp"))?;
    create_dir(&work_dictionaries)?;

    let work_x2t = work_binaries.join("x2t");
    copy_file(x2t_exe, &work_x2t)?;

    // x2t finds its frameworks through @executable_path/../Frameworks, which
    // for this copy is work_dir/Frameworks; a link keeps them in place.
    if let Some(frameworks) = x2t_frameworks_dir(binaries_dir) {
        symlink_path(&frameworks, &work_dir.join("Frameworks"))?;
    }

    let entries = std::fs::read_dir(binaries_dir)
        .map_err(|e| path_error("Failed to read macOS binaries directory", binaries_dir, e))?;
    for entry in entries {
        let entry = entry
            .map_err(|e| path_error("Failed to read macOS binaries entry", binaries_dir, e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dependency = name.ends_with(".dylib")
            || name.ends_with(".dat")
            || (name.ends_with(".config") && name != "DoctRenderer.config");
        if is_dependency {
            copy_file(&entry.path(), &work_binaries.join(&name))?;
        }
    }

    let font_entries = std::fs::read_dir(fonts_dir)
        .map_err(|e| path_error("Failed to read macOS fonts directory", fonts_dir, e))?;
    for entry in font_entries {
        let entry =
            entry.map_err(|e| path_error("Failed to read macOS font entry", fonts_dir, e))?;
        if entry.path().is_file() {
            symlink_path(&entry.path(), &work_fonts.join(entry.file_name()))?;
        }
    }

    let fontsel_src = fontdata_dir.join("font_selection.bin");
    if is_nonempty_file(&fontsel_src) {
        copy_file(&fontsel_src, &work_binaries.join("font_selection.bin"))?;
        copy_file(&fontsel_src, &work_fonts.join("font_selection.bin"))?;
    }

    let work_allfonts = work_binaries.join("AllFonts.js");
    copy_file(allfonts_js, &work_allfonts)?;
    copy_file(allfonts_js, &work_editors.join("sdkjs/common/AllFonts.js"))?;
    write_file(
        &work_binaries.join("DoctRenderer.config"),
        DOCTRENDERER_CONFIG_PARENT,
    )?;
    link_editor_resources(editors_dir, &work_editors)?;

    Ok((work_binaries, work_x2t, work_allfonts, work_fonts))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn link_editor_resources(
    editors_dir: &std::path::Path,
    work_editors: &std::path::Path,
) -> Result<(), String> {
    for relative_path in DOCTRENDERER_EDITOR_RESOURCES {
        let source = editors_dir.join(relative_path);
        symlink_path(&source, &work_editors.join(relative_path))?;
    }
    Ok(())
}

// x2t format ids, shared with the editors: web-apps uses the same numbers in
// utils.defines.FileFormat to pick the icon and to filter the Open Recent list.
// Values from core/Common/OfficeFileFormats.h at v10.0.0.169.
pub fn detect_format(path: &std::path::Path) -> i32 {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("docx") => 65,
        Some("doc") => 66,
        Some("odt") => 67,
        Some("rtf") => 68,
        Some("txt") => 69,
        Some("docm") => 75,
        Some("dotx") => 76,
        Some("dotm") => 77,
        Some("fodt") => 78,
        Some("ott") => 79,
        Some("xlsx") => 257,
        Some("xls") => 258,
        Some("ods") => 259,
        Some("csv") => 260,
        Some("xlsm") => 261,
        Some("xltx") => 262,
        Some("xlsb") => 264,
        Some("fods") => 265,
        Some("ots") => 266,
        Some("pptx") => 129,
        Some("ppt") => 130,
        Some("odp") => 131,
        Some("ppsx") => 132,
        Some("pptm") => 133,
        Some("potx") => 135,
        Some("fodp") => 137,
        Some("otp") => 138,
        Some("pdf") => 513,
        _ => 0,
    }
}

// File managers on Linux create 0-byte files from their "New document" menu and
// opening one is an ordinary desktop flow, so an empty file is treated as a new
// document that saves back to that path (#33, proposed by the reporter).
// The blank template only decides which editor opens: the format written on the
// first save comes from detect_format on the destination, so a 0-byte .odt is
// edited in Word and saved as ODT.
//
// Deliberately absent: pdf (nothing to edit) and the legacy doc/xls/ppt
// (writing those back is not validated), which keep the plain rejection; and
// txt and csv, which x2t refuses when empty (exit 88), so they get the open
// error too.
pub fn blank_template_for(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "docx" | "odt" | "rtf" => Some("templates/blank.docx"),
        "xlsx" | "ods" => Some("templates/blank.xlsx"),
        "pptx" | "odp" => Some("templates/blank.pptx"),
        _ => None,
    }
}

pub fn is_empty_file(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.len() == 0)
        .unwrap_or(false)
}

const ZIP_SIGNATURE: &[u8] = b"PK\x03\x04";
const CFB_SIGNATURE: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

// A container format must start like one; x2t would otherwise pick the format
// from the content. OOXML is a ZIP, or a CFB when it has a password; xlsb is
// always a ZIP. ODF is always a ZIP, flat ODF an XML document. Word 97-2003,
// Excel 97-2003 and PowerPoint 97-2003 are CFB; the legacy extensions also
// take the other content x2t converts under them: see is_legacy_doc,
// is_legacy_xls and is_legacy_ppt. HTML is never accepted, because x2t picks
// the format from the content. Other extensions (rtf, txt, csv...) are left to
// x2t.
pub fn content_matches_extension(path: &std::path::Path, head: &[u8]) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return true;
    };
    let zip = is_zip(head);
    let cfb = is_cfb(head);
    match ext.to_lowercase().as_str() {
        "docx" | "docm" | "dotx" | "dotm" => zip || cfb,
        "xlsx" | "xlsm" | "xltx" | "xltm" => zip || cfb,
        "pptx" | "pptm" | "potx" | "potm" | "ppsx" | "ppsm" => zip || cfb,
        "xlsb" => zip,
        "odt" | "ods" | "odp" | "ott" | "ots" | "otp" => zip,
        "doc" => is_legacy_doc(head),
        "xls" => is_legacy_xls(head),
        "ppt" => is_legacy_ppt(head),
        "fodt" | "fods" | "fodp" => starts_with_xml_declaration(head),
        _ => true,
    }
}

fn is_zip(head: &[u8]) -> bool {
    head.starts_with(ZIP_SIGNATURE)
}

fn is_cfb(head: &[u8]) -> bool {
    head.starts_with(CFB_SIGNATURE)
}

// .doc: Word 97-2003 (CFB), a DOCX renamed to .doc (ZIP) or an RTF saved as .doc.
fn is_legacy_doc(head: &[u8]) -> bool {
    is_cfb(head) || is_zip(head) || starts_with_rtf(head)
}

// .xls: Excel 97-2003 (CFB), an XLSX renamed to .xls (ZIP) or Excel 2003 XML.
fn is_legacy_xls(head: &[u8]) -> bool {
    is_cfb(head) || is_zip(head) || is_spreadsheet_ml(head)
}

// .ppt: PowerPoint 97-2003 (CFB) or a PPTX renamed to .ppt (ZIP).
fn is_legacy_ppt(head: &[u8]) -> bool {
    is_cfb(head) || is_zip(head)
}

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const XML_DECLARATION: &[u8] = b"<?xml";
const RTF_START: &[u8] = b"{\\rtf";
const SPREADSHEET_ML_NAMESPACE: &[u8] = b"urn:schemas-microsoft-com:office:spreadsheet";
const SPREADSHEET_ML_PROGID: &[u8] = b"progid=\"Excel.Sheet\"";

// The head without an optional UTF-8 BOM and the whitespace after it.
fn skip_bom_and_space(head: &[u8]) -> &[u8] {
    let without_bom = head.strip_prefix(UTF8_BOM).unwrap_or(head);
    let first_non_space = without_bom
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(without_bom.len());
    &without_bom[first_non_space..]
}

// An XML document's first bytes: an optional UTF-8 BOM, optional whitespace,
// then "<?xml".
fn starts_with_xml_declaration(head: &[u8]) -> bool {
    skip_bom_and_space(head).starts_with(XML_DECLARATION)
}

// An RTF document's first bytes: an optional UTF-8 BOM, optional whitespace,
// then "{\rtf".
fn starts_with_rtf(head: &[u8]) -> bool {
    skip_bom_and_space(head).starts_with(RTF_START)
}

// Excel 2003 XML (SpreadsheetML): an XML document whose first bytes carry the
// <?mso-application progid="Excel.Sheet"?> instruction or the SpreadsheetML
// namespace on the root <Workbook>. Any other XML, XHTML included, is not.
fn is_spreadsheet_ml(head: &[u8]) -> bool {
    if !starts_with_xml_declaration(head) {
        return false;
    }
    contains(head, SPREADSHEET_ML_PROGID) || contains(head, SPREADSHEET_ML_NAMESPACE)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

// x2t does not use fontconfig: it scans a fixed list of directories
// (/usr/share/fonts, /usr/share/X11/fonts, /usr/local/share/fonts and a couple more),
// so fonts installed per user or exposed by the Flatpak host mounts are invisible to
// the editor even though fc-list finds them (issue #29). Passing them explicitly is
// safe: x2t deduplicates directories it already scans.
#[cfg(target_os = "linux")]
fn extra_font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = vec![
        // Flatpak host font mounts
        std::path::PathBuf::from("/run/host/fonts"),
        std::path::PathBuf::from("/run/host/local-fonts"),
        std::path::PathBuf::from("/run/host/user-fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    dirs.retain(|dir| dir.is_dir());
    dirs
}

#[cfg(not(target_os = "linux"))]
fn extra_font_dirs() -> Vec<std::path::PathBuf> {
    Vec::new()
}

#[cfg(target_os = "linux")]
const FONT_EXTENSIONS: [&str; 5] = ["ttf", "otf", "ttc", "pfb", "pfa"];

#[cfg(target_os = "linux")]
fn has_font_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext = ext.to_ascii_lowercase();
            FONT_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

// x2t's font scanner skips symlink entries when it lists a directory, so fonts
// installed as links (on NixOS every font is one) never reach the editor.
// Resolving the links and handing over their real directories is safe: x2t
// deduplicates directories it already scans.
#[cfg(target_os = "linux")]
fn symlink_font_target_dirs(roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    const MAX_DEPTH: usize = 8;

    let mut found: Vec<std::path::PathBuf> = Vec::new();
    // Walking with an explicit stack and symlink_metadata: following directory
    // links while walking can loop forever.
    let mut stack: Vec<(std::path::PathBuf, usize)> =
        roots.iter().map(|root| (root.clone(), 0usize)).collect();

    while let Some((dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let link_meta = match std::fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if link_meta.file_type().is_symlink() {
                let target = match std::fs::canonicalize(&path) {
                    Ok(target) => target,
                    Err(_) => continue,
                };
                let target_meta = match std::fs::metadata(&target) {
                    Ok(meta) => meta,
                    Err(_) => continue,
                };
                if target_meta.is_dir() {
                    found.push(target);
                } else if has_font_extension(&target) {
                    if let Some(parent) = target.parent() {
                        found.push(parent.to_path_buf());
                    }
                }
            } else if link_meta.is_dir() && depth + 1 < MAX_DEPTH {
                stack.push((path, depth + 1));
            }
        }
    }

    found.sort();
    found.dedup();
    // A target under a root is already covered: x2t scans the roots recursively,
    // and handing it the same font twice shows a duplicate in the font list.
    let roots: Vec<std::path::PathBuf> = roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
    found.retain(|dir| !roots.iter().any(|root| dir.starts_with(root)));
    found
}

#[cfg(not(target_os = "linux"))]
fn symlink_font_target_dirs(_roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    Vec::new()
}

// The directories x2t scans on its own. It skips their symlinks too, so they
// are walked here even though they are not passed as extra arguments.
#[cfg(target_os = "linux")]
fn symlink_scan_roots() -> Vec<std::path::PathBuf> {
    let mut roots = extra_font_dirs();
    for standard in [
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/usr/share/X11/fonts",
    ] {
        let dir = std::path::PathBuf::from(standard);
        if dir.is_dir() {
            roots.push(dir);
        }
    }
    roots
}

#[cfg(not(target_os = "linux"))]
fn symlink_scan_roots() -> Vec<std::path::PathBuf> {
    Vec::new()
}

// The folders x2t's own scan reads on Windows (CApplicationFonts::
// GetSetupFontFiles in ONLYOFFICE core): C:\Windows\Fonts, and for the current
// user AppData\Local\Microsoft\Windows\Fonts and its FontCache\4\CloudFonts.
// The HKLM font entries it also reads point into the Fonts folder or are full
// paths. x2t spells the folders out from C:\Windows and C:\Users\<name>; the
// fingerprint takes them from %WINDIR% and %LOCALAPPDATA%.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn windows_font_dirs(
    windir: Option<&std::path::Path>,
    local_app_data: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windir) = windir {
        dirs.push(windir.join("Fonts"));
    }
    if let Some(local) = local_app_data {
        dirs.push(local.join("Microsoft").join("Windows").join("Fonts"));
        dirs.push(local.join("Microsoft").join("FontCache").join("4").join("CloudFonts"));
    }
    dirs
}

// The folders x2t's own scan reads on macOS: /Library/Fonts and
// /System/Library/Fonts, plus every font CoreText lists, which takes in the
// user's ~/Library/Fonts.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn macos_font_dirs(home: Option<&std::path::Path>) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![
        std::path::PathBuf::from("/System/Library/Fonts"),
        std::path::PathBuf::from("/Library/Fonts"),
    ];
    if let Some(home) = home {
        dirs.push(home.join("Library").join("Fonts"));
    }
    dirs
}

// A folder that does not exist yet stays in the list: the fingerprint writes
// it as missing, so creating it changes the marker.
#[cfg(windows)]
fn platform_font_dirs() -> Vec<std::path::PathBuf> {
    let windir = std::env::var_os("WINDIR").map(std::path::PathBuf::from);
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    windows_font_dirs(windir.as_deref(), local_app_data.as_deref())
}

#[cfg(target_os = "macos")]
fn platform_font_dirs() -> Vec<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    macos_font_dirs(home.as_deref())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_font_dirs() -> Vec<std::path::PathBuf> {
    Vec::new()
}

// x2t writes the font paths exactly as it receives them into AllFonts.js and
// font_selection.bin, and the PDF export reads them back from there, so the
// scan never gets the \\?\ form of the bundled fonts folder.
pub fn font_scan_command(
    x2t_exe: &std::path::Path,
    fontdata_dir: &std::path::Path,
    binaries_dir: &std::path::Path,
) -> std::process::Command {
    let binaries_dir = strip_extended_prefix(binaries_dir);
    let mut cmd = std::process::Command::new(strip_extended_prefix(x2t_exe));
    cmd.current_dir(&binaries_dir)
        .arg("-create-allfonts")
        .arg(fontdata_dir)
        .arg(binaries_dir.join("fonts"))
        .envs(x2t_common_env());
    #[cfg(target_os = "linux")]
    cmd.env("LD_LIBRARY_PATH", &binaries_dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

// Every font folder the scan reads: the bundled one, those passed to x2t and
// the standard ones x2t scans on its own. A change in any of them changes
// the font cache marker.
pub fn font_scan_dirs(binaries_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![strip_extended_prefix(binaries_dir).join("fonts")];
    let roots = symlink_scan_roots();
    dirs.extend(symlink_font_target_dirs(&roots));
    dirs.extend(roots);
    dirs.extend(platform_font_dirs());
    dirs.sort();
    dirs.dedup();
    dirs
}

// Runs x2t's font scan into `fontdata_dir` and reports whether it succeeded.
pub fn generate_fonts(
    fontdata_dir: &std::path::Path,
    binaries_dir: &std::path::Path,
    exe_dir: Option<&std::path::Path>,
    log: &dyn Fn(&str),
) -> bool {
    log("First-run font generation starting...");

    let x2t_exe = match find_x2t(binaries_dir, exe_dir) {
        Ok(p) => p,
        Err(_) => {
            log("ERROR: x2t executable not found, cannot generate fonts");
            return false;
        }
    };

    let mut cmd = font_scan_command(&x2t_exe, fontdata_dir, binaries_dir);
    for dir in extra_font_dirs() {
        log(&format!("Extra font directory: {}", dir.display()));
        cmd.arg(dir);
    }
    let symlink_dirs = symlink_font_target_dirs(&symlink_scan_roots());
    for dir in &symlink_dirs {
        log(&format!("Symlink font target directory: {}", dir.display()));
        cmd.arg(dir);
    }
    if !symlink_dirs.is_empty() {
        log(&format!(
            "Symlink font target directories resolved: {}",
            symlink_dirs.len()
        ));
    }
    match cmd.output() {
        Ok(result) => {
            let code = result.status.code().unwrap_or(-1);
            if result.status.success() {
                let allfonts_size = std::fs::metadata(fontdata_dir.join("AllFonts.js"))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                let selection_size = std::fs::metadata(fontdata_dir.join("font_selection.bin"))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                log(&format!(
                    "Font generation complete: AllFonts.js={} bytes, font_selection.bin={} bytes",
                    allfonts_size, selection_size
                ));
                true
            } else {
                log(&format!(
                    "ERROR: Font generation failed (exit code {}): {}",
                    code,
                    String::from_utf8_lossy(&result.stderr).trim()
                ));
                false
            }
        }
        Err(e) => {
            log(&format!("ERROR: Font generation failed: {}", e));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "eo-x2t-task-{}-{}-{}",
            name,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    // The name is_x2t_binary accepts for an installed x2t on this platform.
    fn x2t_name() -> &'static str {
        if cfg!(target_os = "windows") {
            "x2t.exe"
        } else {
            "x2t"
        }
    }

    // The #35 regression: convert_to_pdf used to drop the params file next to
    // the PDF it was writing, so every export left an x2t_params.xml in the
    // user's folder. The params file belongs in the app temp directory.
    #[test]
    fn params_stay_out_of_the_output_folder() {
        let temp_dir = std::path::Path::new("/tmp/eo-app");
        let output_dir = std::path::Path::new("/home/user/Documents");

        for tag in ["pdf", "convert"] {
            let path = params_path(temp_dir, tag);

            assert_eq!(
                path.parent(),
                Some(temp_dir),
                "params must be written to temp_dir, never beside the output (#35)"
            );
            assert_ne!(
                path.parent(),
                Some(output_dir),
                "params must never land in the folder the user picked (#35)"
            );
        }
    }

    // Two exports in the same session must not share a params file: the second
    // would overwrite the first while x2t is still reading it.
    #[test]
    fn params_path_is_unique_per_call() {
        let temp_dir = std::path::Path::new("/tmp/eo-app");

        let first = params_path(temp_dir, "pdf");
        let second = params_path(temp_dir, "pdf");

        assert_ne!(first, second, "each conversion needs its own params file");
    }

    // Plain strings, so it runs anywhere: only the spelling is under test.
    #[test]
    fn the_extended_prefix_is_dropped_only_where_a_plain_form_exists() {
        use std::path::{Path, PathBuf};
        let cases = [
            (r"\\?\C:\App\binaries", r"C:\App\binaries"),
            (
                r"\\?\UNC\server\share\App\binaries",
                r"\\server\share\App\binaries",
            ),
            (
                r"\\?\Volume{0b1c2d3e-0000-0000-0000-000000000000}\App\binaries",
                r"\\?\Volume{0b1c2d3e-0000-0000-0000-000000000000}\App\binaries",
            ),
            (r"C:\App\binaries", r"C:\App\binaries"),
        ];
        let wrong: Vec<(&str, PathBuf)> = cases
            .iter()
            .map(|(given, expected)| (*given, *expected, strip_extended_prefix(Path::new(given))))
            .filter(|(_, expected, got)| got != &PathBuf::from(expected))
            .map(|(given, _, got)| (given, got))
            .collect();
        assert!(wrong.is_empty(), "wrong spellings: {:?}", wrong);
    }

    #[test]
    fn x2t_names_and_decoys() {
        let windows = cfg!(target_os = "windows");
        assert_eq!(is_x2t_binary("x2t.exe"), windows);
        assert_eq!(is_x2t_binary("x2t-x86_64-pc-windows-msvc.exe"), windows);
        assert_eq!(is_x2t_binary("x2t"), !windows);
        assert_eq!(is_x2t_binary("x2t-x86_64-unknown-linux-gnu"), !windows);
        for decoy in ["x2t.dll", "x2t.pdb", "x2t.exe.config", "libx2t.so", "x2tool"] {
            assert!(!is_x2t_binary(decoy), "{} is not x2t", decoy);
        }
    }

    // .deb: the PDF export and the font scan run binaries/x2t, never the
    // sidecar in usr/bin next to the app.
    #[test]
    fn linux_layout_finds_the_copy_in_binaries() {
        let root = scratch_dir("deb");
        let binaries = root.join("usr/lib/Euro-Office-Lite/binaries");
        let exe_dir = root.join("usr/bin");
        touch(&binaries.join(x2t_name()));
        touch(&binaries.join("libkernel.so"));
        touch(&exe_dir.join(x2t_name()));
        touch(&exe_dir.join("euro-office-lite"));

        assert_eq!(
            find_x2t(&binaries, Some(&exe_dir)),
            Ok(binaries.join(x2t_name()))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // .app: Contents/Resources/binaries/x2t wins over the sidecar in
    // Contents/MacOS.
    #[test]
    fn macos_layout_finds_the_copy_in_binaries() {
        let root = scratch_dir("app");
        let contents = root.join("Euro-Office-Lite.app/Contents");
        let binaries = contents.join("Resources/binaries");
        let macos = contents.join("MacOS");
        touch(&binaries.join(x2t_name()));
        touch(&macos.join(x2t_name()));
        touch(&macos.join("euro-office-lite"));

        assert_eq!(
            find_x2t(&binaries, Some(&macos)),
            Ok(binaries.join(x2t_name()))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // .app: Contents/Frameworks, even with a framework left next to x2t.
    #[test]
    fn an_app_keeps_x2t_frameworks_in_contents_frameworks() {
        let root = scratch_dir("fw-app");
        let contents = root.join("Euro-Office-Lite.app/Contents");
        let binaries = contents.join("Resources/binaries");
        touch(&binaries.join(x2t_name()));
        touch(&binaries.join("graphics.framework/Versions/A/graphics"));
        touch(&contents.join("Frameworks/kernel.framework/Versions/A/kernel"));

        assert_eq!(
            x2t_frameworks_dir(&binaries),
            Some(contents.join("Frameworks"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // Dev and CI trees: get-x2t-ci.sh unzips the frameworks next to x2t.
    #[test]
    fn frameworks_next_to_x2t_are_used_outside_an_app() {
        let root = scratch_dir("fw-dev");
        let binaries = root.join("src-tauri/binaries");
        touch(&binaries.join(x2t_name()));
        touch(&binaries.join("DoctRenderer.config"));
        touch(&binaries.join("kernel.framework/Versions/A/kernel"));

        assert_eq!(x2t_frameworks_dir(&binaries), Some(binaries.clone()));
        let _ = std::fs::remove_dir_all(&root);
    }

    // An x2t with plain dylibs (8.2) has no frameworks; a file that only
    // looks like one does not count.
    #[test]
    fn an_x2t_without_frameworks_gets_no_frameworks_dir() {
        let root = scratch_dir("fw-none");
        let contents = root.join("Euro-Office-Lite.app/Contents");
        let binaries = contents.join("Resources/binaries");
        touch(&binaries.join(x2t_name()));
        touch(&binaries.join("libkernel.dylib"));
        touch(&binaries.join("notes.framework"));
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();

        assert_eq!(x2t_frameworks_dir(&binaries), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    // NSIS: x2t sits in binaries\ with its libraries and the Visual C++
    // runtime; the install folder holds only the app.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_layout_finds_the_one_in_binaries() {
        let root = scratch_dir("nsis");
        let binaries = root.join("binaries");
        touch(&binaries.join("kernel.dll"));
        touch(&binaries.join("msvcp140.dll"));
        touch(&binaries.join("x2t.dll"));
        touch(&binaries.join("x2t.pdb"));
        touch(&binaries.join("x2t.exe"));
        touch(&root.join("euro-office-lite.exe"));

        assert_eq!(find_x2t(&binaries, Some(&root)), Ok(binaries.join("x2t.exe")));
        let _ = std::fs::remove_dir_all(&root);
    }

    // An x2t in the install folder (the sidecar of earlier installers) would
    // load an older Visual C++ runtime from System32 before the one in
    // binaries\, so Windows never runs it, even when it is the only one.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_never_runs_an_x2t_in_the_install_folder() {
        let root = scratch_dir("nsis-root-x2t");
        let binaries = root.join("binaries");
        touch(&binaries.join("kernel.dll"));
        touch(&root.join("x2t.exe"));
        touch(&root.join("euro-office-lite.exe"));

        assert_eq!(
            find_x2t(&binaries, Some(&root)),
            Err("x2t executable not found".to_string())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_search_order_is_binaries_parent_macos_then_exe_dir() {
        let root = scratch_dir("order");
        let contents = root.join("Contents");
        let binaries = contents.join("Resources/binaries");
        let parent = contents.join("Resources");
        let macos = contents.join("MacOS");
        let exe_dir = root.join("exe");
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::create_dir_all(&macos).unwrap();
        touch(&exe_dir.join(x2t_name()));

        assert_eq!(find_x2t(&binaries, Some(&exe_dir)), Ok(exe_dir.join(x2t_name())));
        touch(&macos.join(x2t_name()));
        assert_eq!(find_x2t(&binaries, Some(&exe_dir)), Ok(macos.join(x2t_name())));
        touch(&parent.join(x2t_name()));
        assert_eq!(find_x2t(&binaries, Some(&exe_dir)), Ok(parent.join(x2t_name())));
        touch(&binaries.join(x2t_name()));
        assert_eq!(find_x2t(&binaries, Some(&exe_dir)), Ok(binaries.join(x2t_name())));
        let _ = std::fs::remove_dir_all(&root);
    }

    // A dev build names the sidecar after the target triple.
    #[test]
    fn a_target_triple_name_is_found() {
        let root = scratch_dir("triple");
        let binaries = root.join("binaries");
        let name = if cfg!(target_os = "windows") {
            "x2t-x86_64-pc-windows-msvc.exe"
        } else {
            "x2t-x86_64-unknown-linux-gnu"
        };
        touch(&binaries.join(name));

        assert_eq!(find_x2t(&binaries, None), Ok(binaries.join(name)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_x2t_the_error_is_the_one_the_logs_show() {
        let root = scratch_dir("none");
        let binaries = root.join("binaries");
        touch(&binaries.join("x2t.dll"));
        touch(&binaries.join("x2t.pdb"));

        assert_eq!(
            find_x2t(&binaries, Some(&root.join("missing"))),
            Err("x2t executable not found".to_string())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn xml_escape_covers_the_three_markup_characters() {
        assert_eq!(
            xml_escape("/home/u/Q&A/R&D.docx"),
            "/home/u/Q&amp;A/R&amp;D.docx"
        );
        assert_eq!(
            xml_escape("/home/u/a<b>/c<d>.pdf"),
            "/home/u/a&lt;b&gt;/c&lt;d&gt;.pdf"
        );
        // Quotes stay: every value is element text.
        assert_eq!(xml_escape(r#"/it's "q".docx"#), r#"/it's "q".docx"#);
        assert_eq!(xml_escape("/home/u/plain.docx"), "/home/u/plain.docx");
    }

    // A name that already looks escaped is still a literal name, so its & is
    // escaped like any other and x2t gets back exactly A&amp;B.docx.
    #[test]
    fn xml_escape_runs_once_on_entity_lookalikes() {
        assert_eq!(xml_escape("/d/A&amp;B.docx"), "/d/A&amp;amp;B.docx");
        assert_eq!(xml_escape("/d/x&lt;y.docx"), "/d/x&amp;lt;y.docx");
    }

    #[test]
    fn plain_paths_give_the_same_pdf_params_as_before() {
        let xml = pdf_params_xml(
            "/home/u/Doc.docx",
            "/home/u/Doc.pdf",
            r"C:\eo\fonts",
            r"C:\eo\AllFonts.js",
        );
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<TaskQueueDataConvert xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n                      \
xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n\
<m_sFileFrom>/home/u/Doc.docx</m_sFileFrom>\n\
<m_sFileTo>/home/u/Doc.pdf</m_sFileTo>\n\
<m_nFormatTo>513</m_nFormatTo>\n\
<m_bEmbeddedFonts>true</m_bEmbeddedFonts>\n\
<m_sFontDir>C:/eo/fonts</m_sFontDir>\n\
<m_sAllFontsPath>C:/eo/AllFonts.js</m_sAllFontsPath>\n\
<options>\n\
<allowPrivateIP>false</allowPrivateIP>\n\
<allowNetworkRequest>false</allowNetworkRequest>\n\
</options>\n\
</TaskQueueDataConvert>"
        );
    }

    #[test]
    fn pdf_params_escape_every_path_in_name_and_folder() {
        let xml = pdf_params_xml(
            "/home/u/Q&A/R&D.docx",
            "/home/u/Q&A/p<e>.pdf",
            "/opt/R&D/fonts",
            "/opt/R&D/AllFonts.js",
        );
        assert!(xml.contains("<m_sFileFrom>/home/u/Q&amp;A/R&amp;D.docx</m_sFileFrom>"));
        assert!(xml.contains("<m_sFileTo>/home/u/Q&amp;A/p&lt;e&gt;.pdf</m_sFileTo>"));
        assert!(xml.contains("<m_sFontDir>/opt/R&amp;D/fonts</m_sFontDir>"));
        assert!(xml.contains("<m_sAllFontsPath>/opt/R&amp;D/AllFonts.js</m_sAllFontsPath>"));
    }

    #[test]
    fn pdf_params_escape_an_entity_lookalike_once() {
        let xml = pdf_params_xml("/d/A&amp;B.docx", "/d/A&amp;B.pdf", "/f", "/f/AllFonts.js");
        assert!(xml.contains("<m_sFileFrom>/d/A&amp;amp;B.docx</m_sFileFrom>"));
        assert!(xml.contains("<m_sFileTo>/d/A&amp;amp;B.pdf</m_sFileTo>"));
    }

    #[test]
    fn pdf_change_params_ask_for_the_changes_and_no_extra_files() {
        let extra = pdf_changes_params("/t/R&D/tmp");
        let xml = pdf_params_xml_with("/t/src.pdf", "/t/out.pdf", "/f", "/f/AllFonts.js", &extra);
        assert!(xml.contains("<m_nFormatFrom>513</m_nFormatFrom>"));
        assert!(xml.contains("<m_bFromChanges>true</m_bFromChanges>"));
        assert!(xml.contains("<m_bDontSaveAdditional>true</m_bDontSaveAdditional>"));
        let expected_end = format!(
            "<m_sTempDir>/t/R&amp;D/tmp</m_sTempDir>\n{}</TaskQueueDataConvert>",
            x2t_options_xml()
        );
        assert!(xml.ends_with(&expected_end), "{}", xml);
    }

    // An app folder whose binaries/ holds the bundled font list and a font
    // cache folder that is still empty.
    fn app_without_cache(
        name: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = scratch_dir(name);
        let binaries = root.join("app/binaries");
        touch(&binaries.join("AllFonts.js"));
        let fontdata = root.join("temp/fontdata");
        std::fs::create_dir_all(&fontdata).unwrap();
        (root, binaries, fontdata)
    }

    fn logged(run: impl FnOnce(&dyn Fn(&str))) -> Vec<String> {
        let lines = std::cell::RefCell::new(Vec::new());
        run(&|line: &str| lines.borrow_mut().push(line.to_string()));
        lines.into_inner()
    }

    #[test]
    fn the_bundled_font_list_is_never_the_pdf_list() {
        let (root, _binaries, fontdata) = app_without_cache("bundled");

        let error = pdf_font_list(&fontdata).expect_err("only the font cache makes a good PDF");

        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pdf_without_x2t_logs_the_discovery_failure() {
        let (root, binaries, fontdata) = app_without_cache("pdf-no-x2t");
        let temp = root.join("temp/1-1");

        let mut result = None;
        let lines = logged(|log| {
            result = Some(convert_to_pdf(
                &binaries, None, "/d/in.docx", "/d/out.pdf", &temp, &fontdata, log,
            ));
        });

        assert_eq!(result, Some(Err("x2t executable not found".to_string())));
        assert_eq!(
            lines,
            vec![
                format!("start platform={} output=out.pdf", std::env::consts::OS),
                "x2t discovery failed: x2t executable not found".to_string(),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // The fallback to binaries/AllFonts.js is gone: without the cache the
    // export stops before x2t runs, with the error the frontend recognises.
    #[test]
    fn a_pdf_without_the_font_cache_stops_even_with_the_bundled_list() {
        let (root, binaries, fontdata) = app_without_cache("pdf-no-cache");
        touch(&binaries.join(x2t_name()));
        let temp = root.join("temp/1-1");

        let mut result = None;
        let lines = logged(|log| {
            result = Some(convert_to_pdf(
                &binaries, None, "/d/in.docx", "/d/out.pdf", &temp, &fontdata, log,
            ));
        });

        let error = format!(
            "{}: {} is missing",
            FONT_CACHE_ERROR,
            fontdata.join("AllFonts.js").display()
        );
        assert_eq!(result, Some(Err(error.clone())));
        assert_eq!(
            lines,
            vec![
                format!("start platform={} output=out.pdf", std::env::consts::OS),
                format!("setup failed: {}", error),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // An installed app folder: binaries/ with x2t and its fonts, editors/ with
    // the DoctRenderer files, and a font cache next to it.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn installed_app(
        name: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = scratch_dir(name);
        let binaries = root.join("app/binaries");
        touch(&binaries.join(x2t_name()));
        touch(&binaries.join("fonts/LiberationSerif-Regular.ttf"));
        for relative in DOCTRENDERER_EDITOR_RESOURCES {
            touch(&root.join("app/editors").join(relative));
        }
        let fontdata = root.join("temp/fontdata");
        std::fs::create_dir_all(&fontdata).unwrap();
        std::fs::write(fontdata.join("AllFonts.js"), b"cache").unwrap();
        std::fs::write(fontdata.join("font_selection.bin"), b"selection").unwrap();
        (root, binaries, fontdata)
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_runs_x2t_in_place_with_the_cache_copied_in() {
        let (root, binaries, fontdata) = installed_app("win-direct");
        let app = root.join("app");
        let editors = app.join("editors");
        touch(&editors.join("sdkjs/common/AllFonts.js"));
        let fonts = binaries.join("fonts");

        let run = setup_windows_direct(
            &binaries,
            &binaries.join("x2t.exe"),
            &fonts,
            &fontdata,
            &editors,
            &fontdata.join("AllFonts.js"),
        );

        assert_eq!(
            run,
            Ok((
                binaries.clone(),
                binaries.join("x2t.exe"),
                binaries.join("AllFonts.js"),
                fonts.clone()
            ))
        );
        let read = |p: std::path::PathBuf| std::fs::read(p).unwrap();
        assert_eq!(read(binaries.join("AllFonts.js")), b"cache");
        assert_eq!(read(editors.join("sdkjs/common/AllFonts.js")), b"cache");
        assert_eq!(read(binaries.join("font_selection.bin")), b"selection");
        assert_eq!(read(fonts.join("font_selection.bin")), b"selection");
        assert_eq!(
            read(binaries.join("DoctRenderer.config")),
            DOCTRENDERER_CONFIG_PARENT.as_bytes()
        );
        assert!(!app.join("DoctRenderer.config").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // editors/.../AllFonts.js is only refreshed where the install has one.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_does_not_add_an_editors_font_list() {
        let (root, binaries, fontdata) = installed_app("win-no-editors-list");
        let editors = root.join("app/editors");

        setup_windows_direct(
            &binaries,
            &binaries.join("x2t.exe"),
            &binaries.join("fonts"),
            &fontdata,
            &editors,
            &fontdata.join("AllFonts.js"),
        )
        .unwrap();

        assert!(!editors.join("sdkjs/common/AllFonts.js").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // A .app with or without Contents/Frameworks, its font cache and the
    // instance temp folder the work dir goes in.
    #[cfg(target_os = "macos")]
    fn macos_app(name: &str, with_frameworks: bool) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = scratch_dir(name);
        let contents = root.join("Euro-Office-Lite.app/Contents");
        let resources = contents.join("Resources");
        touch(&resources.join("binaries/x2t"));
        touch(&resources.join("binaries/fonts/LiberationSerif-Regular.ttf"));
        for relative in DOCTRENDERER_EDITOR_RESOURCES {
            touch(&resources.join("editors").join(relative));
        }
        if with_frameworks {
            touch(&contents.join("Frameworks/kernel.framework/Versions/A/kernel"));
        }
        touch(&root.join("temp/fontdata/AllFonts.js"));
        (root, contents)
    }

    #[cfg(target_os = "macos")]
    fn macos_work_dir(root: &std::path::Path, contents: &std::path::Path) -> std::path::PathBuf {
        let binaries = contents.join("Resources/binaries");
        let fontdata = root.join("temp/fontdata");
        setup_macos_workdir(
            &binaries,
            &binaries.join("x2t"),
            &binaries.join("fonts"),
            &fontdata,
            &contents.join("Resources/editors"),
            &fontdata.join("AllFonts.js"),
            &root.join("temp"),
        )
        .unwrap();
        root.join("temp/x2t-workdir")
    }

    // The work copy of x2t is work_dir/binaries/x2t, so its
    // @executable_path/../Frameworks is work_dir/Frameworks.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_work_dir_links_the_app_frameworks() {
        let (root, contents) = macos_app("mac-fw-link", true);

        let work_dir = macos_work_dir(&root, &contents);

        assert_eq!(
            std::fs::read_link(work_dir.join("Frameworks")).unwrap(),
            contents.join("Frameworks")
        );
        assert!(work_dir.join("Frameworks/kernel.framework").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_work_dir_has_no_frameworks_link_without_frameworks() {
        let (root, contents) = macos_app("mac-fw-none", false);

        let work_dir = macos_work_dir(&root, &contents);

        assert!(std::fs::symlink_metadata(work_dir.join("Frameworks")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_builds_a_work_dir_of_links_and_copies() {
        let (root, binaries, fontdata) = installed_app("linux-workdir");
        let libraries = ["libkernel.so", "libicuuc.so.58", "icudt58l.dat", "package.config"];
        for name in libraries {
            touch(&binaries.join(name));
        }
        touch(&binaries.join("notes.txt"));
        let editors = root.join("app/editors");
        let fonts = binaries.join("fonts");
        touch(&fonts.join("DejaVuSans.ttf"));
        // Only files are linked; a folder under fonts/ is left out, as on macOS.
        std::fs::create_dir_all(fonts.join("extra")).unwrap();
        let temp = root.join("temp/1-1");

        let run = setup_linux_workdir(
            &binaries,
            &binaries.join("x2t"),
            &fonts,
            &fontdata,
            &editors,
            &fontdata.join("AllFonts.js"),
            &temp,
        );

        let work = temp.join("x2t-workdir");
        let work_binaries = work.join("binaries");
        assert_eq!(
            run,
            Ok((
                work_binaries.clone(),
                work_binaries.join("x2t"),
                work_binaries.join("AllFonts.js"),
                work_binaries.join("fonts")
            ))
        );
        let link = |p: std::path::PathBuf| std::fs::read_link(p).ok();
        assert_eq!(link(work_binaries.join("x2t")), Some(binaries.join("x2t")));
        for name in libraries {
            assert_eq!(link(work_binaries.join(name)), Some(binaries.join(name)), "{}", name);
        }
        assert!(!work_binaries.join("notes.txt").exists());
        // fonts/ is a real folder with one link per bundled font and the cache's
        // font_selection.bin, which x2t reads from m_sFontDir for the PDF.
        let work_fonts = work_binaries.join("fonts");
        let fonts_meta = std::fs::symlink_metadata(&work_fonts).unwrap();
        assert!(fonts_meta.is_dir() && !fonts_meta.file_type().is_symlink());
        for name in ["LiberationSerif-Regular.ttf", "DejaVuSans.ttf"] {
            assert_eq!(link(work_fonts.join(name)), Some(fonts.join(name)), "{}", name);
        }
        assert!(!work_fonts.join("extra").exists());
        assert_eq!(std::fs::read(work_fonts.join("font_selection.bin")).unwrap(), b"selection");
        assert_eq!(link(work_fonts.join("font_selection.bin")), None);
        for relative in DOCTRENDERER_EDITOR_RESOURCES {
            assert_eq!(
                link(work.join("editors").join(relative)),
                Some(editors.join(relative)),
                "{}",
                relative
            );
        }
        let read = |p: std::path::PathBuf| std::fs::read(p).unwrap();
        assert_eq!(read(work_binaries.join("AllFonts.js")), b"cache");
        assert_eq!(read(work.join("editors/sdkjs/common/AllFonts.js")), b"cache");
        assert_eq!(read(work_binaries.join("font_selection.bin")), b"selection");
        assert_eq!(
            read(work_binaries.join("DoctRenderer.config")),
            DOCTRENDERER_CONFIG_PARENT.as_bytes()
        );
        assert!(work.join("dictionaries").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    // An install that ships its own font_selection.bin: the work folder gets
    // the cache's, and the installed one is never written.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_work_fonts_take_the_cache_selection_over_the_installed_one() {
        let (root, binaries, fontdata) = installed_app("linux-fontsel");
        let fonts = binaries.join("fonts");
        std::fs::write(fonts.join("font_selection.bin"), b"installed").unwrap();
        let temp = root.join("temp/1-1");

        setup_linux_workdir(
            &binaries,
            &binaries.join("x2t"),
            &fonts,
            &fontdata,
            &root.join("app/editors"),
            &fontdata.join("AllFonts.js"),
            &temp,
        )
        .unwrap();

        let work_fonts = temp.join("x2t-workdir/binaries/fonts");
        assert_eq!(std::fs::read_link(work_fonts.join("font_selection.bin")).ok(), None);
        assert_eq!(std::fs::read(work_fonts.join("font_selection.bin")).unwrap(), b"selection");
        assert_eq!(std::fs::read(fonts.join("font_selection.bin")).unwrap(), b"installed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plain_paths_give_the_same_convert_params_as_before() {
        let xml = convert_params_xml(
            r"C:\Users\u\Doc.docx",
            r"C:\Temp\eo\Editor.bin",
            8192,
            r"C:\Temp\eo",
            "<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>\n",
        );
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<TaskQueueDataConvert xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n                      \
xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n\
<m_sFileFrom>C:/Users/u/Doc.docx</m_sFileFrom>\n\
<m_sFileTo>C:/Temp/eo/Editor.bin</m_sFileTo>\n\
<m_nFormatTo>8192</m_nFormatTo>\n\
<m_sTempDir>C:/Temp/eo</m_sTempDir>\n\
<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>\n\
<options>\n\
<allowPrivateIP>false</allowPrivateIP>\n\
<allowNetworkRequest>false</allowNetworkRequest>\n\
</options>\n\
</TaskQueueDataConvert>"
        );
    }

    #[test]
    fn convert_params_escape_every_path_in_name_and_folder() {
        let xml = convert_params_xml(
            "/home/u/Q&A/R&D.docx",
            "/tmp/a<b>/s<r>.docx",
            65,
            "/tmp/T&C",
            "",
        );
        assert!(xml.contains("<m_sFileFrom>/home/u/Q&amp;A/R&amp;D.docx</m_sFileFrom>"));
        assert!(xml.contains("<m_sFileTo>/tmp/a&lt;b&gt;/s&lt;r&gt;.docx</m_sFileTo>"));
        assert!(xml.contains("<m_sTempDir>/tmp/T&amp;C</m_sTempDir>"));
        assert!(!xml.contains("Q&A") && !xml.contains("a<b>") && !xml.contains("T&C"));
    }

    #[test]
    fn convert_params_keep_the_text_options_as_markup() {
        let extra =
            "<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>\n<m_nCsvDelimiter>2</m_nCsvDelimiter>\n";
        let xml = convert_params_xml("/d/A&amp;B.csv", "/t/Editor.bin", 8192, "/t", extra);
        assert!(xml.contains(extra));
        assert!(xml.contains("<m_sFileFrom>/d/A&amp;amp;B.csv</m_sFileFrom>"));
    }

    #[test]
    fn the_convert_env_names_the_app_and_adds_the_library_folder_on_linux_only() {
        use std::ffi::OsString;
        let library_dir = "/opt/Euro Office/binaries";
        let env = convert_env(std::path::Path::new(library_dir));
        let app_name = ("APPLICATION_NAME", OsString::from(APPLICATION_NAME));
        if cfg!(target_os = "linux") {
            let library_path = ("LD_LIBRARY_PATH", OsString::from(library_dir));
            assert_eq!(env, vec![app_name, library_path]);
        } else {
            assert_eq!(env, vec![app_name]);
        }
    }

    // The value of `name` that `cmd` sets for x2t, if it sets one.
    pub(super) fn env_value<'a>(
        cmd: &'a std::process::Command,
        name: &str,
    ) -> Option<&'a std::ffi::OsStr> {
        cmd.get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value)
    }

    fn pdf_test_command() -> std::process::Command {
        pdf_command(
            std::path::Path::new("/tmp/eo/work/x2t"),
            std::path::Path::new("/tmp/eo/work"),
            std::path::Path::new("/tmp/eo/pdf.xml"),
            std::path::Path::new("/opt/Euro Office/binaries"),
        )
    }

    #[test]
    fn the_pdf_export_names_the_app() {
        let cmd = pdf_test_command();
        assert_eq!(
            env_value(&cmd, "APPLICATION_NAME"),
            Some(std::ffi::OsStr::new(APPLICATION_NAME))
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_pdf_export_loads_the_work_dir_libraries_first() {
        let cmd = pdf_test_command();
        assert_eq!(
            env_value(&cmd, "LD_LIBRARY_PATH"),
            Some(std::ffi::OsStr::new("/tmp/eo/work:/opt/Euro Office/binaries"))
        );
    }

    #[test]
    fn convert_params_without_text_options_have_no_extra_elements() {
        let xml = convert_params_xml("/d/in.docx", "/t/Editor.bin", 8192, "/t", "");
        let expected_end = format!("<m_sTempDir>/t</m_sTempDir>\n{}</TaskQueueDataConvert>", x2t_options_xml());
        assert!(xml.ends_with(&expected_end), "{}", xml);
    }

    #[test]
    fn the_options_block_turns_off_network_and_outside_files() {
        let options = x2t_options_xml();
        assert!(options.starts_with("<options>"), "{}", options);
        assert!(options.contains("<allowPrivateIP>false</allowPrivateIP>"), "{}", options);
        assert!(options.contains("<allowNetworkRequest>false</allowNetworkRequest>"), "{}", options);
        assert!(options.ends_with("</options>\n"), "{}", options);
    }

    #[test]
    fn pdf_params_end_with_the_options_block() {
        let xml = pdf_params_xml("/d/in.docx", "/d/out.pdf", "/f", "/f/AllFonts.js");
        let expected_end = format!("{}</TaskQueueDataConvert>", x2t_options_xml());
        assert!(xml.ends_with(&expected_end), "{}", xml);
    }

    #[test]
    fn convert_params_end_with_the_options_block() {
        let expected_end = format!("{}</TaskQueueDataConvert>", x2t_options_xml());
        let plain = convert_params_xml("/d/in.docx", "/t/Editor.bin", 8192, "/t", "");
        assert!(plain.ends_with(&expected_end), "{}", plain);
        let text = "<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>\n";
        let with_text = convert_params_xml("/d/in.csv", "/t/Editor.bin", 8192, "/t", text);
        assert!(with_text.ends_with(&expected_end), "{}", with_text);
    }

    #[test]
    fn every_params_xml_has_the_options_block_once() {
        let all = [
            pdf_params_xml("/d/in.docx", "/d/out.pdf", "/f", "/f/AllFonts.js"),
            pdf_params_xml_with("/d/in.pdf", "/d/out.pdf", "/f", "/f/AllFonts.js", &pdf_changes_params("/t")),
            convert_params_xml("/d/in.docx", "/t/Editor.bin", 8192, "/t", ""),
            convert_params_xml("/t/Editor.bin", "/d/out.docx", 65, "/t", ""),
        ];
        for xml in all {
            assert_eq!(xml.matches("<options>").count(), 1, "{}", xml);
            assert_eq!(xml.matches("<allowPrivateIP>false</allowPrivateIP>").count(), 1, "{}", xml);
            assert_eq!(xml.matches("<allowNetworkRequest>false</allowNetworkRequest>").count(), 1, "{}", xml);
        }
    }

    const HTML_HEAD: &[u8] = b"<html><body>";
    const OOXML_EXTENSIONS: &[&str] = &[
        "docx", "docm", "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "pptx", "pptm", "potx",
        "potm", "ppsx", "ppsm",
    ];
    const ODF_EXTENSIONS: &[&str] = &["odt", "ods", "odp", "ott", "ots", "otp"];

    fn matches(name: &str, head: &[u8]) -> bool {
        content_matches_extension(std::path::Path::new(name), head)
    }

    #[test]
    fn container_extensions_accept_a_zip() {
        for ext in OOXML_EXTENSIONS.iter().chain(ODF_EXTENSIONS) {
            assert!(matches(&format!("/d/a.{ext}"), b"PK\x03\x04rest"), "{ext}");
        }
    }

    #[test]
    fn ooxml_extensions_accept_a_cfb_and_odf_ones_do_not() {
        for ext in OOXML_EXTENSIONS {
            assert!(matches(&format!("/d/a.{ext}"), CFB_SIGNATURE), "{ext}");
        }
        for ext in ODF_EXTENSIONS {
            assert!(!matches(&format!("/d/a.{ext}"), CFB_SIGNATURE), "{ext}");
        }
    }

    #[test]
    fn container_extensions_reject_html_and_an_empty_file() {
        for ext in OOXML_EXTENSIONS.iter().chain(ODF_EXTENSIONS) {
            assert!(!matches(&format!("/d/a.{ext}"), HTML_HEAD), "{ext}");
            assert!(!matches(&format!("/d/a.{ext}"), b""), "{ext}");
        }
    }

    #[test]
    fn container_extension_check_ignores_case() {
        assert!(!matches("/d/A.DOCX", HTML_HEAD));
        assert!(matches("/d/A.DOCX", b"PK\x03\x04"));
    }

    #[test]
    fn text_formats_are_left_to_x2t() {
        for name in ["/d/a.rtf", "/d/a.txt", "/d/a.csv", "/d/a"] {
            assert!(matches(name, HTML_HEAD), "{name}");
        }
    }

    #[test]
    fn legacy_binary_extensions_accept_a_cfb_or_a_zip() {
        for name in ["/d/a.doc", "/d/a.xls", "/d/a.ppt", "/d/A.DOC"] {
            assert!(matches(name, CFB_SIGNATURE), "{name}");
            assert!(matches(name, b"PK\x03\x04rest"), "{name}");
            assert!(!matches(name, b"just some text\n"), "{name}");
            assert!(!matches(name, HTML_HEAD), "{name}");
            assert!(!matches(name, b""), "{name}");
        }
    }

    #[test]
    fn legacy_extensions_reject_html() {
        let doctype = b"<!DOCTYPE html><html><body>";
        let spaced = b"\xEF\xBB\xBF\n<html xmlns:x=\"urn:schemas-microsoft-com:office:excel\">";
        for name in ["/d/a.doc", "/d/a.xls", "/d/a.ppt"] {
            assert!(!matches(name, HTML_HEAD), "{name}");
            assert!(!matches(name, doctype), "{name}");
            assert!(!matches(name, spaced), "{name}");
        }
    }

    #[test]
    fn an_rtf_doc_is_accepted_with_or_without_a_bom() {
        assert!(matches("/d/a.doc", b"{\\rtf1\\ansi\\deff0"));
        assert!(matches("/d/a.doc", b"\xEF\xBB\xBF{\\rtf1\\ansi"));
        assert!(matches("/d/a.doc", b"\xEF\xBB\xBF\r\n {\\rtf1\\ansi"));
    }

    #[test]
    fn rtf_is_only_accepted_as_doc() {
        assert!(!matches("/d/a.xls", b"{\\rtf1\\ansi"));
        assert!(!matches("/d/a.ppt", b"{\\rtf1\\ansi"));
    }

    // The first bytes Excel 2003 writes for "XML Spreadsheet 2003".
    const EXCEL_2003_XML_HEAD: &[u8] = b"<?xml version=\"1.0\"?>\r\n\
<?mso-application progid=\"Excel.Sheet\"?>\r\n\
<Workbook xmlns=\"urn:schemas-microsoft-com:office:spreadsheet\"\r\n\
 xmlns:o=\"urn:schemas-microsoft-com:office:office\"\r\n\
 xmlns:x=\"urn:schemas-microsoft-com:office:excel\"\r\n\
 xmlns:ss=\"urn:schemas-microsoft-com:office:spreadsheet\"\r\n\
 xmlns:html=\"http://www.w3.org/TR/REC-html40\">\r\n";

    #[test]
    fn an_excel_2003_xml_xls_is_accepted() {
        assert!(matches("/d/a.xls", EXCEL_2003_XML_HEAD));
        assert!(matches("/d/A.XLS", EXCEL_2003_XML_HEAD));
    }

    #[test]
    fn spreadsheet_ml_markers_fit_in_the_bytes_read_before_conversion() {
        let head = &EXCEL_2003_XML_HEAD[..EXCEL_2003_XML_HEAD.len().min(256)];
        assert!(matches("/d/a.xls", head));
    }

    #[test]
    fn spreadsheet_ml_is_recognised_by_either_marker() {
        let only_namespace = b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Workbook xmlns=\"urn:schemas-microsoft-com:office:spreadsheet\">";
        let only_progid = b"  <?xml version=\"1.0\"?><?mso-application progid=\"Excel.Sheet\"?><Workbook>";
        assert!(matches("/d/a.xls", only_namespace));
        assert!(matches("/d/a.xls", only_progid));
    }

    #[test]
    fn spreadsheet_ml_is_only_accepted_as_xls() {
        assert!(!matches("/d/a.doc", EXCEL_2003_XML_HEAD));
        assert!(!matches("/d/a.ppt", EXCEL_2003_XML_HEAD));
    }

    #[test]
    fn an_xml_xls_that_is_not_spreadsheet_ml_is_rejected() {
        let xhtml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\">\n\
<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><img src=\"/etc/passwd\"/></body></html>";
        let other_xml = b"<?xml version=\"1.0\"?><root/>";
        assert!(!matches("/d/a.xls", xhtml));
        assert!(!matches("/d/a.xls", other_xml));
    }

    #[test]
    fn spreadsheet_ml_markers_without_an_xml_declaration_are_rejected() {
        let html = b"<html><body>urn:schemas-microsoft-com:office:spreadsheet</body></html>";
        assert!(!matches("/d/a.xls", html));
    }

    #[test]
    fn a_text_file_renamed_to_doc_is_rejected() {
        assert!(!matches("/d/fake.doc", b"Hello, this is plain text.\n"));
    }

    #[test]
    fn xlsb_needs_a_zip() {
        assert!(matches("/d/a.xlsb", b"PK\x03\x04rest"));
        assert!(!matches("/d/a.xlsb", CFB_SIGNATURE));
        assert!(!matches("/d/a.xlsb", b"just some text\n"));
    }

    #[test]
    fn flat_odf_extensions_need_an_xml_declaration() {
        let with_bom = b"\xEF\xBB\xBF<?xml version=\"1.0\"?>";
        let with_space = b" \r\n\t<?xml version=\"1.0\"?>";
        let with_bom_and_space = b"\xEF\xBB\xBF\n<?xml version=\"1.0\"?>";
        for ext in ["fodt", "fods", "fodp", "FODT"] {
            let name = format!("/d/a.{ext}");
            assert!(matches(&name, b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>"), "{ext}");
            assert!(matches(&name, with_bom), "{ext}");
            assert!(matches(&name, with_space), "{ext}");
            assert!(matches(&name, with_bom_and_space), "{ext}");
            assert!(!matches(&name, b"just some text\n"), "{ext}");
            assert!(!matches(&name, b"PK\x03\x04rest"), "{ext}");
            assert!(!matches(&name, b"\xEF\xBB\xBF"), "{ext}");
            assert!(!matches(&name, b""), "{ext}");
        }
    }

    const WORD97_DOC: &[u8] = include_bytes!("../../tests/fixtures/word97.doc");
    const ENCRYPTED_DOCX: &[u8] = include_bytes!("../../tests/fixtures/encrypted-test.docx");

    #[test]
    fn real_fixtures_start_with_the_cfb_signature() {
        assert!(WORD97_DOC.starts_with(CFB_SIGNATURE));
        assert!(ENCRYPTED_DOCX.starts_with(CFB_SIGNATURE));
    }

    #[test]
    fn a_real_word_97_doc_is_accepted() {
        assert!(matches("/d/a.doc", WORD97_DOC));
    }

    #[test]
    fn a_real_encrypted_docx_is_accepted_as_docx_but_not_as_odt() {
        assert!(matches("/d/a.docx", ENCRYPTED_DOCX));
        assert!(!matches("/d/a.odt", ENCRYPTED_DOCX));
    }

    #[test]
    fn detect_format_maps_pdf_to_the_x2t_export_id() {
        // 513 is the value both PDF paths branch on before calling the converter.
        assert_eq!(detect_format(std::path::Path::new("/tmp/out.pdf")), 513);
        assert_eq!(detect_format(std::path::Path::new("/tmp/out.docx")), 65);
        assert_eq!(detect_format(std::path::Path::new("/tmp/out.unknown")), 0);
    }

    #[test]
    fn detect_format_ignores_the_case_of_the_extension() {
        assert_eq!(detect_format(std::path::Path::new("/tmp/REPORT.DOCX")), 65);
        assert_eq!(detect_format(std::path::Path::new("/tmp/Budget.Xls")), 258);
        assert_eq!(detect_format(std::path::Path::new("/tmp/SLIDES.PDF")), 513);
    }

    #[test]
    fn detect_format_knows_the_office_variants_the_open_dialog_offers() {
        let cases = [
            ("docm", 75), ("dotx", 76), ("dotm", 77), ("fodt", 78), ("ott", 79),
            ("xlsm", 261), ("xltx", 262), ("xlsb", 264), ("fods", 265), ("ots", 266),
            ("ppsx", 132), ("pptm", 133), ("potx", 135), ("fodp", 137), ("otp", 138),
        ];
        for (ext, code) in cases {
            let path = std::path::PathBuf::from(format!("/tmp/a.{}", ext));
            assert_eq!(detect_format(&path), code, "{}", ext);
        }
    }

    #[test]
    fn blank_template_covers_the_three_editors_and_nothing_else() {
        let template = |name: &str| blank_template_for(std::path::Path::new(name));

        assert_eq!(template("/tmp/empty.docx"), Some("templates/blank.docx"));
        // The editor comes from the extension, and ODF and RTF are edited in the
        // same one: the format is only decided when the document is written.
        assert_eq!(template("/tmp/empty.odt"), Some("templates/blank.docx"));
        assert_eq!(template("/tmp/EMPTY.ODS"), Some("templates/blank.xlsx"));
        assert_eq!(template("/tmp/empty.odp"), Some("templates/blank.pptx"));

        // Out of scope on purpose: these keep failing the open instead.
        for unsupported in ["/tmp/empty.txt", "/tmp/empty.csv", "/tmp/empty.pdf",
                            "/tmp/empty.doc", "/tmp/empty.xyz", "/tmp/empty"] {
            assert_eq!(template(unsupported), None, "{} must not open blank", unsupported);
        }
    }

    #[test]
    fn the_font_scan_without_x2t_logs_why_and_fails() {
        let root = scratch_dir("fonts-no-x2t");
        let binaries = root.join("app/binaries");
        touch(&binaries.join("fonts/LiberationSans-Regular.ttf"));
        let fontdata = root.join("temp/fontdata.new-1-1");

        let mut generated = None;
        let lines = logged(|log| generated = Some(generate_fonts(&fontdata, &binaries, None, log)));

        assert_eq!(generated, Some(false));
        assert_eq!(
            lines,
            vec![
                "First-run font generation starting...",
                "ERROR: x2t executable not found, cannot generate fonts",
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

// The walker only has a body on Linux, so the assertions are gated the same way
// even though the symlink APIs they use exist on every unix.
#[cfg(all(test, target_os = "linux"))]
mod symlink_tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("eo-fonts-{}-{}", name, unique));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup(name: &str) -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new(name);
        let root = temp.join("root");
        let store = temp.join("store");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&store).expect("create store");
        (temp, root, store)
    }

    fn resolve(root: &PathBuf) -> Vec<PathBuf> {
        symlink_font_target_dirs(&[root.clone()])
    }

    #[test]
    fn symlink_to_font_file_yields_target_parent() {
        let (_temp, root, store) = setup("file");
        std::fs::write(store.join("Sans.ttf"), b"font").expect("write font");
        symlink(store.join("Sans.ttf"), root.join("Sans.ttf")).expect("link");

        let expected = std::fs::canonicalize(&store).expect("canonicalize store");
        assert_eq!(resolve(&root), vec![expected]);
    }

    #[test]
    fn symlink_to_directory_yields_that_directory() {
        let (_temp, root, store) = setup("dir");
        std::fs::write(store.join("Sans.otf"), b"font").expect("write font");
        symlink(&store, root.join("linked")).expect("link");

        let expected = std::fs::canonicalize(&store).expect("canonicalize store");
        assert_eq!(resolve(&root), vec![expected]);
    }

    #[test]
    fn real_font_file_yields_nothing() {
        let (_temp, root, _store) = setup("real");
        std::fs::write(root.join("Sans.ttf"), b"font").expect("write font");

        assert!(resolve(&root).is_empty());
    }

    #[test]
    fn broken_symlink_yields_nothing() {
        let (_temp, root, store) = setup("broken");
        symlink(store.join("Missing.ttf"), root.join("Missing.ttf")).expect("link");

        assert!(resolve(&root).is_empty());
    }

    #[test]
    fn symlink_to_non_font_file_yields_nothing() {
        let (_temp, root, store) = setup("nonfont");
        std::fs::write(store.join("notes.txt"), b"text").expect("write file");
        symlink(store.join("notes.txt"), root.join("notes.txt")).expect("link");

        assert!(resolve(&root).is_empty());
    }

    #[test]
    fn two_symlinks_to_same_target_yield_one_directory() {
        let (_temp, root, store) = setup("dedup");
        std::fs::write(store.join("Sans.ttf"), b"font").expect("write font");
        std::fs::write(store.join("Serif.TTF"), b"font").expect("write font");
        symlink(store.join("Sans.ttf"), root.join("Sans.ttf")).expect("link");
        symlink(store.join("Serif.TTF"), root.join("Serif.TTF")).expect("link");

        let expected = std::fs::canonicalize(&store).expect("canonicalize store");
        assert_eq!(resolve(&root), vec![expected]);
    }

    #[test]
    fn target_inside_the_root_yields_nothing() {
        let (_temp, root, _store) = setup("inside");
        let other = root.join("b");
        std::fs::create_dir_all(&other).expect("create subdir");
        std::fs::create_dir_all(root.join("a")).expect("create subdir");
        std::fs::write(other.join("Sans.ttf"), b"font").expect("write font");
        symlink(other.join("Sans.ttf"), root.join("a/Sans.ttf")).expect("link");

        assert!(resolve(&root).is_empty());
    }

    #[test]
    fn nested_directories_are_walked() {
        let (_temp, root, store) = setup("nested");
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).expect("create nested");
        std::fs::write(store.join("Sans.ttf"), b"font").expect("write font");
        symlink(store.join("Sans.ttf"), nested.join("Sans.ttf")).expect("link");

        let expected = std::fs::canonicalize(&store).expect("canonicalize store");
        assert_eq!(resolve(&root), vec![expected]);
    }
}

#[cfg(test)]
mod font_scan_tests {
    use super::tests::env_value;
    use super::{font_scan_command, APPLICATION_NAME};
    use std::path::{Path, PathBuf};

    #[test]
    fn the_font_scan_names_the_app() {
        let cmd = font_scan_command(
            Path::new("/opt/Euro Office/binaries/x2t"),
            Path::new("/tmp/fontdata"),
            Path::new("/opt/Euro Office/binaries"),
        );
        assert_eq!(
            env_value(&cmd, "APPLICATION_NAME"),
            Some(std::ffi::OsStr::new(APPLICATION_NAME))
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_font_scan_loads_the_libraries_of_the_binaries_folder() {
        let cmd = font_scan_command(
            Path::new("/opt/Euro Office/binaries/x2t"),
            Path::new("/tmp/fontdata"),
            Path::new("/opt/Euro Office/binaries"),
        );
        assert_eq!(
            env_value(&cmd, "LD_LIBRARY_PATH"),
            Some(std::ffi::OsStr::new("/opt/Euro Office/binaries"))
        );
    }

    // resource_dir() on Windows: the scan used to receive this spelling and
    // x2t wrote it into the cache as /?/C:/... and \\?\C:\... An install on a
    // network share comes as \\?\UNC\server\... and must stay absolute.
    #[test]
    fn the_font_scan_never_sees_the_extended_prefix() {
        let cases = [
            (r"\\?\C:\Apps\Euro-Office-Lite", r"C:\Apps\Euro-Office-Lite"),
            (
                r"\\?\UNC\server\share\Euro-Office-Lite",
                r"\\server\share\Euro-Office-Lite",
            ),
        ];
        for (given, plain) in cases {
            let given = PathBuf::from(given);
            let plain = PathBuf::from(plain);
            let cmd = font_scan_command(
                &given.join("x2t.exe"),
                Path::new(r"C:\Temp\euro-office-lite\fontdata.new-1-2"),
                &given.join("binaries"),
            );

            let args: Vec<PathBuf> = cmd.get_args().map(PathBuf::from).collect();
            assert_eq!(
                args,
                vec![
                    PathBuf::from("-create-allfonts"),
                    PathBuf::from(r"C:\Temp\euro-office-lite\fontdata.new-1-2"),
                    plain.join("binaries").join("fonts"),
                ]
            );
            assert_eq!(Path::new(cmd.get_program()), plain.join("x2t.exe"));
            assert_eq!(cmd.get_current_dir(), Some(plain.join("binaries").as_path()));
        }
    }
}

#[cfg(test)]
mod platform_font_dir_tests {
    use super::{macos_font_dirs, windows_font_dirs};
    use std::path::{Path, PathBuf};

    #[test]
    fn windows_dirs_come_from_windir_and_local_app_data() {
        let dirs = windows_font_dirs(
            Some(Path::new(r"C:\Windows")),
            Some(Path::new(r"C:\Users\ana\AppData\Local")),
        );
        assert_eq!(
            dirs,
            vec![
                Path::new(r"C:\Windows").join("Fonts"),
                Path::new(r"C:\Users\ana\AppData\Local")
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
                Path::new(r"C:\Users\ana\AppData\Local")
                    .join("Microsoft")
                    .join("FontCache")
                    .join("4")
                    .join("CloudFonts"),
            ]
        );
    }

    #[test]
    fn windows_dirs_without_the_variables_are_empty() {
        assert!(windows_font_dirs(None, None).is_empty());
        assert_eq!(
            windows_font_dirs(Some(Path::new("W")), None),
            vec![Path::new("W").join("Fonts")]
        );
    }

    #[test]
    fn macos_dirs_add_the_user_folder_when_home_is_known() {
        let system = vec![
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/Library/Fonts"),
        ];
        assert_eq!(macos_font_dirs(None), system);
        let mut with_home = system.clone();
        with_home.push(Path::new("/Users/ana").join("Library").join("Fonts"));
        assert_eq!(macos_font_dirs(Some(Path::new("/Users/ana"))), with_home);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scan_dirs_hold_no_windows_or_macos_folders() {
        use super::font_scan_dirs;
        let dirs = font_scan_dirs(Path::new("/opt/eo/binaries"));
        assert_eq!(dirs[0], Path::new("/opt/eo/binaries").join("fonts"));
        for dir in &dirs {
            assert!(!dir.starts_with("/System/Library"), "{}", dir.display());
            assert!(!dir.starts_with("/Library"), "{}", dir.display());
            assert!(!dir.ends_with("Library/Fonts"), "{}", dir.display());
        }
    }
}
