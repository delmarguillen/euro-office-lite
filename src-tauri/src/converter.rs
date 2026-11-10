use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

use crate::x2t_task::{self, convert_params_xml, params_path, write_file, FONT_CACHE_ERROR};

// A conversion to Editor.bin (8192) takes the path of an EditorInput: the copy
// prepare_editor_input checked.
pub async fn convert_file(
    app: &AppHandle,
    input: &str,
    output: &str,
    _format_from: i32,
    format_to: i32,
    temp_dir: &str,
    // The shared font cache: the PDF export reads it, every other conversion
    // ignores it.
    fontdata_dir: &std::path::Path,
    text_options: Option<crate::text_import::TextOptions>,
) -> Result<String, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let binaries_dir = resource_dir.join("binaries");

    if format_to == 513 {
        let input = input.to_string();
        let output = output.to_string();
        let temp_dir = std::path::PathBuf::from(temp_dir);
        let fontdata = fontdata_dir.to_path_buf();
        return run_pdf_job(app, binaries_dir, move |binaries_dir, exe_dir| {
            x2t_task::convert_to_pdf(
                binaries_dir,
                exe_dir,
                &input,
                &output,
                &temp_dir,
                &fontdata,
                &log_pdf,
            )
        })
        .await;
    }

    let params_path = params_path(std::path::Path::new(temp_dir), "convert");

    let xml = convert_params_xml(
        input,
        output,
        format_to,
        temp_dir,
        // Only set for CSV and TXT: x2t will not read them without an encoding
        // and, for CSV, a delimiter (exit 89), and writes with these too.
        &text_options
            .map(|options| options.xml_elements())
            .unwrap_or_default(),
    );
    write_file(&params_path, &xml)?;

    let command = x2t_command(app, &binaries_dir)?
        .current_dir(&binaries_dir)
        .args([params_path.to_string_lossy().as_ref()])
        .envs(x2t_task::convert_env(&binaries_dir));
    let result = command.output().await.map_err(|e| e.to_string())?;

    let _ = std::fs::remove_file(&params_path);

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let code = result.status.code().unwrap_or(-1);
        Err(format!(
            "x2t conversion failed (exit code {}): {}",
            code, stderr
        ))
    }
}

// Applies the PDF editor's changes to `source_pdf` and writes the result to
// `output`. The changes are in changes/ next to `source_pdf`.
pub async fn apply_pdf_changes(
    app: &AppHandle,
    source_pdf: &std::path::Path,
    output: &std::path::Path,
    temp_dir: &std::path::Path,
    fontdata_dir: &std::path::Path,
) -> Result<String, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let source_pdf = source_pdf.to_string_lossy().into_owned();
    let output = output.to_string_lossy().into_owned();
    let temp_dir = temp_dir.to_path_buf();
    let fontdata = fontdata_dir.to_path_buf();
    run_pdf_job(app, resource_dir.join("binaries"), move |binaries_dir, exe_dir| {
        x2t_task::apply_pdf_changes(
            binaries_dir,
            exe_dir,
            &source_pdf,
            &output,
            &temp_dir,
            &fontdata,
            &log_pdf,
        )
    })
    .await
}

// Runs an x2t job that writes a PDF, once the font cache it reads exists.
async fn run_pdf_job<F>(
    app: &AppHandle,
    binaries_dir: std::path::PathBuf,
    job: F,
) -> Result<String, String>
where
    F: FnOnce(&std::path::Path, Option<&std::path::Path>) -> Result<String, String>
        + Send
        + 'static,
{
    let (font_root, instance_id) = {
        let state = app.state::<crate::file_ops::AppState>();
        (state.font_root.clone(), state.instance.id.clone())
    };
    let exe_dir = crate::app_exe_dir();
    // Building the font cache can wait up to FONT_CACHE_WAIT and x2t takes
    // seconds: neither should hold a thread of the async runtime.
    tauri::async_runtime::spawn_blocking(move || {
        ensure_pdf_font_cache(
            &font_root,
            &instance_id,
            |staging: &std::path::Path| crate::generate_fonts(staging, &binaries_dir),
            crate::FONT_CACHE_WAIT,
            crate::FONT_CACHE_POLL,
        )?;
        job(&binaries_dir, exe_dir.as_deref())
    })
    .await
    .map_err(|e| format!("PDF conversion task failed: {}", e))?
}

// Windows runs the same x2t as the PDF export, the one in binaries\ (see
// x2t_task::find_x2t); Linux and macOS keep the sidecar.
#[cfg(target_os = "windows")]
fn x2t_command(
    app: &AppHandle,
    binaries_dir: &std::path::Path,
) -> Result<tauri_plugin_shell::process::Command, String> {
    let binaries_dir = x2t_task::strip_extended_prefix(binaries_dir);
    let x2t = x2t_task::find_x2t(&binaries_dir, None)?;
    Ok(app.shell().command(x2t))
}

#[cfg(not(target_os = "windows"))]
fn x2t_command(
    app: &AppHandle,
    _binaries_dir: &std::path::Path,
) -> Result<tauri_plugin_shell::process::Command, String> {
    app.shell().sidecar("x2t").map_err(|e| e.to_string())
}

fn log_pdf(msg: &str) {
    crate::temp_layout::append_log(&format!("[PDF] {}", msg));
}

// Without the font cache x2t still exits 0 but the PDF loses its text: the font
// list bundled with the app has no font selection table, and the renderer
// needs one. So a missing cache is built here, and without it there is no PDF.
fn ensure_pdf_font_cache(
    font_root: &std::path::Path,
    instance_id: &str,
    generate: impl FnOnce(&std::path::Path) -> bool,
    wait: std::time::Duration,
    poll: std::time::Duration,
) -> Result<(), String> {
    if crate::temp_layout::fonts_ready(font_root) {
        return Ok(());
    }
    log_pdf("font cache missing, building it before the export");
    let outcome =
        crate::temp_layout::ensure_font_cache(font_root, instance_id, generate, wait, poll);
    // A failed run can leave files that look complete next to an old marker,
    // so the outcome has to say the cache is good too.
    use crate::temp_layout::FontCache;
    let built = matches!(
        outcome,
        FontCache::Ready | FontCache::Generated | FontCache::GeneratedByOther
    );
    if built && crate::temp_layout::fonts_ready(font_root) {
        log_pdf(&format!("font cache ready: {:?}", outcome));
        Ok(())
    } else {
        let error = format!(
            "{}: the font cache could not be built ({:?})",
            FONT_CACHE_ERROR, outcome
        );
        log_pdf(&error);
        Err(error)
    }
}

// What x2t converts to Editor.bin when a document is opened or inserted: a copy
// in the instance's folder, made from one read of the user's file. The checks
// read that copy, and x2t reads the same bytes. Every copy is removed when this
// is dropped, whether the conversion worked or not.
pub struct EditorInput {
    copies: Vec<std::path::PathBuf>,
    // The options x2t reads a TXT or CSV copy with.
    pub options: Option<crate::text_import::TextOptions>,
}

impl EditorInput {
    // The last copy made: an ODF package's references are emptied in a copy of
    // the first one.
    pub fn path(&self) -> &std::path::Path {
        self.copies.last().expect("an EditorInput holds at least one copy")
    }
}

impl Drop for EditorInput {
    fn drop(&mut self) {
        for copy in &self.copies {
            let _ = std::fs::remove_file(copy);
        }
    }
}

// The TXT and CSV copy (text_import::prepare_read_copy) is already one; every
// other document gets a copy of its own.
pub fn prepare_editor_input(
    source: &std::path::Path,
    work_dir: &std::path::Path,
    text_kind: Option<crate::text_import::TextKind>,
    text_options: Option<crate::text_import::TextOptions>,
) -> Result<EditorInput, String> {
    // Nothing is copied under an extension the app does not convert.
    if !x2t_task::is_converted_extension(source) {
        return Err(not_valid_error(source));
    }
    let mut input = read_copy(source, work_dir, text_kind, text_options)?;
    let copy = input.path().to_path_buf();
    let content = x2t_task::ContentSample::read(&copy).map_err(|e| e.to_string())?;
    check_container_content(&copy, &content)?;
    if let Some(package_copy) = package_images_copy(&copy, &content, work_dir)? {
        input.copies.push(package_copy);
    }
    Ok(input)
}

fn read_copy(
    source: &std::path::Path,
    work_dir: &std::path::Path,
    text_kind: Option<crate::text_import::TextKind>,
    text_options: Option<crate::text_import::TextOptions>,
) -> Result<EditorInput, String> {
    let text_copy = crate::text_import::prepare_read_copy(source, work_dir, text_kind, text_options)
        .map_err(|e| format!("read copy: {}", e))?;
    if let Some(copy) = text_copy {
        return Ok(EditorInput { copies: vec![copy.path], options: Some(copy.options) });
    }
    let copy = crate::text_import::document_read_copy(source, work_dir)
        .map_err(|e| format!("read copy: {}", e))?;
    Ok(EditorInput { copies: vec![copy], options: None })
}

// The copy x2t reads an ODF package from when one of its picture references has
// been emptied, or None to read the one it was given.
fn package_images_copy(
    path: &std::path::Path,
    content: &x2t_task::ContentSample,
    temp_dir: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    use crate::odf_links::PackageImages;
    match crate::odf_links::prepare(path, content, temp_dir) {
        PackageImages::Unchanged => Ok(None),
        PackageImages::Copy(copy) => Ok(Some(copy)),
        PackageImages::NotAllowed => Err(not_valid_error(path)),
    }
}

fn not_valid_error(path: &std::path::Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    format!("This file is not a valid .{}", ext.to_lowercase())
}

// Opening and inserting convert the user's file to Editor.bin (8192); this runs
// on the copy x2t reads, before x2t is launched for them.
fn check_container_content(
    path: &std::path::Path,
    content: &x2t_task::ContentSample,
) -> Result<(), String> {
    let matches_format = x2t_task::content_matches_extension(path, content);
    if matches_format
        && opens_if_it_starts_like_a_zip(path, &content.head)
        && crate::package_relationships::relationships_are_allowed(path, content)
    {
        return Ok(());
    }
    Err(not_valid_error(path))
}

// x2t picks the format of a file that starts like a ZIP but does not open as
// one from the rest of its bytes.
fn opens_if_it_starts_like_a_zip(path: &std::path::Path, head: &[u8]) -> bool {
    if !head.starts_with(b"PK\x03\x04") {
        return true;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    zip::ZipArchive::new(file).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x2t_task::pdf_font_list;

    const WAIT: std::time::Duration = std::time::Duration::from_secs(5);
    const POLL: std::time::Duration = std::time::Duration::from_millis(10);

    fn scratch_root() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "eo-converter-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn content_check(name: &str, content: &[u8]) -> Result<(), String> {
        let path = scratch_root().join(name);
        std::fs::write(&path, content).unwrap();
        check_container_content(&path, &x2t_task::ContentSample::read(&path).unwrap())
    }

    fn zip_with(name: &str, content: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(content).unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn a_file_that_starts_like_a_zip_but_does_not_open_as_one_is_refused() {
        let mut broken = b"PK\x03\x04".to_vec();
        broken.extend_from_slice(&[0u8; 60]);
        broken.extend_from_slice(b"<html><body>text</body></html>");
        for name in ["broken.docx", "broken.odt", "broken.xlsb", "broken.doc"] {
            assert!(content_check(name, &broken).is_err(), "{name}");
        }
        assert_eq!(content_check("whole.docx", &zip_with("word/document.xml", b"<w:document/>")), Ok(()));
    }

    #[test]
    fn an_rtf_or_a_flat_odf_followed_by_a_zip_is_refused() {
        let package = zip_with("[Content_Types].xml", b"<Types/>");
        let starts: [(&str, &[u8]); 2] = [
            ("joined.rtf", b"{\\rtf1\\ansi Some text\\par}\n"),
            ("joined.fodt", b"<?xml version=\"1.0\"?><office:document office:version=\"1.3\"></office:document>\n"),
        ];
        for (name, start) in starts {
            assert_eq!(content_check(name, start), Ok(()), "{name}");
            let mut joined = start.to_vec();
            joined.extend_from_slice(&package);
            assert!(content_check(name, &joined).is_err(), "{name}");
        }
    }

    #[test]
    fn a_word_97_doc_holding_a_zip_is_accepted() {
        let mut doc = include_bytes!("../../tests/fixtures/word97.doc").to_vec();
        doc.extend_from_slice(&zip_with("embedded.xml", b"<a/>"));
        assert_eq!(content_check("embedded.doc", &doc), Ok(()));
    }

    #[test]
    fn a_text_file_named_doc_or_fodt_is_refused_before_x2t() {
        let text = b"Hello, this is plain text.\n";
        assert_eq!(
            content_check("fake.doc", text),
            Err("This file is not a valid .doc".to_string())
        );
        assert_eq!(
            content_check("fake.FODT", text),
            Err("This file is not a valid .fodt".to_string())
        );
    }

    #[test]
    fn a_flat_odf_after_a_bom_and_blank_lines_is_accepted() {
        let mut content = b"\xEF\xBB\xBF".to_vec();
        content.extend_from_slice(&[b'\n'; 40]);
        content.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document/>");
        assert_eq!(content_check("flat.fodt", &content), Ok(()));
    }

    #[test]
    fn a_package_with_a_local_image_link_is_refused_before_x2t() {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("word/_rels/document.xml.rels", options).unwrap();
        writer
            .write_all(
                b"<Relationships><Relationship Id=\"r\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"/a/b.png\" TargetMode=\"External\"/></Relationships>",
            )
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert_eq!(
            content_check("linked.docx", &bytes),
            Err("This file is not a valid .docx".to_string())
        );
    }

    #[test]
    fn a_real_word_97_doc_passes_the_content_check() {
        let doc = include_bytes!("../../tests/fixtures/word97.doc");
        assert_eq!(content_check("word97.doc", doc), Ok(()));
    }

    // The original in one folder, the instance's work folder next to it.
    fn original_and_work_dir(name: &str, content: &[u8]) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = scratch_root();
        let original = root.join(name);
        std::fs::write(&original, content).unwrap();
        let work_dir = root.join("work");
        std::fs::create_dir_all(&work_dir).unwrap();
        (original, work_dir)
    }

    fn files_in(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }

    const DOCX: &[u8] = include_bytes!("../../tests/corpus/rich.docx");

    #[test]
    fn the_conversion_reads_a_copy_in_the_work_folder_not_the_original() {
        let (original, work_dir) = original_and_work_dir("Report.docx", DOCX);

        let input = prepare_editor_input(&original, &work_dir, None, None).unwrap();

        assert_ne!(input.path(), original.as_path());
        assert_eq!(input.path().parent(), Some(work_dir.as_path()));
        assert_eq!(std::fs::read(input.path()).unwrap(), DOCX);
        assert_eq!(input.options, None);
    }

    #[test]
    fn the_copy_is_removed_when_the_conversion_ends_whatever_its_outcome() {
        for outcome in [Ok(()), Err("x2t conversion failed (exit code 89)")] {
            let (original, work_dir) = original_and_work_dir("Report.docx", DOCX);
            let input = prepare_editor_input(&original, &work_dir, None, None).unwrap();
            let copy = input.path().to_path_buf();

            assert!(copy.exists());
            // What open_file and convert_for_insert do once x2t returns.
            let converted: Result<(), &str> = outcome;
            drop(input);

            assert!(!copy.exists(), "{:?}", converted);
            assert_eq!(files_in(&work_dir), 0, "{:?}", converted);
            assert_eq!(std::fs::read(&original).unwrap(), DOCX, "the original stays");
        }
    }

    #[test]
    fn no_copy_stays_when_the_copy_is_refused() {
        let (original, work_dir) = original_and_work_dir("fake.doc", b"Hello, plain text.\n");

        let refused = prepare_editor_input(&original, &work_dir, None, None);

        assert_eq!(refused.err(), Some("This file is not a valid .doc".to_string()));
        assert_eq!(files_in(&work_dir), 0);
    }

    #[test]
    fn an_rtf_or_a_doc_after_blank_space_or_a_bom_is_converted_from_its_first_brace() {
        let rtf = b"{\\rtf1\\ansi Some text\\par}";
        for prefix in [&b"  \r\n"[..], b"\xEF\xBB\xBF", b"\xEF\xBB\xBF\t "] {
            for name in ["a.rtf", "a.doc"] {
                let mut content = prefix.to_vec();
                content.extend_from_slice(rtf);
                let (original, work_dir) = original_and_work_dir(name, &content);

                let input = prepare_editor_input(&original, &work_dir, None, None).unwrap();

                assert_eq!(std::fs::read(input.path()).unwrap(), rtf, "{name} {prefix:?}");
            }
        }
    }

    #[test]
    fn an_extension_outside_the_list_is_refused_before_any_copy() {
        for name in ["data.xml", "page.html", "Book.xltm", "noextension"] {
            let (original, work_dir) = original_and_work_dir(name, DOCX);

            let refused = prepare_editor_input(&original, &work_dir, None, None);

            assert!(refused.is_err(), "{name}");
            assert_eq!(files_in(&work_dir), 0, "{name}");
        }
        let (original, work_dir) = original_and_work_dir("data.xml", b"<?xml version=\"1.0\"?><a/>");
        let refused = prepare_editor_input(&original, &work_dir, None, None);
        assert_eq!(refused.err(), Some("This file is not a valid .xml".to_string()));
    }

    #[test]
    fn a_word_97_doc_is_converted_from_a_copy_as_it_is() {
        let doc = include_bytes!("../../tests/fixtures/word97.doc");
        let (original, work_dir) = original_and_work_dir("word97.doc", doc);

        let input = prepare_editor_input(&original, &work_dir, None, None).unwrap();

        assert_eq!(std::fs::read(input.path()).unwrap(), doc);
    }

    #[test]
    fn a_flat_odf_is_converted_from_a_copy_with_its_local_references_emptied() {
        let image = |href: &str| {
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document xmlns:office=\"o\" xmlns:draw=\"d\" xmlns:xlink=\"x\"><draw:image xlink:href=\"{href}\"/></office:document>")
        };
        let (original, work_dir) = original_and_work_dir("linked.fodt", image("Pictures/a.png").as_bytes());

        let input = prepare_editor_input(&original, &work_dir, None, None).unwrap();

        assert_eq!(input.path().parent(), Some(work_dir.as_path()));
        assert_eq!(std::fs::read_to_string(input.path()).unwrap(), image(""));
        assert_eq!(std::fs::read_to_string(&original).unwrap(), image("Pictures/a.png"), "the original stays");
        drop(input);
        assert_eq!(files_in(&work_dir), 0);
    }

    #[test]
    fn a_txt_is_read_from_its_utf32_copy_only() {
        use crate::text_import::{detect_file, TextKind, ENCODING_UTF32_LE};
        let (original, work_dir) = original_and_work_dir("notes.txt", b"line one\nline two\n");
        let options = detect_file(&original, TextKind::Txt).unwrap();

        let input = prepare_editor_input(&original, &work_dir, Some(TextKind::Txt), Some(options)).unwrap();

        assert_eq!(input.path().parent(), Some(work_dir.as_path()));
        assert_eq!(input.options.map(|o| o.encoding), Some(ENCODING_UTF32_LE));
        assert_eq!(files_in(&work_dir), 1, "no second copy");
    }

    // A temp root whose font cache is missing, next to an app folder whose
    // binaries/ holds the bundled list, as an installed app has.
    fn root_without_cache() -> (std::path::PathBuf, std::path::PathBuf) {
        let base = scratch_root();
        let root = base.join("temp");
        std::fs::create_dir_all(&root).unwrap();
        let binaries = base.join("binaries");
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::write(binaries.join("AllFonts.js"), b"bundled").unwrap();
        (base, root)
    }

    fn write_cache(fontdata: &std::path::Path, contents: &[u8]) {
        std::fs::create_dir_all(fontdata).unwrap();
        std::fs::write(fontdata.join("AllFonts.js"), contents).unwrap();
        std::fs::write(fontdata.join("font_selection.bin"), contents).unwrap();
    }

    // What convert_file does before x2t runs: the font list the PDF would be
    // made with, or the error that stops the export.
    fn pdf_list_for_export(
        root: &std::path::Path,
        generate: impl FnOnce(&std::path::Path) -> bool,
        wait: std::time::Duration,
    ) -> Result<std::path::PathBuf, String> {
        ensure_pdf_font_cache(root, "1-1", generate, wait, POLL)?;
        pdf_font_list(&crate::temp_layout::fontdata_dir(root))
    }

    #[test]
    fn an_existing_font_cache_is_used_without_building_it() {
        let (base, root) = root_without_cache();
        write_cache(&crate::temp_layout::fontdata_dir(&root), b"cache");
        let marker = root.join(crate::temp_layout::FONTS_MARKER);
        std::fs::write(marker, crate::temp_layout::expected_marker()).unwrap();

        let list = pdf_list_for_export(&root, |_: &std::path::Path| panic!("built again"), WAIT);

        assert_eq!(list, Ok(crate::temp_layout::fontdata_dir(&root).join("AllFonts.js")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_missing_font_cache_is_built_before_the_pdf() {
        let (base, root) = root_without_cache();

        let list = pdf_list_for_export(
            &root,
            |staging: &std::path::Path| {
                write_cache(staging, b"new");
                true
            },
            WAIT,
        );

        let expected = crate::temp_layout::fontdata_dir(&root).join("AllFonts.js");
        assert_eq!(list, Ok(expected.clone()));
        assert_eq!(std::fs::read(expected).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn no_pdf_when_the_font_cache_cannot_be_built() {
        let (base, root) = root_without_cache();

        let list = pdf_list_for_export(&root, |_: &std::path::Path| false, WAIT);

        let error = list.expect_err("a PDF without the font cache loses its text");
        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        assert!(error.contains("Failed"), "{}", error);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn no_pdf_when_another_instance_keeps_building_past_the_wait() {
        let (base, root) = root_without_cache();
        let _held = crate::temp_layout::take_lock(&root.join("fonts.lock")).unwrap();

        let list = pdf_list_for_export(
            &root,
            |_: &std::path::Path| panic!("built while another instance holds the lock"),
            std::time::Duration::from_millis(200),
        );

        let error = list.expect_err("a PDF without the font cache loses its text");
        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        assert!(error.contains("GaveUp"), "{}", error);
        let _ = std::fs::remove_dir_all(&base);
    }

    // A marker that survived next to a cache missing font_selection.bin is a
    // half-installed cache: it is rebuilt, not used.
    #[test]
    fn a_cache_without_its_selection_file_is_rebuilt() {
        let (base, root) = root_without_cache();
        let fontdata = crate::temp_layout::fontdata_dir(&root);
        std::fs::create_dir_all(&fontdata).unwrap();
        std::fs::write(fontdata.join("AllFonts.js"), b"old").unwrap();
        let marker = root.join(crate::temp_layout::FONTS_MARKER);
        std::fs::write(marker, crate::temp_layout::expected_marker()).unwrap();

        assert!(!crate::temp_layout::fonts_ready(&root));
        let list = pdf_list_for_export(
            &root,
            |staging: &std::path::Path| {
                write_cache(staging, b"new");
                true
            },
            WAIT,
        );

        assert_eq!(list, Ok(fontdata.join("AllFonts.js")));
        assert_eq!(std::fs::read(fontdata.join("AllFonts.js")).unwrap(), b"new");
        assert_eq!(std::fs::read(fontdata.join("font_selection.bin")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&base);
    }

    // The files land but the run still fails (here the new marker cannot be
    // written): the marker of the earlier run went before the files did, so
    // the cache does not pass for ready either.
    #[test]
    fn no_pdf_when_the_build_fails_next_to_an_old_marker() {
        let (base, root) = root_without_cache();
        let marker = root.join(crate::temp_layout::FONTS_MARKER);
        std::fs::write(&marker, crate::temp_layout::expected_marker()).unwrap();
        // write_replacing writes the marker through this file first.
        std::fs::create_dir_all(marker.with_extension("tmp-1-1")).unwrap();

        let list = pdf_list_for_export(
            &root,
            |staging: &std::path::Path| {
                write_cache(staging, b"new");
                true
            },
            WAIT,
        );

        assert!(!crate::temp_layout::fonts_ready(&root), "the old marker is gone");
        let error = list.expect_err("a failed build is not a usable cache");
        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        assert!(error.contains("Failed"), "{}", error);
        let _ = std::fs::remove_dir_all(&base);
    }
}
