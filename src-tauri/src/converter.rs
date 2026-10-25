use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

use crate::x2t_task::{self, convert_params_xml, params_path, write_file, FONT_CACHE_ERROR};

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
        let (temp_root, instance_id) = {
            let state = app.state::<crate::file_ops::AppState>();
            (state.temp_root.clone(), state.instance.id.clone())
        };
        let input = input.to_string();
        let output = output.to_string();
        let temp_dir = std::path::PathBuf::from(temp_dir);
        let fontdata_dir = fontdata_dir.to_path_buf();
        let exe_dir = crate::app_exe_dir();
        // Building the font cache can wait up to FONT_CACHE_WAIT and x2t takes
        // seconds: neither should hold a thread of the async runtime.
        return tauri::async_runtime::spawn_blocking(move || {
            ensure_pdf_font_cache(
                &temp_root,
                &instance_id,
                |staging: &std::path::Path| crate::generate_fonts(staging, &binaries_dir),
                crate::FONT_CACHE_WAIT,
                crate::FONT_CACHE_POLL,
            )?;
            x2t_task::convert_to_pdf(
                &binaries_dir,
                exe_dir.as_deref(),
                &input,
                &output,
                &temp_dir,
                &fontdata_dir,
                &log_pdf,
            )
        })
        .await
        .map_err(|e| format!("PDF conversion task failed: {}", e))?;
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
    temp_root: &std::path::Path,
    instance_id: &str,
    generate: impl FnOnce(&std::path::Path) -> bool,
    wait: std::time::Duration,
    poll: std::time::Duration,
) -> Result<(), String> {
    if crate::temp_layout::fonts_ready(temp_root) {
        return Ok(());
    }
    log_pdf("font cache missing, building it before the export");
    let outcome =
        crate::temp_layout::ensure_font_cache(temp_root, instance_id, generate, wait, poll);
    // A failed run can leave files that look complete next to an old marker,
    // so the outcome has to say the cache is good too.
    use crate::temp_layout::FontCache;
    let built = matches!(
        outcome,
        FontCache::Ready | FontCache::Generated | FontCache::GeneratedByOther
    );
    if built && crate::temp_layout::fonts_ready(temp_root) {
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
        std::fs::write(root.join(crate::temp_layout::FONTS_MARKER), b"generated").unwrap();

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
        std::fs::write(root.join(crate::temp_layout::FONTS_MARKER), b"generated").unwrap();

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
    // written), next to the marker of an earlier run: the files look complete,
    // the outcome says they are not.
    #[test]
    fn no_pdf_when_the_build_fails_next_to_an_old_marker() {
        let (base, root) = root_without_cache();
        let marker = root.join(crate::temp_layout::FONTS_MARKER);
        std::fs::write(&marker, b"generated").unwrap();
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

        assert!(crate::temp_layout::fonts_ready(&root), "the files look complete");
        let error = list.expect_err("a failed build is not a usable cache");
        assert!(error.starts_with(FONT_CACHE_ERROR), "{}", error);
        assert!(error.contains("Failed"), "{}", error);
        let _ = std::fs::remove_dir_all(&base);
    }
}
