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

fn log_dir_recursive(dir: &std::path::Path, base: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap_or(&path);
            if path.is_dir() {
                log_pdf(&format!("  DIR  {}/", rel.display()));
                log_dir_recursive(&path, base);
            } else {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                log_pdf(&format!("  FILE {} ({} bytes)", rel.display(), size));
            }
        }
    }
}

fn convert_to_pdf(
    binaries_dir: &std::path::Path,
    input: &str,
    output: &str,
) -> Result<String, String> {
    log_pdf("=== convert_to_pdf START ===");
    let x2t_exe = find_x2t_exe(binaries_dir)?;
    let fonts_dir = binaries_dir.join("fonts");
    let fontdata_dir = std::env::temp_dir().join("euro-office-lite").join("fontdata");
    let allfonts_js = if fontdata_dir.join("AllFonts.js").exists() {
        fontdata_dir.join("AllFonts.js")
    } else {
        binaries_dir.join("AllFonts.js")
    };

    log_pdf(&format!("binaries_dir={}", binaries_dir.display()));
    log_pdf(&format!("x2t={}", x2t_exe.display()));
    log_pdf(&format!("input={}", input));
    log_pdf(&format!("output={}", output));

    log_pdf("=== Files in binaries/ ===");
    if let Ok(entries) = std::fs::read_dir(binaries_dir) {
        for entry in entries.flatten() {
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            log_pdf(&format!("  {} {} {}",
                if is_dir { "DIR " } else { "FILE" },
                entry.file_name().to_string_lossy(),
                if is_dir { String::new() } else { format!("({} bytes)", size) }
            ));
        }
    }

    let editors_dir = binaries_dir.parent().unwrap_or(binaries_dir).join("editors");
    log_pdf(&format!("=== editors/ dir exists: {} (at {:?}) ===", editors_dir.exists(), editors_dir));
    if editors_dir.exists() {
        log_dir_recursive(&editors_dir, &editors_dir);
    }

    let config_path = binaries_dir.join("DoctRenderer.config");
    log_pdf(&format!("=== DoctRenderer.config ({:?}) ===", config_path));
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        log_pdf(&content);
    } else {
        log_pdf("ERROR: Cannot read DoctRenderer.config");
    }

    for candidate in &[
        fontdata_dir.join("font_selection.bin"),
        binaries_dir.join("font_selection.bin"),
    ] {
        let exists = candidate.exists();
        let size = std::fs::metadata(candidate).map(|m| m.len()).unwrap_or(0);
        log_pdf(&format!("font_selection.bin at {:?}: exists={}, size={}", candidate, exists, size));
    }

    let params_xml = std::path::PathBuf::from(output)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("x2t_params.xml");

    let fonts_dir_abs = std::fs::canonicalize(&fonts_dir)
        .unwrap_or_else(|_| fonts_dir.clone());
    let allfonts_abs = std::fs::canonicalize(&allfonts_js)
        .unwrap_or_else(|_| allfonts_js.clone());

    log_pdf(&format!("=== AllFonts.js ({:?}, {} bytes) ===",
        allfonts_abs, std::fs::metadata(&allfonts_abs).map(|m| m.len()).unwrap_or(0)));
    if let Ok(content) = std::fs::read_to_string(&allfonts_abs) {
        for (i, line) in content.lines().take(5).enumerate() {
            let truncated = &line[..line.len().min(200)];
            log_pdf(&format!("  L{}: {}", i + 1, truncated));
        }
        log_pdf(&format!("  ... total {} lines", content.lines().count()));
    }

    for name in &["sdk-all-min.js", "sdk-word-bundle.js"] {
        let p = binaries_dir.join(name);
        if p.exists() {
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            log_pdf(&format!("{}: {} bytes", name, size));
        }
    }
    let sdk_in_editors = editors_dir.join("sdkjs").join("word").join("sdk-all-min.js");
    if sdk_in_editors.exists() {
        let size = std::fs::metadata(&sdk_in_editors).map(|m| m.len()).unwrap_or(0);
        log_pdf(&format!("editors/sdkjs/word/sdk-all-min.js: {} bytes", size));
    }

    let dictionaries_dir = binaries_dir.parent().unwrap_or(binaries_dir).join("dictionaries");
    log_pdf(&format!("dictionaries/ dir exists: {} (at {:?})", dictionaries_dir.exists(), dictionaries_dir));

    log_pdf(&format!("x2t executable: {:?}", x2t_exe));
    log_pdf(&format!("working dir (current_dir): {:?}", binaries_dir));

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
        fonts_dir_abs.to_string_lossy().replace('\\', "/"),
        allfonts_abs.to_string_lossy().replace('\\', "/"),
    );

    log_pdf(&format!("=== XML params ===\n{}", xml));

    std::fs::write(&params_xml, &xml).map_err(|e| e.to_string())?;

    let mut cmd = std::process::Command::new(&x2t_exe);
    cmd.current_dir(binaries_dir)
        .arg(params_xml.to_string_lossy().as_ref());
    #[cfg(target_os = "linux")]
    cmd.env("LD_LIBRARY_PATH", binaries_dir);

    log_pdf("Spawning x2t...");
    let result = cmd.output()
        .map_err(|e| format!("Failed to spawn x2t: {}", e))?;

    let code = result.status.code().unwrap_or(-999);
    log_pdf(&format!("x2t exit code: {}", code));
    if !result.stdout.is_empty() {
        log_pdf(&format!("x2t stdout ({} bytes): {}", result.stdout.len(),
            String::from_utf8_lossy(&result.stdout)));
    }
    if !result.stderr.is_empty() {
        log_pdf(&format!("x2t stderr ({} bytes): {}", result.stderr.len(),
            String::from_utf8_lossy(&result.stderr)));
    }

    let pdf_exists = std::path::Path::new(output).exists();
    let pdf_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    log_pdf(&format!("Output PDF: exists={}, size={} bytes", pdf_exists, pdf_size));
    log_pdf("=== convert_to_pdf END ===");

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!("x2t conversion failed (exit code {}): {}", code, stderr))
    }
}

fn find_x2t_exe(binaries_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let mut search_dirs = vec![
        binaries_dir.to_path_buf(),
        binaries_dir.parent().unwrap_or(binaries_dir).to_path_buf(),
    ];
    // Tauri places sidecars in Contents/MacOS/ on macOS
    if let Some(resources) = binaries_dir.parent() {
        if let Some(contents) = resources.parent() {
            let macos_dir = contents.join("MacOS");
            if macos_dir.exists() {
                search_dirs.push(macos_dir);
            }
        }
    }
    // Also check next to the current executable
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
