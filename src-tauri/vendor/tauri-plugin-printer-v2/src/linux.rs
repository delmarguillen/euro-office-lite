//! Linux subset used by Euro-Office Lite: discovery and backend-only PDF submission.
mod cups;

pub fn get_printers() -> Result<String, String> {
    let printers: Vec<_> = cups::get_printers()?
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

/// Returns a CUPS job ID after the scheduler has spooled the PDF. Does not
/// delete the source file; the application's validated print flow owns it.
pub fn print_pdf(printer: &str, pdf: &std::path::Path) -> Result<i32, String> {
    cups::print_pdf(printer, pdf)
}
