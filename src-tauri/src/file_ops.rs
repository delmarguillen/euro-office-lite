use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};

pub struct AppState {
    pub current_file: Mutex<Option<PathBuf>>,
    pub temp_dir: PathBuf,
    pub modified: Mutex<bool>,
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

    super::converter::convert_file(
        &app,
        &input.to_string_lossy(),
        &dest.to_string_lossy(),
        format_from,
        format_to,
    )
    .await?;

    *state.current_file.lock().unwrap() = Some(dest);
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
