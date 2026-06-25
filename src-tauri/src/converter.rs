use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

pub async fn convert_file(
    app: &AppHandle,
    input: &str,
    output: &str,
    _format_from: i32,
    format_to: i32,
) -> Result<String, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let binaries_dir = resource_dir.join("binaries");

    if format_to == 513 {
        return convert_to_pdf(&binaries_dir, input, output);
    }

    let mut sidecar = app
        .shell()
        .sidecar("x2t")
        .map_err(|e| e.to_string())?
        .current_dir(&binaries_dir)
        .args([input, output]);
    #[cfg(target_os = "linux")]
    {
        sidecar = sidecar.envs([("LD_LIBRARY_PATH", binaries_dir.to_string_lossy().as_ref())]);
    }
    let result = sidecar
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let code = result.status.code().unwrap_or(-1);
        Err(format!("x2t conversion failed (exit code {}): {}", code, stderr))
    }
}

fn log_pdf(msg: &str) {
    use std::io::Write;
    let log_path = std::env::temp_dir().join("euro-office-lite").join("js-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "[PDF] {}", msg);
    }
}

fn log_dir_recursive(dir: &std::path::Path, base: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap_or(&path);
            if path.is_dir() {
                log_pdf(&format!("  DIR  {}/", rel.display()));
                log_dir_recursive(&path, base);
            } else {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                log_pdf(&format!("  FILE {} ({} bytes)", rel.display(), size));
            }
        }
    }
}

fn log_file_access(path: &std::path::Path, label: &str) {
    let exists = path.exists();
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let readable = std::fs::File::open(path).is_ok();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map(|m| format!("{:o}", m.permissions().mode()))
            .unwrap_or_else(|_| "???".into());
        log_pdf(&format!("[{}] {:?} exists={} size={} readable={} mode={}",
            label, path, exists, size, readable, mode));
    }
    #[cfg(not(target_os = "linux"))]
    {
        log_pdf(&format!("[{}] {:?} exists={} size={} readable={}",
            label, path, exists, size, readable));
    }
    if readable && size > 0 {
        let first_bytes = std::fs::read(path).map(|d| d.len()).unwrap_or(0);
        log_pdf(&format!("[{}] actually read {} bytes OK", label, first_bytes));
    }
}

fn convert_to_pdf(
    binaries_dir: &std::path::Path,
    input: &str,
    output: &str,
) -> Result<String, String> {
    log_pdf("=== convert_to_pdf START ===");
    let x2t_exe = find_x2t_exe(binaries_dir)?;
    let fonts_dir = binaries_dir.join("fonts");
    let fontdata_dir = std::env::temp_dir().join("euro-office-lite").join("fontdata");
    let editors_dir = binaries_dir.parent().unwrap_or(binaries_dir).join("editors");

    log_pdf(&format!("binaries_dir={}", binaries_dir.display()));
    log_pdf(&format!("x2t={}", x2t_exe.display()));
    log_pdf(&format!("input={}", input));
    log_pdf(&format!("output={}", output));

    // --- STEP 1: Verify input file ---
    log_pdf("--- STEP 1: Input file ---");
    log_file_access(std::path::Path::new(input), "Editor.bin");

    // --- STEP 2: List binaries/ ---
    log_pdf("--- STEP 2: Files in binaries/ ---");
    if let Ok(entries) = std::fs::read_dir(binaries_dir) {
        for entry in entries.flatten() {
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            log_pdf(&format!("  {} {} {}",
                if is_dir { "DIR " } else { "FILE" },
                entry.file_name().to_string_lossy(),
                if is_dir { String::new() } else { format!("({} bytes)", size) }
            ));
        }
    }

    // --- STEP 3: List editors/ ---
    log_pdf("--- STEP 3: editors/ structure ---");
    log_pdf(&format!("editors/ exists: {} (at {:?})", editors_dir.exists(), editors_dir));
    if editors_dir.exists() {
        log_dir_recursive(&editors_dir, &editors_dir);
    }

    // --- STEP 4: DoctRenderer.config ---
    log_pdf("--- STEP 4: DoctRenderer.config ---");
    let config_path = binaries_dir.join("DoctRenderer.config");
    log_file_access(&config_path, "DoctRenderer.config");
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        log_pdf(&content);
    }

    // --- STEP 5: font_selection.bin — check readability ---
    log_pdf("--- STEP 5: font_selection.bin ---");
    let fontsel_src = fontdata_dir.join("font_selection.bin");
    log_file_access(&fontsel_src, "font_selection.bin (fontdata)");

    // --- STEP 6: AllFonts.js ---
    log_pdf("--- STEP 6: AllFonts.js ---");
    let allfonts_fontdata = fontdata_dir.join("AllFonts.js");
    let allfonts_binaries = binaries_dir.join("AllFonts.js");

    log_file_access(&allfonts_fontdata, "AllFonts.js (fontdata/server)");
    log_file_access(&allfonts_binaries, "AllFonts.js (binaries)");

    let allfonts_js = if allfonts_fontdata.exists() {
        allfonts_fontdata.clone()
    } else {
        allfonts_binaries.clone()
    };
    let allfonts_size = std::fs::metadata(&allfonts_js).map(|m| m.len()).unwrap_or(0);
    log_pdf(&format!("Selected AllFonts.js: {:?} ({} bytes)", allfonts_js, allfonts_size));

    if let Ok(content) = std::fs::read_to_string(&allfonts_js) {
        for (i, line) in content.lines().take(5).enumerate() {
            let end = line.len().min(200);
            log_pdf(&format!("  L{}: {}", i + 1, &line[..end]));
        }
        log_pdf(&format!("  ... total {} lines", content.lines().count()));
    }

    // --- STEP 7: Build writable mirror in /tmp for files DoctRenderer needs ---
    // /usr/lib/ is read-only. DoctRenderer.config uses relative paths from x2t binary.
    // We create a writable work dir that mirrors the expected layout and run x2t from there.
    log_pdf("--- STEP 7: Build writable work directory ---");
    let work_dir = std::env::temp_dir().join("euro-office-lite").join("x2t-workdir");
    let work_binaries = work_dir.join("binaries");
    let work_editors = work_dir.join("editors");
    let work_dictionaries = work_dir.join("dictionaries");

    let _ = std::fs::create_dir_all(&work_binaries);
    let _ = std::fs::create_dir_all(&work_editors.join("sdkjs/common/Native"));
    let _ = std::fs::create_dir_all(&work_editors.join("sdkjs/common/libfont/engine"));
    let _ = std::fs::create_dir_all(&work_editors.join("sdkjs/word"));
    let _ = std::fs::create_dir_all(&work_editors.join("web-apps/vendor/xregexp"));
    let _ = std::fs::create_dir_all(&work_dictionaries);
    log_pdf(&format!("work_dir: {:?}", work_dir));

    // Symlink x2t binary
    let work_x2t = work_binaries.join("x2t");
    if !work_x2t.exists() {
        #[cfg(target_os = "linux")]
        { let _ = std::os::unix::fs::symlink(&x2t_exe, &work_x2t); }
        #[cfg(not(target_os = "linux"))]
        { let _ = std::fs::copy(&x2t_exe, &work_x2t); }
    }
    log_file_access(&work_x2t, "work x2t");

    // Symlink all .so libs from binaries/
    if let Ok(entries) = std::fs::read_dir(binaries_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".so") || name.contains(".so.") || name.ends_with(".dat") || name == "package.config" {
                let dst = work_binaries.join(&name);
                if !dst.exists() {
                    #[cfg(target_os = "linux")]
                    { let _ = std::os::unix::fs::symlink(entry.path(), &dst); }
                    #[cfg(not(target_os = "linux"))]
                    { let _ = std::fs::copy(entry.path(), &dst); }
                }
            }
        }
    }

    // Symlink fonts/ directory
    let work_fonts = work_binaries.join("fonts");
    if !work_fonts.exists() {
        #[cfg(target_os = "linux")]
        { let _ = std::os::unix::fs::symlink(&fonts_dir, &work_fonts); }
        #[cfg(not(target_os = "linux"))]
        { /* on Windows the original binaries_dir is used */ }
    }
    log_file_access(&work_fonts.join("Carlito-Regular.ttf"), "work fonts/Carlito");

    // Copy font_selection.bin to work binaries
    if fontsel_src.exists() {
        let dst = work_binaries.join("font_selection.bin");
        match std::fs::copy(&fontsel_src, &dst) {
            Ok(b) => log_pdf(&format!("Copied font_selection.bin to work dir: {} bytes", b)),
            Err(e) => log_pdf(&format!("FAILED copy font_selection.bin: {}", e)),
        }
        log_file_access(&dst, "work font_selection.bin");
    }

    // Copy server AllFonts.js to work binaries AND to work editors/
    let work_allfonts = work_binaries.join("AllFonts.js");
    match std::fs::copy(&allfonts_js, &work_allfonts) {
        Ok(b) => log_pdf(&format!("Copied AllFonts.js (server) to work binaries: {} bytes", b)),
        Err(e) => log_pdf(&format!("FAILED copy AllFonts.js to work binaries: {}", e)),
    }
    let work_editors_allfonts = work_editors.join("sdkjs/common/AllFonts.js");
    match std::fs::copy(&allfonts_js, &work_editors_allfonts) {
        Ok(b) => log_pdf(&format!("Copied AllFonts.js (server) to work editors: {} bytes", b)),
        Err(e) => log_pdf(&format!("FAILED copy AllFonts.js to work editors: {}", e)),
    }

    // Write DoctRenderer.config with correct paths
    let work_config = work_binaries.join("DoctRenderer.config");
    let config_content = r#"<Settings>
<file>../editors/sdkjs/common/Native/native.js</file>
<file>../editors/sdkjs/common/Native/jquery_native.js</file>
<allfonts>../editors/sdkjs/common/AllFonts.js</allfonts>
<file>../editors/web-apps/vendor/xregexp/xregexp-all-min.js</file>
<sdkjs>../editors/sdkjs</sdkjs>
<dictionaries>../dictionaries</dictionaries>
</Settings>"#;
    match std::fs::write(&work_config, config_content) {
        Ok(()) => log_pdf("Wrote DoctRenderer.config to work dir"),
        Err(e) => log_pdf(&format!("FAILED write DoctRenderer.config: {}", e)),
    }

    // Symlink/copy JS files to work editors/
    let editor_file_mappings = [
        (editors_dir.join("sdkjs/common/Native/native.js"), work_editors.join("sdkjs/common/Native/native.js")),
        (editors_dir.join("sdkjs/common/Native/jquery_native.js"), work_editors.join("sdkjs/common/Native/jquery_native.js")),
        (editors_dir.join("sdkjs/common/libfont/engine/fonts_native.js"), work_editors.join("sdkjs/common/libfont/engine/fonts_native.js")),
        (editors_dir.join("sdkjs/word/sdk-all-min.js"), work_editors.join("sdkjs/word/sdk-all-min.js")),
        (editors_dir.join("web-apps/vendor/xregexp/xregexp-all-min.js"), work_editors.join("web-apps/vendor/xregexp/xregexp-all-min.js")),
    ];
    for (src, dst) in &editor_file_mappings {
        if src.exists() && !dst.exists() {
            #[cfg(target_os = "linux")]
            { let _ = std::os::unix::fs::symlink(src, dst); }
            #[cfg(not(target_os = "linux"))]
            { let _ = std::fs::copy(src, dst); }
        }
        let ok = dst.exists();
        let size = std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        let rel = dst.strip_prefix(&work_dir).unwrap_or(dst);
        log_pdf(&format!("  work {} — exists={}, size={}", rel.display(), ok, size));
    }

    // Log full work dir structure
    log_pdf("=== Work directory structure ===");
    log_dir_recursive(&work_dir, &work_dir);

    log_pdf("--- dictionaries/ ---");
    log_pdf(&format!("work dictionaries/ exists: {}", work_dictionaries.exists()));

    // --- STEP 8: SDK bundle ---
    log_pdf("--- STEP 8: SDK bundle ---");
    for name in &["sdk-all-min.js", "sdk-word-bundle.js"] {
        let p = binaries_dir.join(name);
        if p.exists() {
            log_file_access(&p, name);
        }
    }
    let sdk_in_editors = editors_dir.join("sdkjs").join("word").join("sdk-all-min.js");
    if sdk_in_editors.exists() {
        log_file_access(&sdk_in_editors, "editors/sdkjs/word/sdk-all-min.js");
    }

    // --- STEP 9: Build XML params ---
    log_pdf("--- STEP 9: XML params ---");
    let params_xml = std::path::PathBuf::from(output)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("x2t_params.xml");

    let fonts_dir_abs = std::fs::canonicalize(&fonts_dir)
        .unwrap_or_else(|_| fonts_dir.clone());
    let allfonts_abs = std::fs::canonicalize(&work_allfonts)
        .unwrap_or_else(|_| work_allfonts.clone());

    let xml = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<TaskQueueDataConvert xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
                      xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<m_sFileFrom>{}</m_sFileFrom>
<m_sFileTo>{}</m_sFileTo>
<m_nFormatTo>513</m_nFormatTo>
<m_bEmbeddedFonts>true</m_bEmbeddedFonts>
<m_sFontDir>{}</m_sFontDir>
<m_sAllFontsPath>{}</m_sAllFontsPath>
</TaskQueueDataConvert>"#,
        input,
        output,
        fonts_dir_abs.to_string_lossy().replace('\\', "/"),
        allfonts_abs.to_string_lossy().replace('\\', "/"),
    );

    log_pdf(&format!("{}", xml));
    std::fs::write(&params_xml, &xml).map_err(|e| e.to_string())?;
    log_pdf(&format!("Wrote params XML to {:?}", params_xml));

    // --- STEP 10: Verify all critical files one last time before spawn ---
    log_pdf("--- STEP 10: Pre-spawn verification ---");
    let critical_files: Vec<(&str, std::path::PathBuf)> = vec![
        ("x2t", work_x2t.clone()),
        ("Editor.bin", std::path::PathBuf::from(input)),
        ("DoctRenderer.config", work_config.clone()),
        ("AllFonts.js (XML param)", allfonts_abs.clone()),
        ("AllFonts.js (editors)", work_editors_allfonts.clone()),
        ("font_selection.bin", work_binaries.join("font_selection.bin")),
        ("sdk-all-min.js", work_editors.join("sdkjs/word/sdk-all-min.js")),
        ("params.xml", params_xml.clone()),
    ];
    for (label, path) in &critical_files {
        let ok = std::fs::File::open(path).is_ok();
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        log_pdf(&format!("  [{}] {} — readable={}, size={}",
            if ok { "OK" } else { "FAIL" }, label, ok, size));
    }

    // --- STEP 11: Spawn x2t from writable work directory ---
    log_pdf("--- STEP 11: Spawning x2t ---");
    log_pdf(&format!("cwd: {:?}", work_binaries));
    log_pdf(&format!("LD_LIBRARY_PATH: {:?}", binaries_dir));
    let mut cmd = std::process::Command::new(&work_x2t);
    cmd.current_dir(&work_binaries)
        .arg(params_xml.to_string_lossy().as_ref());
    #[cfg(target_os = "linux")]
    cmd.env("LD_LIBRARY_PATH", binaries_dir);

    let result = cmd.output()
        .map_err(|e| format!("Failed to spawn x2t: {}", e))?;

    // --- STEP 12: Result ---
    log_pdf("--- STEP 12: x2t result ---");
    let code = result.status.code().unwrap_or(-999);
    log_pdf(&format!("exit code: {} (None/-999 means killed by signal)", code));
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = result.status.signal() {
            log_pdf(&format!("killed by signal: {} (11=SIGSEGV, 6=SIGABRT, 9=SIGKILL)", sig));
        }
    }
    if !result.stdout.is_empty() {
        log_pdf(&format!("stdout ({} bytes): {}", result.stdout.len(),
            String::from_utf8_lossy(&result.stdout)));
    }
    if !result.stderr.is_empty() {
        log_pdf(&format!("stderr ({} bytes): {}", result.stderr.len(),
            String::from_utf8_lossy(&result.stderr)));
    }

    let pdf_exists = std::path::Path::new(output).exists();
    let pdf_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    log_pdf(&format!("Output PDF: exists={}, size={} bytes", pdf_exists, pdf_size));
    log_pdf("=== convert_to_pdf END ===");

    if result.status.success() {
        Ok("ok".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!("x2t conversion failed (exit code {}): {}", code, stderr))
    }
}

fn find_x2t_exe(binaries_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let mut search_dirs = vec![
        binaries_dir.to_path_buf(),
        binaries_dir.parent().unwrap_or(binaries_dir).to_path_buf(),
    ];
    // Tauri places sidecars in Contents/MacOS/ on macOS
    if let Some(resources) = binaries_dir.parent() {
        if let Some(contents) = resources.parent() {
            let macos_dir = contents.join("MacOS");
            if macos_dir.exists() {
                search_dirs.push(macos_dir);
            }
        }
    }
    // Also check next to the current executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            search_dirs.push(exe_dir.to_path_buf());
        }
    }
    for dir in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if is_x2t_binary(&name) {
                    return Ok(entry.path());
                }
            }
        }
    }
    Err("x2t executable not found".to_string())
}

pub fn is_x2t_binary(name: &str) -> bool {
    if cfg!(target_os = "windows") {
        (name.starts_with("x2t-") || name == "x2t.exe") && name.ends_with(".exe")
    } else {
        (name.starts_with("x2t-") && !name.contains('.')) || name == "x2t"
    }
}
