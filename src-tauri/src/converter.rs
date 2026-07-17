use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

pub async fn convert_file(
    app: &AppHandle,
    input: &str,
    output: &str,
    _format_from: i32,
    format_to: i32,
    temp_dir: &str,
) -> Result<String, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let binaries_dir = resource_dir.join("binaries");

    if format_to == 513 {
        return convert_to_pdf(&binaries_dir, input, output, std::path::Path::new(temp_dir));
    }

    let params_dir = std::path::Path::new(output)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let params_path = params_dir.join("x2t_params_convert.xml");

    let xml = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<TaskQueueDataConvert xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
                      xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<m_sFileFrom>{}</m_sFileFrom>
<m_sFileTo>{}</m_sFileTo>
<m_nFormatTo>{}</m_nFormatTo>
<m_sTempDir>{}</m_sTempDir>
</TaskQueueDataConvert>"#,
        input.replace('\\', "/"),
        output.replace('\\', "/"),
        format_to,
        temp_dir.replace('\\', "/"),
    );
    std::fs::write(&params_path, &xml).map_err(|e| e.to_string())?;

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

fn log_pdf(temp_dir: &std::path::Path, msg: &str) {
    use std::io::Write;
    if let Err(e) = std::fs::create_dir_all(temp_dir) {
        eprintln!(
            "[PDF] Failed to create log directory {}: {}",
            temp_dir.display(),
            e
        );
        return;
    }
    let log_path = temp_dir.join("js-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "[PDF] {}", msg);
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

fn describe_file(path: &std::path::Path) -> String {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => format!("{} bytes", metadata.len()),
        Ok(_) => "not-a-file".to_string(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing".to_string(),
        Err(error) => format!("unreadable ({})", error),
    }
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
fn strip_extended_prefix(p: &std::path::Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    if s.starts_with("\\\\?\\") {
        std::path::PathBuf::from(&s[4..])
    } else {
        p.to_path_buf()
    }
}

fn convert_to_pdf(
    binaries_dir: &std::path::Path,
    input: &str,
    output: &str,
    temp_dir: &std::path::Path,
) -> Result<String, String> {
    let binaries_dir = &strip_extended_prefix(binaries_dir);
    log_pdf(
        temp_dir,
        &format!(
            "conversion start platform={} input={} output={} temp={} binaries={}",
            std::env::consts::OS,
            input,
            output,
            temp_dir.display(),
            binaries_dir.display()
        ),
    );
    let x2t_exe = find_x2t_exe(binaries_dir).map_err(|error| {
        log_pdf(temp_dir, &format!("x2t discovery failed: {}", error));
        error
    })?;
    let fonts_dir = binaries_dir.join("fonts");
    let fontdata_dir = temp_dir.join("fontdata");
    let editors_dir = binaries_dir
        .parent()
        .unwrap_or(binaries_dir)
        .join("editors");

    let allfonts_fontdata = fontdata_dir.join("AllFonts.js");
    let allfonts_binaries = binaries_dir.join("AllFonts.js");
    let allfonts_js = if is_nonempty_file(&allfonts_fontdata) {
        allfonts_fontdata
    } else if is_nonempty_file(&allfonts_binaries) {
        allfonts_binaries
    } else {
        let error = format!(
            "No usable AllFonts.js found at {} or {}",
            allfonts_fontdata.display(),
            allfonts_binaries.display()
        );
        log_pdf(temp_dir, &format!("setup failed: {}", error));
        return Err(error);
    };

    let runtime_strategy = if cfg!(target_os = "windows") {
        "windows-direct"
    } else if cfg!(target_os = "macos") {
        "macos-workdir"
    } else {
        "linux-workdir"
    };
    let font_selection_source = fontdata_dir.join("font_selection.bin");
    log_pdf(
        temp_dir,
        &format!(
            "setup platform={} strategy={} binaries={} allfonts_source={} allfonts_state={} font_selection_source={} font_selection_state={}",
            std::env::consts::OS,
            runtime_strategy,
            binaries_dir.display(),
            allfonts_js.display(),
            describe_file(&allfonts_js),
            font_selection_source.display(),
            describe_file(&font_selection_source)
        ),
    );

    validate_editor_resources(&editors_dir).map_err(|error| {
        log_pdf(temp_dir, &format!("setup failed: {}", error));
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
        log_pdf(
            temp_dir,
            &format!("setup failed strategy={}: {}", runtime_strategy, error),
        );
        error
    })?;
    let config_path = if cfg!(target_os = "windows") {
        binaries_dir
            .parent()
            .unwrap_or(binaries_dir)
            .join("DoctRenderer.config")
    } else {
        run_dir.join("DoctRenderer.config")
    };
    let runtime_font_selection = run_dir.join("font_selection.bin");

    log_pdf(
        temp_dir,
        &format!(
            "runtime x2t={} cwd={} allfonts={} allfonts_state={} fonts={} config={} config_state={} font_selection={} font_selection_state={}",
            run_x2t.display(),
            run_dir.display(),
            run_allfonts.display(),
            describe_file(&run_allfonts),
            run_fonts.display(),
            config_path.display(),
            describe_file(&config_path),
            runtime_font_selection.display(),
            describe_file(&runtime_font_selection)
        ),
    );

    let params_xml = std::path::PathBuf::from(output)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("x2t_params.xml");

    let fonts_dir_for_xml = std::fs::canonicalize(&run_fonts).unwrap_or_else(|_| run_fonts.clone());
    let allfonts_abs =
        std::fs::canonicalize(&run_allfonts).unwrap_or_else(|_| run_allfonts.clone());

    let xml = format!(
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
        input,
        output,
        fonts_dir_for_xml.to_string_lossy().replace('\\', "/"),
        allfonts_abs.to_string_lossy().replace('\\', "/"),
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
        log_pdf(temp_dir, &message);
        message
    })?;
    let code = result.status.code().unwrap_or(-999);
    let pdf_exists = std::path::Path::new(output).exists();
    let pdf_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    log_pdf(
        temp_dir,
        &format!(
            "x2t exit={} pdf_exists={} pdf_size={}",
            code, pdf_exists, pdf_size
        ),
    );
    if !result.status.success() && !result.stdout.is_empty() {
        log_pdf(
            temp_dir,
            &format!("x2t stdout: {}", String::from_utf8_lossy(&result.stdout)),
        );
    }
    if !result.status.success() && !result.stderr.is_empty() {
        log_pdf(
            temp_dir,
            &format!("x2t stderr: {}", String::from_utf8_lossy(&result.stderr)),
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = result.status.signal() {
            log_pdf(temp_dir, &format!("x2t killed by signal {}", sig));
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

fn find_x2t_exe(binaries_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            search_dirs.push(exe_dir.to_path_buf());
        }
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

pub fn is_x2t_binary(name: &str) -> bool {
    if cfg!(target_os = "windows") {
        (name.starts_with("x2t-") || name == "x2t.exe") && name.ends_with(".exe")
    } else {
        (name.starts_with("x2t-") && !name.contains('.')) || name == "x2t"
    }
}
