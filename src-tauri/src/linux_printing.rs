//! Linux printer discovery and PDF submission through CUPS.
mod cups;

pub use cups::print_pdf;

#[tauri::command]
pub async fn get_printers() -> Result<String, String> {
    let printers = tauri::async_runtime::spawn_blocking(cups::get_printers)
        .await
        .map_err(|e| e.to_string())??;
    let printers: Vec<_> = printers
        .into_iter()
        .map(|printer| {
            serde_json::json!({
                "name": printer.name,
                "is_default": printer.is_default,
                "color_supported": printer.color_supported,
                "duplex_supported": printer.duplex_supported,
            })
        })
        .collect();
    serde_json::to_string(&printers).map_err(|e| e.to_string())
}
