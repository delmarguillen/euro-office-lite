// Where an installed app keeps its executable, resources and sidecar, worked
// out from the install root with Tauri's own rules instead of searching, so a
// badly packaged installer fails here rather than passing by luck.

use std::path::{Path, PathBuf};

// tauri.conf.json: productName names the Linux resource folder,
// mainBinaryName the executable.
pub const PRODUCT_NAME: &str = "Euro-Office-Lite";
pub const MAIN_BINARY: &str = "euro-office-lite";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    Macos,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }

    fn exe_suffix(self) -> &'static str {
        match self {
            Os::Windows => ".exe",
            _ => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub exe: PathBuf,
    // The executable's folder as given, not canonicalized: what the app gets
    // from current_exe() and hands to find_x2t.
    pub exe_dir_plain: PathBuf,
    pub resource_dir: PathBuf,
    pub binaries_dir: PathBuf,
    pub sidecar: PathBuf,
}

// The executable an extracted installer must have. A macOS root must hold
// exactly one .app.
pub fn expected_exe(os: Os, root: &Path) -> Result<PathBuf, String> {
    let exe_name = format!("{}{}", MAIN_BINARY, os.exe_suffix());
    let exe = match os {
        Os::Linux => root.join("usr").join("bin").join(&exe_name),
        Os::Windows => root.join(&exe_name),
        Os::Macos => {
            let entries = std::fs::read_dir(root)
                .map_err(|e| format!("cannot read install root {}: {}", root.display(), e))?;
            let mut apps: Vec<PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir() && path.extension().is_some_and(|ext| ext == "app"))
                .collect();
            if apps.len() != 1 {
                return Err(format!(
                    "expected exactly one .app in {}, found {}",
                    root.display(),
                    apps.len()
                ));
            }
            apps.remove(0).join("Contents").join("MacOS").join(&exe_name)
        }
    };
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(format!("executable not found: {}", exe.display()))
    }
}

// Tauri canonicalizes the running executable and derives resource_dir from
// that: next to it on Windows, ../Resources on macOS, and ../lib/<product>
// for an extracted .deb.
pub fn layout_from_exe(os: Os, exe: &Path) -> Result<Layout, String> {
    let exe = std::path::absolute(exe)
        .map_err(|e| format!("cannot make {} absolute: {}", exe.display(), e))?;
    let exe_dir_plain = parent(&exe)?;
    let canonical_exe = std::fs::canonicalize(&exe)
        .map_err(|e| format!("executable not found: {}: {}", exe.display(), e))?;
    let exe_dir = parent(&canonical_exe)?;
    let resource_dir = match os {
        Os::Windows => exe_dir.clone(),
        Os::Macos => canonical(&exe_dir.join("..").join("Resources"))?,
        Os::Linux => canonical(&exe_dir.join("..").join("lib").join(PRODUCT_NAME))?,
    };
    let binaries_dir = resource_dir.join("binaries");
    if !binaries_dir.is_dir() {
        return Err(format!("binaries folder not found: {}", binaries_dir.display()));
    }
    let sidecar = exe_dir.join(format!("x2t{}", os.exe_suffix()));
    if !sidecar.is_file() {
        return Err(format!("x2t sidecar not found: {}", sidecar.display()));
    }
    Ok(Layout {
        exe,
        exe_dir_plain,
        resource_dir,
        binaries_dir,
        sidecar,
    })
}

fn parent(path: &Path) -> Result<PathBuf, String> {
    path.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no parent folder", path.display()))
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map_err(|e| format!("resource folder not found: {}: {}", path.display(), e))
}

// Folder and file names with &, < and > for the special-name variant: the
// params XML must escape them (x2t exits 88 otherwise). Windows forbids < and
// > in names, so it gets the & alone.
pub fn special_names(os: Os, ext: &str) -> (String, String) {
    match os {
        Os::Windows => ("R&D".to_string(), format!("A&B.{}", ext)),
        _ => ("R&D <x>".to_string(), format!("A&B <y>.{}", ext)),
    }
}

// The folder the special-temp variant puts its temp dir under: what a user
// account named A&B gives on Windows.
pub fn special_temp_dir(os: Os) -> &'static str {
    match os {
        Os::Windows => "T&mp",
        _ => "T&mp <x>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("corpus-layout-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    fn deb_tree(root: &Path) {
        touch(&root.join("usr/bin/euro-office-lite"));
        touch(&root.join("usr/bin/x2t"));
        touch(&root.join("usr/lib/Euro-Office-Lite/binaries/x2t"));
    }

    #[test]
    fn the_names_match_tauri_conf() {
        let conf = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/tauri.conf.json"),
        )
        .unwrap();
        assert!(conf.contains(&format!("\"productName\": \"{}\"", PRODUCT_NAME)));
        assert!(conf.contains(&format!("\"mainBinaryName\": \"{}\"", MAIN_BINARY)));
    }

    #[test]
    fn an_extracted_deb_resolves_to_usr_lib() {
        let root = scratch("deb");
        deb_tree(&root);

        let exe = expected_exe(Os::Linux, &root).unwrap();
        assert_eq!(exe, root.join("usr/bin/euro-office-lite"));
        let layout = layout_from_exe(Os::Linux, &exe).unwrap();

        let lib = std::fs::canonicalize(root.join("usr/lib/Euro-Office-Lite")).unwrap();
        let bin = std::fs::canonicalize(root.join("usr/bin")).unwrap();
        assert_eq!(layout.exe_dir_plain, root.join("usr/bin"));
        assert_eq!(layout.resource_dir, lib);
        assert_eq!(layout.binaries_dir, lib.join("binaries"));
        assert_eq!(layout.sidecar, bin.join("x2t"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mac_app_resolves_to_contents_resources() {
        let root = scratch("app");
        let contents = root.join("Euro-Office-Lite.app/Contents");
        touch(&contents.join("MacOS/euro-office-lite"));
        touch(&contents.join("MacOS/x2t"));
        touch(&contents.join("Resources/binaries/x2t"));

        let exe = expected_exe(Os::Macos, &root).unwrap();
        let layout = layout_from_exe(Os::Macos, &exe).unwrap();

        let resources = std::fs::canonicalize(contents.join("Resources")).unwrap();
        let macos = std::fs::canonicalize(contents.join("MacOS")).unwrap();
        assert_eq!(layout.resource_dir, resources);
        assert_eq!(layout.binaries_dir, resources.join("binaries"));
        assert_eq!(layout.sidecar, macos.join("x2t"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mac_root_with_two_apps_is_refused() {
        let root = scratch("two-apps");
        std::fs::create_dir_all(root.join("A.app")).unwrap();
        std::fs::create_dir_all(root.join("B.app")).unwrap();

        let error = expected_exe(Os::Macos, &root).unwrap_err();
        assert!(error.contains("exactly one .app"), "{}", error);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_windows_install_keeps_its_resources_next_to_the_exe() {
        let root = scratch("nsis");
        touch(&root.join("euro-office-lite.exe"));
        touch(&root.join("x2t.exe"));
        std::fs::create_dir_all(root.join("binaries")).unwrap();

        let exe = expected_exe(Os::Windows, &root).unwrap();
        let layout = layout_from_exe(Os::Windows, &exe).unwrap();

        let dir = std::fs::canonicalize(&root).unwrap();
        assert_eq!(layout.resource_dir, dir);
        assert_eq!(layout.binaries_dir, dir.join("binaries"));
        assert_eq!(layout.sidecar, dir.join("x2t.exe"));
        let _ = std::fs::remove_dir_all(&root);
    }

    // What 7-Zip extracts from the published NSIS installer, x64 and arm64
    // alike: $INSTDIR at the root, plus the installer's own $PLUGINSDIR.
    const NSIS_FILES: [&str; 36] = [
        "euro-office-lite.exe",
        "x2t.exe",
        "binaries/AllFonts.js",
        "binaries/DjVuFile.dll",
        "binaries/DoctRenderer.config",
        "binaries/DocxRenderer.dll",
        "binaries/EpubFile.dll",
        "binaries/Fb2File.dll",
        "binaries/HWPFile.dll",
        "binaries/HtmlFile2.dll",
        "binaries/IWorkFile.dll",
        "binaries/OFDFile.dll",
        "binaries/PdfFile.dll",
        "binaries/StarMathConverter.dll",
        "binaries/UnicodeConverter.dll",
        "binaries/XpsFile.dll",
        "binaries/doctrenderer.dll",
        "binaries/graphics.dll",
        "binaries/icudt74.dll",
        "binaries/icudtl.dat",
        "binaries/icudtl_extra.dat",
        "binaries/icuuc74.dll",
        "binaries/kernel.dll",
        "binaries/kernel_network.dll",
        "binaries/native_shim.js",
        "binaries/ooxmlsignature.dll",
        "binaries/package.config",
        "binaries/sdk-all-min.js",
        "binaries/sdk-all.js",
        "binaries/sdk-word-bundle.js",
        "binaries/xregexp-all-min.js",
        "editors/sdkjs/common/AllFonts.js",
        "editors/web-apps/vendor/xregexp/xregexp-all-min.js",
        "templates/blank.docx",
        "templates/blank.pptx",
        "templates/blank.xlsx",
    ];
    const NSIS_PLUGINS: [&str; 7] = [
        "LangDLL.dll",
        "NSISdl.dll",
        "StartMenu.dll",
        "System.dll",
        "modern-wizard.bmp",
        "nsDialogs.dll",
        "nsis_tauri_utils.dll",
    ];

    fn nsis_tree(root: &Path, with_plugins: bool) {
        for file in NSIS_FILES {
            touch(&root.join(file));
        }
        // The 16 bundled faces, loose in binaries/ and again in two folders.
        for family in ["Carlito", "LiberationMono", "LiberationSans", "LiberationSerif"] {
            for style in ["Regular", "Bold", "Italic", "BoldItalic"] {
                for dir in ["binaries", "binaries/fonts", "binaries/core-fonts"] {
                    touch(&root.join(dir).join(format!("{}-{}.ttf", family, style)));
                }
            }
        }
        for editor in ["word", "cell", "slide"] {
            for bundle in ["sdk-all.js", "sdk-all-min.js"] {
                touch(&root.join("editors/sdkjs").join(editor).join(bundle));
            }
        }
        for file in ["Native/jquery_native.js", "Native/native.js", "libfont/engine/fonts_native.js"] {
            touch(&root.join("editors/sdkjs/common").join(file));
        }
        if with_plugins {
            for file in NSIS_PLUGINS {
                touch(&root.join("$PLUGINSDIR").join(file));
            }
        }
    }

    // binaries/ has no x2t, so the PDF export finds the sidecar itself one
    // folder up, as in the installed app; leftover installer plugins change
    // nothing.
    #[test]
    fn the_tree_nsis_extracts_resolves_like_an_install() {
        for with_plugins in [false, true] {
            let root = scratch(if with_plugins { "nsis-plugins" } else { "nsis-real" });
            nsis_tree(&root, with_plugins);

            let exe = expected_exe(Os::Windows, &root).unwrap();
            assert_eq!(exe, root.join("euro-office-lite.exe"));
            let layout = layout_from_exe(Os::Windows, &exe).unwrap();
            let dir = std::fs::canonicalize(&root).unwrap();
            assert_eq!(layout.exe_dir_plain, root);
            assert_eq!(layout.binaries_dir, dir.join("binaries"));
            assert_eq!(layout.sidecar, dir.join("x2t.exe"));
            if cfg!(windows) {
                let pdf_x2t = crate::x2t_task::find_x2t(&layout.binaries_dir, Some(&layout.exe_dir_plain)).unwrap();
                assert_eq!(std::fs::canonicalize(pdf_x2t).unwrap(), std::fs::canonicalize(&layout.sidecar).unwrap());
            }
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn missing_pieces_give_a_clear_error() {
        let root = scratch("missing");
        let error = expected_exe(Os::Linux, &root).unwrap_err();
        assert!(error.starts_with("executable not found"), "{}", error);

        touch(&root.join("usr/bin/euro-office-lite"));
        let exe = root.join("usr/bin/euro-office-lite");
        let error = layout_from_exe(Os::Linux, &exe).unwrap_err();
        assert!(error.starts_with("resource folder not found"), "{}", error);

        std::fs::create_dir_all(root.join("usr/lib/Euro-Office-Lite")).unwrap();
        let error = layout_from_exe(Os::Linux, &exe).unwrap_err();
        assert!(error.starts_with("binaries folder not found"), "{}", error);

        std::fs::create_dir_all(root.join("usr/lib/Euro-Office-Lite/binaries")).unwrap();
        let error = layout_from_exe(Os::Linux, &exe).unwrap_err();
        assert!(error.starts_with("x2t sidecar not found"), "{}", error);

        touch(&root.join("usr/bin/x2t"));
        assert!(layout_from_exe(Os::Linux, &exe).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn special_names_carry_the_markup_characters_each_platform_allows() {
        for os in [Os::Linux, Os::Macos, Os::Windows] {
            let (dir, file) = special_names(os, "docx");
            let temp = special_temp_dir(os);
            for name in [dir.as_str(), file.as_str(), temp] {
                assert!(name.contains('&'), "{:?} {}", os, name);
                assert_eq!(os == Os::Windows, !name.contains('<') && !name.contains('>'), "{:?} {}", os, name);
            }
            assert!(file.ends_with(".docx"));
        }
    }
}
