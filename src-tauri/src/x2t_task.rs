// What the app does to run x2t that needs no Tauri. The file uses only std and
// never names crate:: or tauri, so another crate can include it with #[path]
// and run x2t with exactly the app's code. Log lines go out through a `log`
// parameter, without the prefix the app adds.

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

// The x2t the PDF export and the font scan run: the first match in binaries/,
// then its parent, then Contents/MacOS, then `exe_dir` (the folder of the
// app's executable). On Linux and macOS binaries/ holds its own copy of x2t,
// so these two never run the sidecar that the other conversions use.
pub fn find_x2t(
    binaries_dir: &std::path::Path,
    exe_dir: Option<&std::path::Path>,
) -> Result<std::path::PathBuf, String> {
    let mut search_dirs = vec![
        binaries_dir.to_path_buf(),
        binaries_dir.parent().unwrap_or(binaries_dir).to_path_buf(),
    ];
    if let Some(resources) = binaries_dir.parent() {
        if let Some(contents) = resources.parent() {
            let macos_dir = contents.join("MacOS");
            if macos_dir.exists() {
                search_dirs.push(macos_dir);
            }
        }
    }
    if let Some(exe_dir) = exe_dir {
        search_dirs.push(exe_dir.to_path_buf());
    }
    for dir in &search_dirs {
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

const DOCTRENDERER_CONFIG_CURRENT: &str = r#"<Settings>
<file>./editors/sdkjs/common/Native/native.js</file>
<file>./editors/sdkjs/common/Native/jquery_native.js</file>
<allfonts>./editors/sdkjs/common/AllFonts.js</allfonts>
<file>./editors/web-apps/vendor/xregexp/xregexp-all-min.js</file>
<sdkjs>./editors/sdkjs</sdkjs>
<dictionaries>./dictionaries</dictionaries>
<DoctSdk>
<file>./editors/sdkjs/word/sdk-all-min.js</file>
<file>./editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>./editors/sdkjs/word/sdk-all.js</file>
</DoctSdk>
<PpttSdk>
<file>./editors/sdkjs/slide/sdk-all-min.js</file>
<file>./editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>./editors/sdkjs/slide/sdk-all.js</file>
</PpttSdk>
<XlstSdk>
<file>./editors/sdkjs/cell/sdk-all-min.js</file>
<file>./editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>./editors/sdkjs/cell/sdk-all.js</file>
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
pub fn xml_escape(value: &str) -> String {
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

// Paths are passed raw and escaped here, once.
fn pdf_params_xml(input: &str, output: &str, fonts_dir: &str, allfonts: &str) -> String {
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
</TaskQueueDataConvert>"#,
        xml_escape(input),
        xml_escape(output),
        xml_escape(&fonts_dir.replace('\\', "/")),
        xml_escape(&allfonts.replace('\\', "/")),
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

    let xml = pdf_params_xml(
        input,
        output,
        &fonts_dir_for_xml.to_string_lossy(),
        &allfonts_abs.to_string_lossy(),
    );
    write_file(&params_xml, &xml)?;

    let mut cmd = std::process::Command::new(&run_x2t);
    cmd.current_dir(&run_dir)
        .arg(params_xml.to_string_lossy().as_ref());
    #[cfg(target_os = "linux")]
    cmd.env(
        "LD_LIBRARY_PATH",
        format!("{}:{}", run_dir.display(), binaries_dir.display()),
    );
    #[cfg(target_os = "macos")]
    cmd.env("DYLD_LIBRARY_PATH", &run_dir);

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

/// Windows: run x2t directly from binaries_dir (writable, all DLLs present).
/// Write DoctRenderer.config and AllFonts.js (server) in place.
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
    let app_root = binaries_dir.parent().unwrap_or(binaries_dir);
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
        &app_root.join("DoctRenderer.config"),
        DOCTRENDERER_CONFIG_CURRENT,
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

    let work_fonts = work_binaries.join("fonts");
    symlink_path(fonts_dir, &work_fonts)?;

    let fontsel_src = fontdata_dir.join("font_selection.bin");
    if is_nonempty_file(&fontsel_src) {
        copy_file(&fontsel_src, &work_binaries.join("font_selection.bin"))?;
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

    // NSIS: binaries/ holds no x2t, so the one in the install folder is used,
    // the same file the sidecar runs.
    #[test]
    fn windows_layout_finds_the_one_in_the_install_folder() {
        let root = scratch_dir("nsis");
        let binaries = root.join("binaries");
        touch(&binaries.join("kernel.dll"));
        touch(&binaries.join("x2t.dll"));
        touch(&binaries.join("x2t.pdb"));
        touch(&root.join(x2t_name()));
        touch(&root.join("euro-office-lite.exe"));

        assert_eq!(find_x2t(&binaries, Some(&root)), Ok(root.join(x2t_name())));
        let _ = std::fs::remove_dir_all(&root);
    }

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
            read(app.join("DoctRenderer.config")),
            DOCTRENDERER_CONFIG_CURRENT.as_bytes()
        );
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
        assert_eq!(link(work_binaries.join("fonts")), Some(fonts.clone()));
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
}
