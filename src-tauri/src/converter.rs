use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

pub async fn convert_file(
    app: &AppHandle,
    input: &str,
    output: &str,
    _format_from: i32,
    format_to: i32,
) -> Result<String, String> {
    let binaries_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");

    if format_to == 513 {
        return convert_to_pdf(&binaries_dir, input, output);
    }

    let result = app
        .shell()
        .sidecar("x2t")
        .map_err(|e| e.to_string())?
        .current_dir(&binaries_dir)
        .args([input, output])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!("x2t failed ({:?}): {}", result.status, stderr))
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

    let params_xml = std::path::PathBuf::from(output)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("x2t_params.xml");

    let fonts_dir_abs = std::fs::canonicalize(&fonts_dir)
        .unwrap_or_else(|_| fonts_dir.clone());
    let allfonts_abs = std::fs::canonicalize(&allfonts_js)
        .unwrap_or_else(|_| allfonts_js.clone());

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
    std::fs::write(&params_xml, &xml).map_err(|e| e.to_string())?;

    let result = std::process::Command::new(&x2t_exe)
        .current_dir(binaries_dir)
        .arg(params_xml.to_string_lossy().as_ref())
        .output()
        .map_err(|e| format!("Failed to spawn x2t: {}", e))?;

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!("x2t failed ({:?}): {}", result.status, stderr))
    }
}

fn find_x2t_exe(binaries_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    for entry in std::fs::read_dir(binaries_dir).map_err(|e| e.to_string())? {
        if let Ok(entry) = entry {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("x2t-") && name.ends_with(".exe") {
                return Ok(entry.path());
            }
        }
    }
    Err("x2t executable not found in binaries directory".to_string())
}
