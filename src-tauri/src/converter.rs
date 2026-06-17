use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;

const X2T_PARAMS_TEMPLATE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<TaskQueueDataConvert>
  <m_sFileFrom>{input}</m_sFileFrom>
  <m_sFileTo>{output}</m_sFileTo>
  <m_sFontDir>{fonts_dir}</m_sFontDir>
  <m_sThemeDir></m_sThemeDir>
  <m_nFormatFrom>{format_from}</m_nFormatFrom>
  <m_nFormatTo>{format_to}</m_nFormatTo>
</TaskQueueDataConvert>"#;

#[tauri::command]
pub async fn convert_file(
    app: AppHandle,
    input: String,
    output: String,
    format_from: i32,
    format_to: i32,
) -> Result<String, String> {
    let temp_dir = std::env::temp_dir().join("euro-office");
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let params_path = temp_dir.join("convert_params.xml");
    let fonts_dir = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("fonts");

    let params = X2T_PARAMS_TEMPLATE
        .replace("{input}", &input)
        .replace("{output}", &output)
        .replace("{fonts_dir}", &fonts_dir.to_string_lossy())
        .replace("{format_from}", &format_from.to_string())
        .replace("{format_to}", &format_to.to_string());

    std::fs::write(&params_path, &params).map_err(|e| e.to_string())?;

    let output = app
        .shell()
        .sidecar("x2t")
        .map_err(|e| e.to_string())?
        .args([params_path.to_string_lossy().as_ref()])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok("ok".to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}
