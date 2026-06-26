use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

pub async fn convert_file(
    app: &AppHandle,
    input: &str,
    output: &str,
    _format_from: i32,
    format_to: i32,
) -> Result<String, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let binaries_dir = resource_dir.join("binaries");

    if format_to == 513 {
        return convert_to_pdf(&binaries_dir, input, output);
    }

    let mut sidecar = app
        .shell()
        .sidecar("x2t")
        .map_err(|e| e.to_string())?
        .current_dir(&binaries_dir)
        .args([input, output]);
    #[cfg(target_os = "linux")]
    {
        sidecar = sidecar.envs([("LD_LIBRARY_PATH", binaries_dir.to_string_lossy().as_ref())]);
    }
    let result = sidecar
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let code = result.status.code().unwrap_or(-1);
        Err(format!("x2t conversion failed (exit code {}): {}", code, stderr))
    }
}

fn log_pdf(msg: &str) {
    use std::io::Write;
    let log_path = std::env::temp_dir().join("euro-office-lite").join("js-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "[PDF] {}", msg);
    }
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
) -> Result<String, String> {
    let binaries_dir = &strip_extended_prefix(binaries_dir);
    let x2t_exe = find_x2t_exe(binaries_dir)?;
    let fonts_dir = binaries_dir.join("fonts");
    let fontdata_dir = std::env::temp_dir().join("euro-office-lite").join("fontdata");
    let editors_dir = binaries_dir.parent().unwrap_or(binaries_dir).join("editors");

    let allfonts_fontdata = fontdata_dir.join("AllFonts.js");
    let allfonts_binaries = binaries_dir.join("AllFonts.js");
    let allfonts_js = if allfonts_fontdata.exists() {
        allfonts_fontdata.clone()
    } else {
        allfonts_binaries.clone()
    };

    #[cfg(target_os = "linux")]
    let (run_dir, run_x2t, run_allfonts) = setup_linux_workdir(
        binaries_dir, &x2t_exe, &fonts_dir, &fontdata_dir, &editors_dir, &allfonts_js,
    )?;

    #[cfg(not(target_os = "linux"))]
    let (run_dir, run_x2t, run_allfonts) = setup_windows_direct(
        binaries_dir, &x2t_exe, &fonts_dir, &fontdata_dir, &editors_dir, &allfonts_js,
    )?;

    // Build XML params for x2t
    let params_xml = std::path::PathBuf::from(output)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("x2t_params.xml");

    let fonts_dir_for_xml = std::fs::canonicalize(&fonts_dir)
        .unwrap_or_else(|_| fonts_dir.clone());
    let allfonts_abs = std::fs::canonicalize(&run_allfonts)
        .unwrap_or_else(|_| run_allfonts.clone());

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
    std::fs::write(&params_xml, &xml).map_err(|e| e.to_string())?;

    let mut cmd = std::process::Command::new(&run_x2t);
    cmd.current_dir(&run_dir)
        .arg(params_xml.to_string_lossy().as_ref());
    #[cfg(target_os = "linux")]
    cmd.env("LD_LIBRARY_PATH", format!("{}:{}", run_dir.display(), binaries_dir.display()));

    let result = cmd.output()
        .map_err(|e| format!("Failed to spawn x2t: {}", e))?;

    let code = result.status.code().unwrap_or(-999);
    let pdf_exists = std::path::Path::new(output).exists();
    let pdf_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    if !result.status.success() {
        log_pdf(&format!("x2t exit={} pdf_exists={} pdf_size={}", code, pdf_exists, pdf_size));
        if !result.stderr.is_empty() {
            log_pdf(&format!("x2t stderr: {}", String::from_utf8_lossy(&result.stderr)));
        }
    }

    #[cfg(target_os = "linux")]
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
        Err(format!("x2t conversion failed (exit code {}): {}", code, stderr))
    }
}

/// Windows: run x2t directly from binaries_dir (writable, all DLLs present).
/// Write DoctRenderer.config and AllFonts.js (server) in place.
#[cfg(not(target_os = "linux"))]
fn setup_windows_direct(
    binaries_dir: &std::path::Path,
    x2t_exe: &std::path::Path,
    fonts_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
    editors_dir: &std::path::Path,
    allfonts_js: &std::path::Path,
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    // x2t.exe lives in the app root (not binaries/), so DoctRenderer resolves
    // its config relative to the exe location = app root.
    let app_root = binaries_dir.parent().unwrap_or(binaries_dir);
    // Copy server AllFonts.js over the web version in binaries/ and editors/
    let bin_allfonts = binaries_dir.join("AllFonts.js");
    let _ = std::fs::copy(allfonts_js, &bin_allfonts);
    let editors_allfonts = editors_dir.join("sdkjs/common/AllFonts.js");
    if editors_allfonts.exists() {
        let _ = std::fs::copy(allfonts_js, &editors_allfonts);
    }

    // Copy font_selection.bin next to AllFonts.js
    let fontsel_src = fontdata_dir.join("font_selection.bin");
    if fontsel_src.exists() {
        let _ = std::fs::copy(&fontsel_src, binaries_dir.join("font_selection.bin"));
        let _ = std::fs::copy(&fontsel_src, fonts_dir.join("font_selection.bin"));
    }

    // Write DoctRenderer.config next to x2t.exe (app root).
    // Paths are relative to x2t.exe location, so ./editors/ not ../editors/.
    let config_path = app_root.join("DoctRenderer.config");
    let config_content = r#"<Settings>
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
</Settings>"#;
    let _ = std::fs::write(&config_path, config_content);

    // current_dir must be binaries/ so Windows finds DLLs there.
    // DoctRenderer.config is next to x2t.exe (app root) — resolved by exe location.
    Ok((binaries_dir.to_path_buf(), x2t_exe.to_path_buf(), bin_allfonts))
}

/// Linux: build writable work directory (/usr/lib/ is read-only at runtime).
/// Symlink binaries, copy writable files, mirror editors/ structure.
#[cfg(target_os = "linux")]
fn setup_linux_workdir(
    binaries_dir: &std::path::Path,
    x2t_exe: &std::path::Path,
    fonts_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
    editors_dir: &std::path::Path,
    allfonts_js: &std::path::Path,
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    let work_dir = std::env::temp_dir().join("euro-office-lite").join("x2t-workdir");
    let work_binaries = work_dir.join("binaries");
    let work_editors = work_dir.join("editors");
    let work_dictionaries = work_dir.join("dictionaries");

    let _ = std::fs::create_dir_all(&work_binaries);
    let _ = std::fs::create_dir_all(&work_editors.join("sdkjs/common/Native"));
    let _ = std::fs::create_dir_all(&work_editors.join("sdkjs/common/libfont/engine"));
    let _ = std::fs::create_dir_all(&work_editors.join("sdkjs/word"));
    let _ = std::fs::create_dir_all(&work_editors.join("web-apps/vendor/xregexp"));
    let _ = std::fs::create_dir_all(&work_dictionaries);

    // Symlink x2t binary
    let work_x2t = work_binaries.join("x2t");
    if !work_x2t.exists() {
        let _ = std::os::unix::fs::symlink(x2t_exe, &work_x2t);
    }

    // Symlink shared libraries and data files
    if let Ok(entries) = std::fs::read_dir(binaries_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".so") || name.contains(".so.") || name.ends_with(".dat") || name == "package.config" {
                let dst = work_binaries.join(&name);
                if !dst.exists() {
                    let _ = std::os::unix::fs::symlink(entry.path(), &dst);
                }
            }
        }
    }

    // Symlink fonts directory
    let work_fonts = work_binaries.join("fonts");
    if !work_fonts.exists() {
        let _ = std::os::unix::fs::symlink(fonts_dir, &work_fonts);
    }

    // Copy font_selection.bin
    let fontsel_src = fontdata_dir.join("font_selection.bin");
    if fontsel_src.exists() {
        let _ = std::fs::copy(&fontsel_src, work_binaries.join("font_selection.bin"));
        if work_fonts.exists() {
            let _ = std::fs::copy(&fontsel_src, work_fonts.join("font_selection.bin"));
        }
    }

    // Copy server AllFonts.js
    let work_allfonts = work_binaries.join("AllFonts.js");
    let _ = std::fs::copy(allfonts_js, &work_allfonts);
    let _ = std::fs::copy(allfonts_js, work_editors.join("sdkjs/common/AllFonts.js"));

    // Write DoctRenderer.config
    let work_config = work_binaries.join("DoctRenderer.config");
    let config_content = r#"<Settings>
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
</Settings>"#;
    let _ = std::fs::write(&work_config, config_content);

    // Symlink JS files from installed editors/
    let editor_file_mappings = [
        "sdkjs/common/Native/native.js",
        "sdkjs/common/Native/jquery_native.js",
        "sdkjs/common/libfont/engine/fonts_native.js",
        "sdkjs/word/sdk-all-min.js",
        "sdkjs/word/sdk-all.js",
        "web-apps/vendor/xregexp/xregexp-all-min.js",
    ];
    for rel in &editor_file_mappings {
        let src = editors_dir.join(rel);
        let dst = work_editors.join(rel);
        if src.exists() && !dst.exists() {
            let _ = std::os::unix::fs::symlink(&src, &dst);
        }
    }

    Ok((work_binaries.clone(), work_x2t, work_allfonts))
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
