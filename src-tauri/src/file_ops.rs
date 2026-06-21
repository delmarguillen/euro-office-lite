use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};

pub struct AppState {
    pub current_file: Mutex<Option<PathBuf>>,
    pub temp_dir: PathBuf,
    pub modified: Mutex<bool>,
}

fn log_print(state: &AppState, msg: &str) {
    println!("[PRINT] {}", msg);
    use std::io::Write;
    let log_path = state.temp_dir.join("js-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "[PRINT] {}", msg);
    }
}

fn clear_changes(temp_dir: &std::path::Path) {
    let changes_dir = temp_dir.join("changes");
    if changes_dir.exists() {
        let _ = std::fs::remove_dir_all(&changes_dir);
    }
}

#[tauri::command]
pub async fn open_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let input = PathBuf::from(&path);
    let output = state.temp_dir.join("Editor.bin");

    let format_from = detect_format(&input);
    let format_to = 8192;

    super::converter::convert_file(&app, &path, &output.to_string_lossy(), format_from, format_to)
        .await?;

    let bin_data = std::fs::read(&output).map_err(|e| e.to_string())?;
    let b64 = STANDARD.encode(&bin_data);

    *state.current_file.lock().unwrap() = Some(input);
    *state.modified.lock().unwrap() = false;

    Ok(b64)
}

#[tauri::command]
pub async fn save_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    _data: String,
) -> Result<String, String> {
    let current = state.current_file.lock().unwrap().clone();
    let dest = current.ok_or("No file is currently open")?;

    let input = state.temp_dir.join("Editor.bin");
    let format_from = 8192;
    let format_to = detect_format(&dest);

    super::converter::convert_file(
        &app,
        &input.to_string_lossy(),
        &dest.to_string_lossy(),
        format_from,
        format_to,
    )
    .await?;

    *state.modified.lock().unwrap() = false;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn save_file_as(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let dest = PathBuf::from(&path);
    let input = state.temp_dir.join("Editor.bin");
    let format_from = 8192;
    let format_to = detect_format(&dest);

    if format_to == 513 {
        clear_changes(&state.temp_dir);
    }

    super::converter::convert_file(
        &app,
        &input.to_string_lossy(),
        &dest.to_string_lossy(),
        format_from,
        format_to,
    )
    .await?;

    if format_to != 513 {
        *state.current_file.lock().unwrap() = Some(dest);
    }
    *state.modified.lock().unwrap() = false;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn save_changes(
    state: State<'_, AppState>,
    changes: String,
    _delete_index: Option<i32>,
    count: i32,
) -> Result<String, String> {
    let changes_dir = state.temp_dir.join("changes");
    std::fs::create_dir_all(&changes_dir).map_err(|e| e.to_string())?;

    let filename = format!("change_{}.json", count);
    std::fs::write(changes_dir.join(&filename), &changes).map_err(|e| e.to_string())?;

    Ok("ok".to_string())
}

#[tauri::command]
pub async fn write_editor_bin(
    state: State<'_, AppState>,
    data: String,
) -> Result<String, String> {
    let bin_data = STANDARD.decode(&data).map_err(|e| e.to_string())?;
    let output = state.temp_dir.join("Editor.bin");
    std::fs::write(&output, &bin_data).map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn print_document(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let editor_bin = state.temp_dir.join("Editor.bin");
    log_print(&state, &format!("Editor.bin path: {:?}, exists: {}", editor_bin, editor_bin.exists()));

    if !editor_bin.exists() {
        return Err("Editor.bin not found".to_string());
    }

    let bin_size = std::fs::metadata(&editor_bin).map(|m| m.len()).unwrap_or(0);
    log_print(&state, &format!("Editor.bin size: {} bytes", bin_size));

    let pdf_path = state.temp_dir.join("print_output.pdf");
    if pdf_path.exists() {
        log_print(&state, "Removing previous print_output.pdf");
        let _ = std::fs::remove_file(&pdf_path);
    }

    clear_changes(&state.temp_dir);
    log_print(&state, "Cleared changes directory");

    log_print(&state, "Running x2t conversion Editor.bin -> PDF...");
    super::converter::convert_file(
        &app,
        &editor_bin.to_string_lossy(),
        &pdf_path.to_string_lossy(),
        8192,
        513,
    )
    .await
    .map_err(|e| {
        log_print(&state, &format!("x2t conversion failed: {}", e));
        e
    })?;

    let pdf_size = std::fs::metadata(&pdf_path).map(|m| m.len()).unwrap_or(0);
    log_print(&state, &format!("PDF created: {:?}, size: {} bytes", pdf_path, pdf_size));

    if pdf_size == 0 {
        return Err("PDF file is empty".to_string());
    }

    log_print(&state, "Opening PDF with system viewer...");
    let pdf_str = pdf_path.to_string_lossy().to_string();
    std::process::Command::new("cmd")
        .args(["/c", "start", "", &pdf_str])
        .spawn()
        .map_err(|e| {
            log_print(&state, &format!("Failed to open PDF: {}", e));
            e.to_string()
        })?;

    log_print(&state, "PDF opened successfully");
    Ok("ok".to_string())
}

#[tauri::command]
pub fn get_current_path(state: State<'_, AppState>) -> Option<String> {
    state
        .current_file
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn create_new(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    doc_type: String,
) -> Result<String, String> {
    let template = match doc_type.as_str() {
        "word" => "templates/blank.docx",
        "cell" => "templates/blank.xlsx",
        "slide" => "templates/blank.pptx",
        _ => return Err(format!("Unknown type: {}", doc_type)),
    };

    let template_path = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join(template);

    *state.current_file.lock().unwrap() = None;
    *state.modified.lock().unwrap() = false;

    open_file(app, state, template_path.to_string_lossy().to_string()).await
}

fn detect_format(path: &PathBuf) -> i32 {
    match path.extension().and_then(|e| e.to_str()) {
        Some("docx") => 65,
        Some("doc") => 66,
        Some("odt") => 67,
        Some("rtf") => 68,
        Some("txt") => 69,
        Some("xlsx") => 257,
        Some("xls") => 258,
        Some("ods") => 259,
        Some("csv") => 260,
        Some("pptx") => 129,
        Some("ppt") => 130,
        Some("odp") => 131,
        Some("pdf") => 513,
        _ => 0,
    }
}
