use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

pub async fn convert_file(
    app: &AppHandle,
    input: &str,
    output: &str,
    _format_from: i32,
    _format_to: i32,
) -> Result<String, String> {
    let binaries_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");

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
