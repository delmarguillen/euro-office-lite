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

fn convert_to_pdf(
    binaries_dir: &std::path::Path,
    input: &str,
    output: &str,
) -> Result<String, String> {
    let x2t_exe = find_x2t_exe(binaries_dir)?;
    let fonts_dir = binaries_dir.join("fonts");
    let allfonts_js = binaries_dir.join("AllFonts.js");

    eprintln!("[convert_to_pdf] binaries_dir={}", binaries_dir.display());
    eprintln!("[convert_to_pdf] x2t={}", x2t_exe.display());
    for name in &["DoctRenderer.config", "AllFonts.js", "xregexp-all-min.js", "sdk-word-bundle.js"] {
        let p = binaries_dir.join(name);
        eprintln!("[convert_to_pdf] {} exists={}", name, p.exists());
    }
    eprintln!("[convert_to_pdf] fonts/ exists={}", fonts_dir.exists());

    let params_xml = std::path::PathBuf::from(output)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("x2t_params.xml");

    let fonts_dir_abs = std::fs::canonicalize(&fonts_dir)
        .unwrap_or_else(|_| fonts_dir.clone());
    let allfonts_abs = std::fs::canonicalize(&allfonts_js)
        .unwrap_or_else(|_| allfonts_js.clone());
    let binaries_abs = std::fs::canonicalize(binaries_dir)
        .unwrap_or_else(|_| binaries_dir.to_path_buf());
    let doctrenderer_abs = binaries_abs.join("DoctRenderer.config");

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
<m_sDoctRendererPath>{}</m_sDoctRendererPath>
</TaskQueueDataConvert>"#,
        input,
        output,
        fonts_dir_abs.to_string_lossy().replace('\\', "/"),
        allfonts_abs.to_string_lossy().replace('\\', "/"),
        doctrenderer_abs.to_string_lossy().replace('\\', "/"),
    );
    std::fs::write(&params_xml, &xml).map_err(|e| e.to_string())?;

    let mut cmd = std::process::Command::new(&x2t_exe);
    cmd.current_dir(binaries_dir)
        .arg(params_xml.to_string_lossy().as_ref());
    #[cfg(target_os = "linux")]
    cmd.env("LD_LIBRARY_PATH", binaries_dir);
    let result = cmd.output()
        .map_err(|e| format!("Failed to spawn x2t: {}", e))?;

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let code = result.status.code().unwrap_or(-1);
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
