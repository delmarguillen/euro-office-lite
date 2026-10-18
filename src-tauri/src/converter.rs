use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

use crate::x2t_task::{find_x2t, params_path, strip_extended_prefix};

pub async fn convert_file(
    app: &AppHandle,
    input: &str,
    output: &str,
    _format_from: i32,
    format_to: i32,
    temp_dir: &str,
    // The shared font cache: the PDF export reads it, every other conversion
    // ignores it.
    fontdata_dir: &std::path::Path,
    text_options: Option<crate::text_import::TextOptions>,
) -> Result<String, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let binaries_dir = resource_dir.join("binaries");

    if format_to == 513 {
        let (temp_root, instance_id) = {
            let state = app.state::<crate::file_ops::AppState>();
            (state.temp_root.clone(), state.instance.id.clone())
        };
        let input = input.to_string();
        let output = output.to_string();
        let temp_dir = std::path::PathBuf::from(temp_dir);
        let fontdata_dir = fontdata_dir.to_path_buf();
        // Building the font cache can wait up to FONT_CACHE_WAIT and x2t takes
        // seconds: neither should hold a thread of the async runtime.
        return tauri::async_runtime::spawn_blocking(move || {
            ensure_pdf_font_cache(
                &temp_root,
                &instance_id,
                |staging: &std::path::Path| crate::generate_fonts(staging, &binaries_dir),
                crate::FONT_CACHE_WAIT,
                crate::FONT_CACHE_POLL,
            )?;
            convert_to_pdf(&binaries_dir, &input, &output, &temp_dir, &fontdata_dir)
        })
        .await
        .map_err(|e| format!("PDF conversion task failed: {}", e))?;
    }

    let params_path = params_path(std::path::Path::new(temp_dir), "convert");

    let xml = convert_params_xml(
        input,
        output,
        format_to,
        temp_dir,
        // Only set for CSV and TXT: x2t will not read them without an encoding
        // and, for CSV, a delimiter (exit 89), and writes with these too.
        &text_options
            .map(|options| options.xml_elements())
            .unwrap_or_default(),
    );
    write_file(&params_path, &xml)?;

    let mut sidecar = app
        .shell()
        .sidecar("x2t")
        .map_err(|e| e.to_string())?
        .current_dir(&binaries_dir)
        .args([params_path.to_string_lossy().as_ref()]);
    #[cfg(target_os = "linux")]
    {
        sidecar = sidecar.envs([("LD_LIBRARY_PATH", binaries_dir.to_string_lossy().as_ref())]);
    }
    let result = sidecar.output().await.map_err(|e| e.to_string())?;

    let _ = std::fs::remove_file(&params_path);

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let code = result.status.code().unwrap_or(-1);
        Err(format!(
            "x2t conversion failed (exit code {}): {}",
            code, stderr
        ))
    }
}

fn log_pdf(msg: &str) {
    crate::temp_layout::append_log(&format!("[PDF] {}", msg));
}

/// Start of the error a PDF export or print returns when there is no font
/// cache, so the frontend can say why instead of showing a generic error.
pub const FONT_CACHE_ERROR: &str = "font-cache-unavailable";

// Without the font cache x2t still exits 0 but the PDF loses its text: the font
// list bundled with the app has no font selection table, and the renderer
// needs one. So a missing cache is built here, and without it there is no PDF.
fn ensure_pdf_font_cache(
    temp_root: &std::path::Path,
    instance_id: &str,
    generate: impl FnOnce(&std::path::Path) -> bool,
    wait: std::time::Duration,
    poll: std::time::Duration,
) -> Result<(), String> {
    if crate::temp_layout::fonts_ready(temp_root) {
        return Ok(());
    }
    log_pdf("font cache missing, building it before the export");
    let outcome =
        crate::temp_layout::ensure_font_cache(temp_root, instance_id, generate, wait, poll);
    // A failed run can leave files that look complete next to an old marker,
    // so the outcome has to say the cache is good too.
    use crate::temp_layout::FontCache;
    let built = matches!(
        outcome,
        FontCache::Ready | FontCache::Generated | FontCache::GeneratedByOther
    );
    if built && crate::temp_layout::fonts_ready(temp_root) {
        log_pdf(&format!("font cache ready: {:?}", outcome));
        Ok(())
    } else {
        let error = format!(
            "{}: the font cache could not be built ({:?})",
            FONT_CACHE_ERROR, outcome
        );
        log_pdf(&error);
        Err(error)
    }
}

// The font list x2t gets for a PDF. Never the one bundled in binaries/: it has
// no font selection table, and a PDF made with it loses its text.
fn pdf_font_list(fontdata_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
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

fn write_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
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

// Paths are passed raw and escaped here, once. `extra` is already XML: the
// numeric CSV/TXT elements from TextOptions::xml_elements.
fn convert_params_xml(
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
{}</TaskQueueDataConvert>"#,
        xml_escape(&input.replace('\\', "/")),
        xml_escape(&output.replace('\\', "/")),
        format_to,
        xml_escape(&temp_dir.replace('\\', "/")),
        extra,
    )
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

fn convert_to_pdf(
    binaries_dir: &std::path::Path,
    input: &str,
    output: &str,
    temp_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
) -> Result<String, String> {
    let binaries_dir = &strip_extended_prefix(binaries_dir);
    let output_name = std::path::Path::new(output)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown.pdf");
    log_pdf(&format!(
        "start platform={} output={}",
        std::env::consts::OS,
        output_name
    ));
    let x2t_exe = find_x2t(binaries_dir, crate::app_exe_dir().as_deref()).map_err(|error| {
        log_pdf(&format!("x2t discovery failed: {}", error));
        error
    })?;
    let fonts_dir = binaries_dir.join("fonts");
    let editors_dir = binaries_dir
        .parent()
        .unwrap_or(binaries_dir)
        .join("editors");

    let allfonts_js = pdf_font_list(fontdata_dir).map_err(|error| {
        log_pdf(&format!("setup failed: {}", error));
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
        log_pdf(&format!("setup failed: {}", error));
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
        log_pdf(&format!("setup failed strategy={}: {}", runtime_strategy, error));
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
        log_pdf(&message);
        message
    })?;
    let _ = std::fs::remove_file(&params_xml);
    let code = result.status.code().unwrap_or(-999);
    let pdf_exists = std::path::Path::new(output).exists();
    let pdf_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    if result.status.success() {
        log_pdf(&format!("success pdf_size={}", pdf_size));
    } else {
        log_pdf(&format!(
            "failed exit={} pdf_exists={} pdf_size={}",
            code, pdf_exists, pdf_size
        ));
    }
    if !result.status.success() && !result.stdout.is_empty() {
        log_pdf(&format!("x2t stdout: {}", String::from_utf8_lossy(&result.stdout)));
    }
    if !result.status.success() && !result.stderr.is_empty() {
        log_pdf(&format!("x2t stderr: {}", String::from_utf8_lossy(&result.stderr)));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = result.status.signal() {
            log_pdf(&format!("x2t killed by signal {}", sig));
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
</TaskQueueDataConvert>"
        );
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

    const WAIT: std::time::Duration = std::time::Duration::from_secs(5);
    const POLL: std::time::Duration = std::time::Duration::from_millis(10);

    fn scratch_root() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "eo-converter-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // A temp root whose font cache is missing, next to an app folder whose
    // binaries/ holds the bundled list, as an installed app has.
    fn root_without_cache() -> (std::path::PathBuf, std::path::PathBuf) {
        let base = scratch_root();
        let root = base.join("temp");
        std::fs::create_dir_all(&root).unwrap();
        let binaries = base.join("binaries");
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::write(binaries.join("AllFonts.js"), b"bundled").unwrap();
        (base, root)
    }

    fn write_cache(fontdata: &std::path::Path, contents: &[u8]) {
        std::fs::create_dir_all(fontdata).unwrap();
        std::fs::write(fontdata.join("AllFonts.js"), contents).unwrap();
        std::fs::write(fontdata.join("font_selection.bin"), contents).unwrap();
    }

    // What convert_file does before x2t runs: the font list the PDF would be
    // made with, or the error that stops the export.
    fn pdf_list_for_export(
        root: &std::path::Path,
        generate: impl FnOnce(&std::path::Path) -> bool,
        wait: std::time::Duration,
    ) -> Result<std::path::PathBuf, String> {
        ensure_pdf_font_cache(root, "1-1", generate, wait, POLL)?;
        pdf_font_list(&crate::temp_layout::fontdata_dir(root))
    }

    #[test]
    fn an_existing_font_cache_is_used_without_building_it() {
        let (base, root) = root_without_cache();
        write_cache(&crate::temp_layout::fontdata_dir(&root), b"cache");
        std::fs::write(root.join(crate::temp_layout::FONTS_MARKER), b"generated").unwrap();

        let list = pdf_list_for_export(&root, |_: &std::path::Path| panic!("built again"), WAIT);

        assert_eq!(list, Ok(crate::temp_layout::fontdata_dir(&root).join("AllFonts.js")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_missing_font_cache_is_built_before_the_pdf() {
        let (base, root) = root_without_cache();

        let list = pdf_list_for_export(
            &root,
            |staging: &std::path::Path| {
                write_cache(staging, b"new");
                true
            },
            WAIT,
        );

        let expected = crate::temp_layout::fontdata_dir(&root).join("AllFonts.js");
        assert_eq!(list, Ok(expected.clone()));
        assert_eq!(std::fs::read(expected).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn no_pdf_when_the_font_cache_cannot_be_built() {
        let (base, root) = root_without_cache();

        let list = pdf_list_for_export(&root, |_: &std::path::Path| false, WAIT);

        let error = list.expect_err("a PDF without the font cache loses its text");
        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        assert!(error.contains("Failed"), "{}", error);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn no_pdf_when_another_instance_keeps_building_past_the_wait() {
        let (base, root) = root_without_cache();
        let _held = crate::temp_layout::take_lock(&root.join("fonts.lock")).unwrap();

        let list = pdf_list_for_export(
            &root,
            |_: &std::path::Path| panic!("built while another instance holds the lock"),
            std::time::Duration::from_millis(200),
        );

        let error = list.expect_err("a PDF without the font cache loses its text");
        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        assert!(error.contains("GaveUp"), "{}", error);
        let _ = std::fs::remove_dir_all(&base);
    }

    // A marker that survived next to a cache missing font_selection.bin is a
    // half-installed cache: it is rebuilt, not used.
    #[test]
    fn a_cache_without_its_selection_file_is_rebuilt() {
        let (base, root) = root_without_cache();
        let fontdata = crate::temp_layout::fontdata_dir(&root);
        std::fs::create_dir_all(&fontdata).unwrap();
        std::fs::write(fontdata.join("AllFonts.js"), b"old").unwrap();
        std::fs::write(root.join(crate::temp_layout::FONTS_MARKER), b"generated").unwrap();

        assert!(!crate::temp_layout::fonts_ready(&root));
        let list = pdf_list_for_export(
            &root,
            |staging: &std::path::Path| {
                write_cache(staging, b"new");
                true
            },
            WAIT,
        );

        assert_eq!(list, Ok(fontdata.join("AllFonts.js")));
        assert_eq!(std::fs::read(fontdata.join("AllFonts.js")).unwrap(), b"new");
        assert_eq!(std::fs::read(fontdata.join("font_selection.bin")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&base);
    }

    // The files land but the run still fails (here the new marker cannot be
    // written), next to the marker of an earlier run: the files look complete,
    // the outcome says they are not.
    #[test]
    fn no_pdf_when_the_build_fails_next_to_an_old_marker() {
        let (base, root) = root_without_cache();
        let marker = root.join(crate::temp_layout::FONTS_MARKER);
        std::fs::write(&marker, b"generated").unwrap();
        // write_replacing writes the marker through this file first.
        std::fs::create_dir_all(marker.with_extension("tmp-1-1")).unwrap();

        let list = pdf_list_for_export(
            &root,
            |staging: &std::path::Path| {
                write_cache(staging, b"new");
                true
            },
            WAIT,
        );

        assert!(crate::temp_layout::fonts_ready(&root), "the files look complete");
        let error = list.expect_err("a failed build is not a usable cache");
        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        assert!(error.contains("Failed"), "{}", error);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_bundled_font_list_is_never_the_pdf_list() {
        let (base, root) = root_without_cache();
        let fontdata = crate::temp_layout::fontdata_dir(&root);

        let error = pdf_font_list(&fontdata).expect_err("only the font cache makes a good PDF");

        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        let _ = std::fs::remove_dir_all(&base);
    }
}
