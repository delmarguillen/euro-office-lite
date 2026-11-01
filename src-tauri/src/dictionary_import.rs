// "Add language…": the user picks a hunspell .aff and .dic they already have,
// and the app copies them into <user_dir>/<lang>/<lang>.aff and .dic, the
// layout list_user_dictionaries reads (dictionaries.rs).

use crate::dictionaries::{is_dictionary_asset, is_folder_name, user_dir};
use crate::save_target::write_beside_and_rename;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

// Start of every error the command returns. The bridge reads the code after
// it ("dictionary-exists: es") to pick the message it shows.
pub const IMPORT_ERROR: &str = "dictionary";

// A browser may save es.aff as es.aff.txt, so the dialog offers .txt too.
const PICK_EXTENSIONS: [&str; 3] = ["aff", "dic", "txt"];

#[derive(Debug, PartialEq)]
pub enum ImportError {
    // Not exactly one .aff and one .dic among the chosen files.
    NotAPair,
    // The two names give different languages: es.aff with fr.dic.
    NamesDiffer { aff: String, dic: String },
    // The name is not a language folder name (dictionaries::is_folder_name).
    BadName(String),
    // The file is empty, or is not a dictionary (an HTML page).
    NotADictionary(String),
    // The language is installed and replace was not asked for.
    Exists(String),
    Io(String),
}

impl ImportError {
    fn code(&self) -> &'static str {
        match self {
            ImportError::NotAPair => "not-a-pair",
            ImportError::NamesDiffer { .. } => "names-differ",
            ImportError::BadName(_) => "bad-name",
            ImportError::NotADictionary(_) => "not-a-dictionary",
            ImportError::Exists(_) => "exists",
            ImportError::Io(_) => "io",
        }
    }

    fn detail(&self) -> String {
        match self {
            ImportError::NotAPair => "choose one .aff and one .dic file".to_string(),
            ImportError::NamesDiffer { aff, dic } => format!("{aff} and {dic}"),
            ImportError::BadName(name)
            | ImportError::NotADictionary(name)
            | ImportError::Exists(name)
            | ImportError::Io(name) => name.clone(),
        }
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}: {}", IMPORT_ERROR, self.code(), self.detail())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Aff,
    Dic,
}

// "es_ES.aff" -> (es_ES, Aff); "es.aff.txt" -> (es, Aff). The extension is
// matched without case: Windows shows ES.AFF and es.aff as the same file.
fn split_name(name: &str) -> Option<(&str, Part)> {
    let name = strip_suffix_ignore_case(name, ".txt").unwrap_or(name);
    if let Some(stem) = strip_suffix_ignore_case(name, ".aff") {
        return Some((stem, Part::Aff));
    }
    if let Some(stem) = strip_suffix_ignore_case(name, ".dic") {
        return Some((stem, Part::Dic));
    }
    None
}

fn strip_suffix_ignore_case<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = name.len().checked_sub(suffix.len())?;
    let tail = name.get(cut..)?;
    if !tail.eq_ignore_ascii_case(suffix) {
        return None;
    }
    Some(&name[..cut])
}

// The language the two chosen file names give, e.g. es_ES or en-GB.
pub fn language_from_names(aff_name: &str, dic_name: &str) -> Result<String, ImportError> {
    let Some((aff_stem, Part::Aff)) = split_name(aff_name) else {
        return Err(ImportError::NotAPair);
    };
    let Some((dic_stem, Part::Dic)) = split_name(dic_name) else {
        return Err(ImportError::NotAPair);
    };
    if aff_stem != dic_stem {
        return Err(ImportError::NamesDiffer {
            aff: aff_name.to_string(),
            dic: dic_name.to_string(),
        });
    }
    if !is_folder_name(aff_stem) {
        return Err(ImportError::BadName(aff_stem.to_string()));
    }
    Ok(aff_stem.to_string())
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

// The chosen files, as (.aff, .dic). Anything but exactly one of each is
// refused, whatever order the dialog returned them in.
pub fn sort_picked(paths: &[PathBuf]) -> Result<(PathBuf, PathBuf), ImportError> {
    let [first, second] = paths else {
        return Err(ImportError::NotAPair);
    };
    let first_part = split_name(&file_name_of(first)).map(|(_, part)| part);
    match first_part {
        Some(Part::Aff) => Ok((first.clone(), second.clone())),
        Some(Part::Dic) => Ok((second.clone(), first.clone())),
        None => Err(ImportError::NotAPair),
    }
}

// Both files hold bytes and neither is an HTML page. The .aff is not required
// to carry a SET line: hunspell defaults to ISO8859-1 without one, and some
// real .aff files put it after a long comment header.
pub fn check_content(name: &str, bytes: &[u8]) -> Result<(), ImportError> {
    if !is_dictionary_asset(bytes) {
        return Err(ImportError::NotADictionary(name.to_string()));
    }
    Ok(())
}

fn check_file(path: &Path) -> Result<(), ImportError> {
    let name = file_name_of(path);
    let bytes = std::fs::read(path).map_err(|e| ImportError::Io(format!("{name}: {e}")))?;
    check_content(&name, &bytes)
}

// An installed language is a folder holding either of its two files.
fn is_installed(folder: &Path, lang: &str) -> bool {
    folder.join(format!("{lang}.aff")).exists() || folder.join(format!("{lang}.dic")).exists()
}

// Copies the pair to <dir>/<lang>/<lang>.aff and .dic. Each file is written
// beside its destination and renamed over it (save_target.rs).
pub fn install(
    dir: &Path,
    lang: &str,
    aff: &Path,
    dic: &Path,
    replace: bool,
) -> Result<(), ImportError> {
    let folder = dir.join(lang);
    if !replace && is_installed(&folder, lang) {
        return Err(ImportError::Exists(lang.to_string()));
    }
    std::fs::create_dir_all(&folder).map_err(|e| ImportError::Io(e.to_string()))?;
    write_beside_and_rename(aff, &folder.join(format!("{lang}.aff"))).map_err(ImportError::Io)?;
    write_beside_and_rename(dic, &folder.join(format!("{lang}.dic"))).map_err(ImportError::Io)?;
    Ok(())
}

// Checks the chosen pair and installs it; returns the language.
pub fn import_pair(dir: &Path, paths: &[PathBuf], replace: bool) -> Result<String, ImportError> {
    let (aff, dic) = sort_picked(paths)?;
    let lang = language_from_names(&file_name_of(&aff), &file_name_of(&dic))?;
    check_file(&aff)?;
    check_file(&dic)?;
    install(dir, &lang, &aff, &dic, replace)?;
    Ok(lang)
}

// The last pair chosen, kept in the backend so that "Replace" after an
// "already installed" answer does not open the dialog a second time. The
// webview never sees these paths.
#[derive(Default)]
pub struct PendingImport(Mutex<Vec<PathBuf>>);

async fn pick_pair(app: &AppHandle, window: &tauri::WebviewWindow) -> Result<Vec<PathBuf>, String> {
    let dialog = app
        .dialog()
        .file()
        .set_parent(window)
        .add_filter("Hunspell dictionary", &PICK_EXTENSIONS);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_files())
        .await
        .map_err(|e| e.to_string())?;
    let paths = picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|choice| choice.into_path().ok())
        .collect();
    Ok(paths)
}

// Without replace, opens the dialog. With replace, installs over the language
// the previous call reported as existing, from the files chosen then.
// Ok(None) means the user cancelled the dialog.
#[tauri::command]
pub async fn import_dictionary(
    app: AppHandle,
    window: tauri::WebviewWindow,
    pending: State<'_, PendingImport>,
    replace: bool,
) -> Result<Option<String>, String> {
    let paths = if replace {
        std::mem::take(&mut *pending.0.lock().unwrap())
    } else {
        pick_pair(&app, &window).await?
    };
    if paths.is_empty() {
        return Ok(None);
    }
    let Some(dir) = user_dir(&app) else {
        return Err(ImportError::Io("no app data folder".to_string()).to_string());
    };
    match import_pair(&dir, &paths, replace) {
        Ok(lang) => Ok(Some(lang)),
        Err(error) => {
            if matches!(error, ImportError::Exists(_)) {
                *pending.0.lock().unwrap() = paths;
            }
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn test_dir(label: &str) -> PathBuf {
        let n = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eo-dict-import-{}-{}-{}",
            label,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn lang(aff: &str, dic: &str) -> Result<String, ImportError> {
        language_from_names(aff, dic)
    }

    #[test]
    fn the_language_comes_from_the_file_names() {
        assert_eq!(lang("es.aff", "es.dic"), Ok("es".to_string()));
        assert_eq!(lang("es_ES.aff", "es_ES.dic"), Ok("es_ES".to_string()));
        assert_eq!(lang("en-GB.aff", "en-GB.dic"), Ok("en-GB".to_string()));
        assert_eq!(lang("ES.AFF", "ES.Dic"), Ok("ES".to_string()));
    }

    #[test]
    fn a_txt_added_by_a_browser_is_dropped() {
        assert_eq!(lang("es.aff.txt", "es.dic"), Ok("es".to_string()));
        assert_eq!(lang("es.aff.TXT", "es.dic.txt"), Ok("es".to_string()));
    }

    #[test]
    fn two_languages_are_refused() {
        assert_eq!(
            lang("es.aff", "fr.dic"),
            Err(ImportError::NamesDiffer {
                aff: "es.aff".to_string(),
                dic: "fr.dic".to_string()
            })
        );
    }

    #[test]
    fn a_missing_half_is_refused() {
        assert_eq!(lang("es.dic", "es.dic"), Err(ImportError::NotAPair));
        assert_eq!(lang("es.aff", "es.aff"), Err(ImportError::NotAPair));
        assert_eq!(lang("es.txt", "es.dic"), Err(ImportError::NotAPair));
        assert_eq!(lang("readme", "es.dic"), Err(ImportError::NotAPair));
    }

    #[test]
    fn a_name_that_is_not_a_folder_name_is_refused() {
        assert_eq!(
            lang(".aff", ".dic"),
            Err(ImportError::BadName(String::new()))
        );
        assert_eq!(
            lang("es ES.aff", "es ES.dic"),
            Err(ImportError::BadName("es ES".to_string()))
        );
        assert_eq!(
            lang("es.v2.aff", "es.v2.dic"),
            Err(ImportError::BadName("es.v2".to_string()))
        );
    }

    #[test]
    fn the_pair_is_sorted_whatever_the_dialog_order() {
        let aff = PathBuf::from("/d/es.aff");
        let dic = PathBuf::from("/d/es.dic");
        let expected = Ok((aff.clone(), dic.clone()));
        assert_eq!(sort_picked(&[aff.clone(), dic.clone()]), expected);
        assert_eq!(sort_picked(&[dic.clone(), aff.clone()]), expected);
        assert_eq!(sort_picked(&[aff.clone()]), Err(ImportError::NotAPair));
        assert_eq!(
            sort_picked(&[aff.clone(), dic.clone(), aff]),
            Err(ImportError::NotAPair)
        );
    }

    #[test]
    fn empty_or_html_content_is_refused() {
        assert_eq!(
            check_content("es.aff", b""),
            Err(ImportError::NotADictionary("es.aff".to_string()))
        );
        assert_eq!(
            check_content("es.dic", b"<!DOCTYPE html><html>"),
            Err(ImportError::NotADictionary("es.dic".to_string()))
        );
        assert_eq!(check_content("es.aff", b"SET UTF-8\nTRY abc"), Ok(()));
        assert_eq!(check_content("es.dic", b"2\nhola\nadios"), Ok(()));
    }

    #[test]
    fn an_import_creates_the_folder_and_the_two_names() {
        let source = test_dir("source");
        let dir = test_dir("user").join("dictionaries");
        let aff = write(&source, "es.aff.txt", b"SET UTF-8\n");
        let dic = write(&source, "es.dic", b"1\nhola\n");

        let installed = import_pair(&dir, &[dic, aff], false);

        assert_eq!(installed, Ok("es".to_string()));
        assert_eq!(
            std::fs::read(dir.join("es/es.aff")).unwrap(),
            b"SET UTF-8\n"
        );
        assert_eq!(std::fs::read(dir.join("es/es.dic")).unwrap(), b"1\nhola\n");
        let mut names: Vec<String> = std::fs::read_dir(dir.join("es"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["es.aff", "es.dic"]);
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn an_installed_language_is_kept_unless_replace_is_asked() {
        let source = test_dir("source");
        let dir = test_dir("user");
        std::fs::create_dir_all(dir.join("es")).unwrap();
        std::fs::write(dir.join("es/es.dic"), b"1\nold\n").unwrap();
        let aff = write(&source, "es.aff", b"SET UTF-8\n");
        let dic = write(&source, "es.dic", b"1\nnew\n");
        let paths = [aff, dic];

        let refused = import_pair(&dir, &paths, false);
        assert_eq!(refused, Err(ImportError::Exists("es".to_string())));
        assert_eq!(std::fs::read(dir.join("es/es.dic")).unwrap(), b"1\nold\n");
        assert!(!dir.join("es/es.aff").exists());

        assert_eq!(import_pair(&dir, &paths, true), Ok("es".to_string()));
        assert_eq!(std::fs::read(dir.join("es/es.dic")).unwrap(), b"1\nnew\n");
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_check_writes_nothing() {
        let source = test_dir("source");
        let dir = test_dir("user");
        let aff = write(&source, "es.aff", b"<html><body>404</body></html>");
        let dic = write(&source, "es.dic", b"1\nhola\n");

        let refused = import_pair(&dir, &[aff, dic], false);

        assert_eq!(
            refused,
            Err(ImportError::NotADictionary("es.aff".to_string()))
        );
        assert!(!dir.join("es").exists());
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_error_text_carries_its_code() {
        assert_eq!(
            ImportError::Exists("es".to_string()).to_string(),
            "dictionary-exists: es"
        );
        assert_eq!(
            ImportError::BadName("es v2".to_string()).to_string(),
            "dictionary-bad-name: es v2"
        );
    }
}
