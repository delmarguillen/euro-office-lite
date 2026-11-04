// "Add language…": the user picks a hunspell .aff and .dic they already have,
// and the app copies them into <user_dir>/<lang>/<lang>.aff and .dic, the
// layout list_user_dictionaries reads (dictionaries.rs).

use crate::dictionaries::{is_dictionary_asset, is_folder_name, user_dir};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
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
    // The spell checker has no folder for this language, or several and none
    // is <lang>_<LANG>. Holds the language and the folders it could take.
    UnknownLanguage { lang: String, options: Vec<String> },
    // The language is installed and replace was not asked for.
    Exists(String),
    // Reading or writing failed. Holds the language or the chosen file name,
    // never a path: the error text goes to the webview.
    Io(String),
}

impl ImportError {
    fn code(&self) -> &'static str {
        match self {
            ImportError::NotAPair => "not-a-pair",
            ImportError::NamesDiffer { .. } => "names-differ",
            ImportError::BadName(_) => "bad-name",
            ImportError::NotADictionary(_) => "not-a-dictionary",
            ImportError::UnknownLanguage { .. } => "unknown-language",
            ImportError::Exists(_) => "exists",
            ImportError::Io(_) => "io",
        }
    }

    fn detail(&self) -> String {
        match self {
            ImportError::NotAPair => "choose one .aff and one .dic file".to_string(),
            ImportError::NamesDiffer { aff, dic } => format!("{aff} and {dic}"),
            ImportError::UnknownLanguage { lang, options } if options.is_empty() => lang.clone(),
            ImportError::UnknownLanguage { lang, options } => {
                format!("{lang} ({})", options.join(", "))
            }
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

// The folder the spell checker reads this language from. `known` is its own
// list of folder names, sent by the bridge. LibreOffice names its French
// files fr.aff, but the checker only knows fr_FR, so a short code takes the
// one known folder that starts with it, or <lang>_<LANG> among several.
// Entries of `known` that are not language folder names are ignored: the
// webview sends the list, so it must not decide a path.
pub fn folder_for(lang: &str, known: &[String]) -> Result<String, ImportError> {
    let known: Vec<String> = known
        .iter()
        .filter(|code| is_language_folder(code))
        .cloned()
        .collect();
    if known.iter().any(|code| code == lang) {
        return Ok(lang.to_string());
    }
    let prefix = format!("{lang}_");
    let options: Vec<String> = known
        .iter()
        .filter(|code| code.starts_with(&prefix))
        .cloned()
        .collect();
    if let [only] = options.as_slice() {
        return Ok(only.clone());
    }
    let main_region = format!("{lang}_{}", lang.to_ascii_uppercase());
    if options.contains(&main_region) {
        return Ok(main_region);
    }
    Err(ImportError::UnknownLanguage {
        lang: lang.to_string(),
        options,
    })
}

// The names sdkjs gives its spell checker folders: a lower-case language,
// then region, script or variant parts (fr_FR, sr_Latn_RS, ca_ES_valencia).
// Letters and underscores only, so never a separator, a dot or a drive.
pub fn is_language_folder(name: &str) -> bool {
    let mut parts = name.split('_');
    let Some(language) = parts.next() else {
        return false;
    };
    let language_ok =
        (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_lowercase());
    if !language_ok {
        return false;
    }
    parts.all(|part| (2..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphabetic()))
}

// <dir>/<folder>, only when `folder` is one plain path component.
fn folder_path(dir: &Path, folder: &str) -> Result<PathBuf, ImportError> {
    if !is_language_folder(folder) {
        return Err(ImportError::BadName(folder.to_string()));
    }
    let mut components = Path::new(folder).components();
    let single = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if !single {
        return Err(ImportError::BadName(folder.to_string()));
    }
    Ok(dir.join(folder))
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

// Hunspell .aff and .dic files are text; a NUL byte means an image or an
// archive renamed. Only the start is read: that is where a binary shows it.
const BINARY_SNIFF_BYTES: usize = 4096;

fn looks_binary(bytes: &[u8]) -> bool {
    let start = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    start.contains(&0)
}

// Both files hold bytes, neither is an HTML page nor binary. The .aff is not required
// to carry a SET line: hunspell defaults to ISO8859-1 without one, and some
// real .aff files put it after a long comment header.
pub fn check_content(name: &str, bytes: &[u8]) -> Result<(), ImportError> {
    if !is_dictionary_asset(bytes) {
        return Err(ImportError::NotADictionary(name.to_string()));
    }
    if looks_binary(bytes) {
        return Err(ImportError::NotADictionary(name.to_string()));
    }
    Ok(())
}

fn check_file(path: &Path) -> Result<(), ImportError> {
    let name = file_name_of(path);
    let bytes = std::fs::read(path).map_err(|e| io_error(&name, "read", path, e))?;
    check_content(&name, &bytes)
}

// An installed language is a folder holding either of its two files.
fn is_installed(folder: &Path, lang: &str) -> bool {
    folder.join(format!("{lang}.aff")).exists() || folder.join(format!("{lang}.dic")).exists()
}

// The app log (js-debug-<instance>.log), local to this machine, so it may
// hold paths. stderr goes nowhere in the Windows GUI build.
fn log(line: &str) {
    crate::temp_layout::append_log(&format!("[dictionary] {line}"));
}

// The webview gets the language only; the path and the OS error go to the
// app log.
fn io_error(lang: &str, what: &str, path: &Path, error: std::io::Error) -> ImportError {
    log(&format!("{what} {}: {error}", path.display()));
    ImportError::Io(lang.to_string())
}

// A symlink, or on Windows any reparse point (a junction too). Checked with
// symlink_metadata, so the link itself is looked at, not what it points to.
fn is_link(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() {
        return true;
    }
    is_reparse_point(&metadata)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn refuse_link(lang: &str, path: &Path) -> Result<(), ImportError> {
    if !is_link(path) {
        return Ok(());
    }
    log(&format!("refused, a link: {}", path.display()));
    Err(ImportError::Io(lang.to_string()))
}

// dictionaries/ itself, checked before anything is created or written in
// it. Checked again after create_dir_all, which follows a link at that name.
fn open_dir(lang: &str, dir: &Path) -> Result<(), ImportError> {
    refuse_link(lang, dir)?;
    std::fs::create_dir_all(dir).map_err(|e| io_error(lang, "create", dir, e))?;
    refuse_link(lang, dir)
}

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

// Hidden and unique to this process and call, in the destination folder so
// the rename never crosses file systems.
fn temp_path(folder: &Path, file_name: &str) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    folder.join(format!(".{file_name}.{}-{n}.tmp", std::process::id()))
}

// Copies `from` to a new temp file in `folder` and syncs it to disk.
// create_new never opens an existing file or follows a link at that name.
fn write_temp(
    lang: &str,
    from: &Path,
    folder: &Path,
    file_name: &str,
) -> Result<PathBuf, ImportError> {
    let tmp = temp_path(folder, file_name);
    let result = copy_and_sync(from, &tmp);
    let Err(error) = result else {
        return Ok(tmp);
    };
    let _ = std::fs::remove_file(&tmp);
    Err(io_error(lang, "write", &tmp, error))
}

fn copy_and_sync(from: &Path, tmp: &Path) -> std::io::Result<()> {
    let mut source = File::open(from)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(tmp)?;
    std::io::copy(&mut source, &mut file)?;
    file.sync_all()
}

// Copies the pair to <dir>/<lang>/<lang>.aff and .dic. Both files are first
// written as temp files beside the destinations; only then are they renamed
// over them, .dic first. A failure before the renames leaves the installed
// pair as it was. If the .aff rename fails after the .dic one, a first
// install removes the new .dic, so no lone half stays; a replace keeps the
// new .dic with the old .aff, which hunspell still loads.
pub fn install(
    dir: &Path,
    lang: &str,
    aff: &Path,
    dic: &Path,
    replace: bool,
) -> Result<(), ImportError> {
    let folder = folder_path(dir, lang)?;
    if !replace && is_installed(&folder, lang) {
        return Err(ImportError::Exists(lang.to_string()));
    }
    open_dir(lang, dir)?;
    refuse_link(lang, &folder)?;
    if !folder.exists() {
        std::fs::create_dir(&folder).map_err(|e| io_error(lang, "create", &folder, e))?;
    }
    let aff_name = format!("{lang}.aff");
    let dic_name = format!("{lang}.dic");
    let aff_dest = folder.join(&aff_name);
    let dic_dest = folder.join(&dic_name);
    refuse_link(lang, &aff_dest)?;
    refuse_link(lang, &dic_dest)?;
    let first_install = !is_installed(&folder, lang);

    let aff_tmp = write_temp(lang, aff, &folder, &aff_name)?;
    let dic_tmp = match write_temp(lang, dic, &folder, &dic_name) {
        Ok(tmp) => tmp,
        Err(error) => {
            let _ = std::fs::remove_file(&aff_tmp);
            return Err(error);
        }
    };
    if let Err(error) = std::fs::rename(&dic_tmp, &dic_dest) {
        let _ = std::fs::remove_file(&aff_tmp);
        let _ = std::fs::remove_file(&dic_tmp);
        return Err(io_error(lang, "rename", &dic_dest, error));
    }
    if let Err(error) = std::fs::rename(&aff_tmp, &aff_dest) {
        let _ = std::fs::remove_file(&aff_tmp);
        if first_install {
            let _ = std::fs::remove_file(&dic_dest);
        }
        return Err(io_error(lang, "rename", &aff_dest, error));
    }
    Ok(())
}

// A checked pair and the folder it goes to.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub aff: PathBuf,
    pub dic: PathBuf,
    pub folder: String,
}

// Checks the chosen pair and picks its folder; writes nothing.
pub fn resolve(paths: &[PathBuf], known: &[String]) -> Result<Resolved, ImportError> {
    let (aff, dic) = sort_picked(paths)?;
    let lang = language_from_names(&file_name_of(&aff), &file_name_of(&dic))?;
    let folder = folder_for(&lang, known)?;
    check_file(&aff)?;
    check_file(&dic)?;
    Ok(Resolved { aff, dic, folder })
}

// Checks the chosen pair and installs it; returns the folder it went to.
pub fn import_pair(
    dir: &Path,
    paths: &[PathBuf],
    known: &[String],
    replace: bool,
) -> Result<String, ImportError> {
    let resolved = resolve(paths, known)?;
    install(dir, &resolved.folder, &resolved.aff, &resolved.dic, replace)?;
    Ok(resolved.folder)
}

// A first import. On Exists, also returns what the user is asked to replace.
pub fn start_import(
    dir: &Path,
    paths: &[PathBuf],
    known: &[String],
) -> (Result<String, ImportError>, Option<Resolved>) {
    let resolved = match resolve(paths, known) {
        Ok(resolved) => resolved,
        Err(error) => return (Err(error), None),
    };
    match install(dir, &resolved.folder, &resolved.aff, &resolved.dic, false) {
        Ok(()) => (Ok(resolved.folder), None),
        Err(error @ ImportError::Exists(_)) => (Err(error), Some(resolved)),
        Err(error) => (Err(error), None),
    }
}

// Replaces exactly the folder the user confirmed, with the files chosen then.
// It takes no language list, so a second call cannot point elsewhere. The
// files are checked again: they may have changed while the question was up.
pub fn finish_replace(dir: &Path, confirmed: &Resolved) -> Result<String, ImportError> {
    check_file(&confirmed.aff)?;
    check_file(&confirmed.dic)?;
    install(dir, &confirmed.folder, &confirmed.aff, &confirmed.dic, true)?;
    Ok(confirmed.folder.clone())
}

// The pair waiting for a "Replace?" answer, kept in the backend so the dialog
// does not open a second time. It holds the folder resolved then. It is
// dropped when used, when cancelled and when another import starts. The
// webview never sees these paths.
#[derive(Default)]
pub struct PendingImport(Mutex<Option<Resolved>>);

impl PendingImport {
    fn take(&self) -> Option<Resolved> {
        self.0.lock().unwrap().take()
    }

    fn keep(&self, resolved: Option<Resolved>) {
        *self.0.lock().unwrap() = resolved;
    }
}

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

// Without replace, opens the dialog. With replace, installs over the folder
// the previous call reported as existing, from the files chosen then; `known`
// is ignored. Ok(None) means the user cancelled the dialog, or there was
// nothing waiting to replace. `known` is the spell checker's list of
// language folder names, never a path.
#[tauri::command]
pub async fn import_dictionary(
    app: AppHandle,
    window: tauri::WebviewWindow,
    pending: State<'_, PendingImport>,
    known: Vec<String>,
    replace: bool,
) -> Result<Option<String>, String> {
    let waiting = pending.take();
    let Some(dir) = user_dir(&app) else {
        return Err(ImportError::Io("no app data folder".to_string()).to_string());
    };
    if replace {
        let Some(confirmed) = waiting else {
            return Ok(None);
        };
        return finish_replace(&dir, &confirmed)
            .map(Some)
            .map_err(|error| error.to_string());
    }
    let paths = pick_pair(&app, &window).await?;
    if paths.is_empty() {
        return Ok(None);
    }
    let (result, to_confirm) = start_import(&dir, &paths, &known);
    pending.keep(to_confirm);
    result.map(Some).map_err(|error| error.to_string())
}

// The user answered "No" to "Replace?".
#[tauri::command]
pub fn cancel_dictionary_import(pending: State<'_, PendingImport>) {
    pending.take();
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // A slice of the sdkjs table: several regions for en and pt, one for fr.
    fn known() -> Vec<String> {
        [
            "ar", "en_GB", "en_US", "es_ES", "fr_FR", "pt_BR", "pt_PT", "de_AT", "de_CH",
        ]
        .iter()
        .map(|code| code.to_string())
        .collect()
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
    fn binary_content_is_refused() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(
            check_content("it_IT.aff", png),
            Err(ImportError::NotADictionary("it_IT.aff".to_string()))
        );
        let mut late_nul = vec![b'a'; 4095];
        late_nul.push(0);
        assert_eq!(
            check_content("it_IT.dic", &late_nul),
            Err(ImportError::NotADictionary("it_IT.dic".to_string()))
        );
        let mut past_the_start = vec![b'a'; 4096];
        past_the_start.push(0);
        assert_eq!(check_content("it_IT.dic", &past_the_start), Ok(()));
    }

    #[test]
    fn an_import_creates_the_folder_and_the_two_names() {
        let source = test_dir("source");
        let dir = test_dir("user").join("dictionaries");
        let aff = write(&source, "es.aff.txt", b"SET UTF-8\n");
        let dic = write(&source, "es.dic", b"1\nhola\n");

        let installed = import_pair(&dir, &[dic, aff], &known(), false);

        assert_eq!(installed, Ok("es_ES".to_string()));
        assert_eq!(
            std::fs::read(dir.join("es_ES/es_ES.aff")).unwrap(),
            b"SET UTF-8\n"
        );
        assert_eq!(
            std::fs::read(dir.join("es_ES/es_ES.dic")).unwrap(),
            b"1\nhola\n"
        );
        assert!(!dir.join("es").exists());
        let mut names: Vec<String> = std::fs::read_dir(dir.join("es_ES"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["es_ES.aff", "es_ES.dic"]);
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn an_installed_language_is_kept_unless_replace_is_asked() {
        let source = test_dir("source");
        let dir = test_dir("user");
        std::fs::create_dir_all(dir.join("es_ES")).unwrap();
        std::fs::write(dir.join("es_ES/es_ES.dic"), b"1\nold\n").unwrap();
        let aff = write(&source, "es_ES.aff", b"SET UTF-8\n");
        let dic = write(&source, "es_ES.dic", b"1\nnew\n");
        let paths = [aff, dic];

        let refused = import_pair(&dir, &paths, &known(), false);
        assert_eq!(refused, Err(ImportError::Exists("es_ES".to_string())));
        assert_eq!(
            std::fs::read(dir.join("es_ES/es_ES.dic")).unwrap(),
            b"1\nold\n"
        );
        assert!(!dir.join("es_ES/es_ES.aff").exists());

        assert_eq!(
            import_pair(&dir, &paths, &known(), true),
            Ok("es_ES".to_string())
        );
        assert_eq!(
            std::fs::read(dir.join("es_ES/es_ES.dic")).unwrap(),
            b"1\nnew\n"
        );
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_check_writes_nothing() {
        let source = test_dir("source");
        let dir = test_dir("user");
        let aff = write(&source, "es.aff", b"<html><body>404</body></html>");
        let dic = write(&source, "es.dic", b"1\nhola\n");

        let refused = import_pair(&dir, &[aff, dic], &known(), false);

        assert_eq!(
            refused,
            Err(ImportError::NotADictionary("es.aff".to_string()))
        );
        assert!(!dir.join("es").exists());
        assert!(!dir.join("es_ES").exists());
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_known_code_keeps_its_folder() {
        assert_eq!(folder_for("es_ES", &known()), Ok("es_ES".to_string()));
        assert_eq!(folder_for("ar", &known()), Ok("ar".to_string()));
    }

    #[test]
    fn a_short_code_with_one_region_takes_it() {
        assert_eq!(folder_for("fr", &known()), Ok("fr_FR".to_string()));
        assert_eq!(folder_for("es", &known()), Ok("es_ES".to_string()));
    }

    #[test]
    fn a_short_code_with_several_regions_takes_its_main_one() {
        assert_eq!(folder_for("pt", &known()), Ok("pt_PT".to_string()));
    }

    #[test]
    fn several_regions_without_a_main_one_are_refused_with_the_options() {
        assert_eq!(
            folder_for("en", &known()),
            Err(ImportError::UnknownLanguage {
                lang: "en".to_string(),
                options: vec!["en_GB".to_string(), "en_US".to_string()],
            })
        );
        assert_eq!(
            folder_for("de", &known()),
            Err(ImportError::UnknownLanguage {
                lang: "de".to_string(),
                options: vec!["de_AT".to_string(), "de_CH".to_string()],
            })
        );
    }

    #[test]
    fn a_code_the_checker_does_not_know_is_refused() {
        assert_eq!(
            folder_for("xx", &known()),
            Err(ImportError::UnknownLanguage {
                lang: "xx".to_string(),
                options: vec![],
            })
        );
        assert_eq!(
            folder_for("fr_CA", &known()),
            Err(ImportError::UnknownLanguage {
                lang: "fr_CA".to_string(),
                options: vec![],
            })
        );
    }

    #[test]
    fn an_unknown_language_writes_nothing() {
        let source = test_dir("source");
        let dir = test_dir("user");
        let aff = write(&source, "xx.aff", b"SET UTF-8\n");
        let dic = write(&source, "xx.dic", b"1\nhola\n");

        let refused = import_pair(&dir, &[aff, dic], &known(), false);

        assert!(matches!(refused, Err(ImportError::UnknownLanguage { .. })));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_short_code_installs_under_the_checker_folder() {
        let source = test_dir("source");
        let dir = test_dir("user");
        let aff = write(&source, "fr.aff", b"SET UTF-8\n");
        let dic = write(&source, "fr.dic", b"1\nbonjour\n");

        let installed = import_pair(&dir, &[aff, dic], &known(), false);

        assert_eq!(installed, Ok("fr_FR".to_string()));
        assert!(dir.join("fr_FR/fr_FR.aff").exists());
        assert!(dir.join("fr_FR/fr_FR.dic").exists());
        assert!(!dir.join("fr").exists());
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn language_folder_names_follow_sdkjs() {
        for good in ["ar", "fr_FR", "sr_Latn_RS", "ca_ES_valencia", "kmr_TR"] {
            assert!(is_language_folder(good), "{good}");
        }
        for bad in [
            "",
            "_",
            "es_",
            "ES",
            "Es_ES",
            "es-ES",
            "es/ES",
            "es\\ES",
            "..",
            "es_..",
            "es_/../../outside/es_/..",
            "c:",
            "e",
            "abcd_ES",
        ] {
            assert!(!is_language_folder(bad), "{bad}");
        }
    }

    #[test]
    fn a_known_entry_that_is_a_path_is_ignored() {
        let hostile: Vec<String> = [
            "es_/../../outside/es_/..",
            "es_ES/..",
            "es_..\\..",
            "",
            "es_eS_/",
            "ES_ES",
        ]
        .iter()
        .map(|code| code.to_string())
        .collect();
        assert_eq!(
            folder_for("es", &hostile),
            Err(ImportError::UnknownLanguage {
                lang: "es".to_string(),
                options: vec![],
            })
        );

        let source = test_dir("source");
        let root = test_dir("user");
        let dir = root.join("dictionaries");
        let aff = write(&source, "es.aff", b"SET UTF-8\n");
        let dic = write(&source, "es.dic", b"1\nhola\n");
        let refused = import_pair(&dir, &[aff, dic], &hostile, false);
        assert!(matches!(refused, Err(ImportError::UnknownLanguage { .. })));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_refuses_a_folder_that_is_not_a_language() {
        let source = test_dir("source");
        let root = test_dir("user");
        let dir = root.join("dictionaries");
        let aff = write(&source, "es.aff", b"SET UTF-8\n");
        let dic = write(&source, "es.dic", b"1\nhola\n");
        for folder in ["../outside", "es_ES/..", ""] {
            let refused = install(&dir, folder, &aff, &dic, false);
            assert_eq!(refused, Err(ImportError::BadName(folder.to_string())));
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn installed_es_es(dir: &Path) {
        std::fs::create_dir_all(dir.join("es_ES")).unwrap();
        std::fs::write(dir.join("es_ES/es_ES.aff"), b"SET old\n").unwrap();
        std::fs::write(dir.join("es_ES/es_ES.dic"), b"1\nold\n").unwrap();
    }

    fn names_in(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_write_error_sent_to_the_webview_holds_no_path() {
        let source = test_dir("source");
        let dir = test_dir("user");
        installed_es_es(&dir);
        let aff = write(&source, "es_ES.aff", b"SET UTF-8\n");
        let missing = source.join("es_ES.dic");

        let refused = install(&dir, "es_ES", &aff, &missing, true).unwrap_err();
        let text = refused.to_string();

        assert_eq!(text, "dictionary-io: es_ES");
        assert!(!text.contains(&*source.to_string_lossy()));
        assert!(!text.contains(&*dir.to_string_lossy()));
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_second_write_keeps_the_installed_pair() {
        let source = test_dir("source");
        let dir = test_dir("user");
        installed_es_es(&dir);
        let aff = write(&source, "es_ES.aff", b"SET new\n");
        // A folder opens but cannot be read as a file.
        let dic = source.join("es_ES.dic");
        std::fs::create_dir(&dic).unwrap();

        let refused = install(&dir, "es_ES", &aff, &dic, true);

        assert_eq!(refused, Err(ImportError::Io("es_ES".to_string())));
        assert_eq!(
            std::fs::read(dir.join("es_ES/es_ES.aff")).unwrap(),
            b"SET old\n"
        );
        assert_eq!(
            std::fs::read(dir.join("es_ES/es_ES.dic")).unwrap(),
            b"1\nold\n"
        );
        assert_eq!(names_in(&dir.join("es_ES")), vec!["es_ES.aff", "es_ES.dic"]);
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_rename_keeps_the_installed_aff_and_no_temp_files() {
        let source = test_dir("source");
        let dir = test_dir("user");
        std::fs::create_dir_all(dir.join("es_ES")).unwrap();
        std::fs::write(dir.join("es_ES/es_ES.aff"), b"SET old\n").unwrap();
        // A file cannot be renamed over a non-empty folder.
        std::fs::create_dir_all(dir.join("es_ES/es_ES.dic/inside")).unwrap();
        let aff = write(&source, "es_ES.aff", b"SET new\n");
        let dic = write(&source, "es_ES.dic", b"1\nnew\n");

        let refused = install(&dir, "es_ES", &aff, &dic, true);

        assert_eq!(refused, Err(ImportError::Io("es_ES".to_string())));
        assert_eq!(
            std::fs::read(dir.join("es_ES/es_ES.aff")).unwrap(),
            b"SET old\n"
        );
        assert_eq!(names_in(&dir.join("es_ES")), vec!["es_ES.aff", "es_ES.dic"]);
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_refused() {
        let source = test_dir("source");
        let dir = test_dir("user");
        let outside = test_dir("outside");
        std::os::unix::fs::symlink(&outside, dir.join("es_ES")).unwrap();
        let aff = write(&source, "es_ES.aff", b"SET UTF-8\n");
        let dic = write(&source, "es_ES.dic", b"1\nhola\n");

        let refused = install(&dir, "es_ES", &aff, &dic, true);

        assert_eq!(refused, Err(ImportError::Io("es_ES".to_string())));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        for path in [&source, &dir, &outside] {
            let _ = std::fs::remove_dir_all(path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_dictionaries_folder_is_refused() {
        let source = test_dir("source");
        let root = test_dir("user");
        let outside = test_dir("outside");
        let dir = root.join("dictionaries");
        std::os::unix::fs::symlink(&outside, &dir).unwrap();
        let aff = write(&source, "es_ES.aff", b"SET UTF-8\n");
        let dic = write(&source, "es_ES.dic", b"1\nhola\n");

        let refused = install(&dir, "es_ES", &aff, &dic, true);

        assert_eq!(refused, Err(ImportError::Io("es_ES".to_string())));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        for path in [&source, &root, &outside] {
            let _ = std::fs::remove_dir_all(path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_file_is_refused() {
        let source = test_dir("source");
        let dir = test_dir("user");
        let outside = test_dir("outside");
        let target = write(&outside, "target", b"kept\n");
        std::fs::create_dir_all(dir.join("es_ES")).unwrap();
        std::os::unix::fs::symlink(&target, dir.join("es_ES/es_ES.dic")).unwrap();
        let aff = write(&source, "es_ES.aff", b"SET UTF-8\n");
        let dic = write(&source, "es_ES.dic", b"1\nhola\n");

        let refused = install(&dir, "es_ES", &aff, &dic, true);

        assert_eq!(refused, Err(ImportError::Io("es_ES".to_string())));
        assert_eq!(std::fs::read(&target).unwrap(), b"kept\n");
        assert!(!dir.join("es_ES/es_ES.aff").exists());
        for path in [&source, &dir, &outside] {
            let _ = std::fs::remove_dir_all(path);
        }
    }

    #[test]
    fn a_replace_uses_the_folder_the_user_confirmed() {
        let source = test_dir("source");
        let dir = test_dir("user");
        std::fs::create_dir_all(dir.join("pt_PT")).unwrap();
        std::fs::write(dir.join("pt_PT/pt_PT.dic"), b"1\nold\n").unwrap();
        let aff = write(&source, "pt.aff", b"SET UTF-8\n");
        let dic = write(&source, "pt.dic", b"1\nnovo\n");

        let (result, to_confirm) = start_import(&dir, &[aff, dic], &known());
        assert_eq!(result, Err(ImportError::Exists("pt_PT".to_string())));
        let confirmed = to_confirm.unwrap();
        assert_eq!(confirmed.folder, "pt_PT");

        // The second call carries no language list, so a new one sent then
        // (only pt_BR) cannot move the replace.
        assert_eq!(finish_replace(&dir, &confirmed), Ok("pt_PT".to_string()));
        assert_eq!(
            std::fs::read(dir.join("pt_PT/pt_PT.dic")).unwrap(),
            b"1\nnovo\n"
        );
        assert!(!dir.join("pt_BR").exists());
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_an_existing_language_waits_for_a_replace() {
        let source = test_dir("source");
        let dir = test_dir("user");
        let aff = write(&source, "fr.aff", b"SET UTF-8\n");
        let dic = write(&source, "fr.dic", b"1\nbonjour\n");
        let (result, to_confirm) = start_import(&dir, &[aff.clone(), dic.clone()], &known());
        assert_eq!(result, Ok("fr_FR".to_string()));
        assert_eq!(to_confirm, None);
        let (result, to_confirm) = start_import(&dir, &[aff], &known());
        assert_eq!(result, Err(ImportError::NotAPair));
        assert_eq!(to_confirm, None);
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
        let unknown = ImportError::UnknownLanguage {
            lang: "en".to_string(),
            options: vec!["en_GB".to_string(), "en_US".to_string()],
        };
        assert_eq!(
            unknown.to_string(),
            "dictionary-unknown-language: en (en_GB, en_US)"
        );
    }
}
