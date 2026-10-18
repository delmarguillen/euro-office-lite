// What the app does to run x2t that needs no Tauri. The file uses only std and
// never names crate:: or tauri, so another crate can include it with #[path]
// and run x2t with exactly the app's code. Log lines go out through a `log`
// parameter, without the prefix the app adds.

// On Windows, resource_dir() and canonicalize() return \\?\C:\... paths. x2t
// copies whatever spelling it is given into its font cache, and its PDF export
// cannot open fonts listed under that prefix, so paths handed to x2t drop it.
// Only drive and UNC forms have a plain spelling (C:\..., \\server\...); any
// other form, such as \\?\Volume{GUID}\..., is kept as it is.
pub fn strip_extended_prefix(p: &std::path::Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    let Some(rest) = s.strip_prefix(r"\\?\") else {
        return p.to_path_buf();
    };
    let bytes = rest.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        std::path::PathBuf::from(rest)
    } else if let Some(share) = rest
        .get(..4)
        .filter(|head| head.eq_ignore_ascii_case(r"UNC\"))
        .map(|_| &rest[4..])
    {
        std::path::PathBuf::from(format!(r"\\{}", share))
    } else {
        p.to_path_buf()
    }
}

pub fn is_x2t_binary(name: &str) -> bool {
    if cfg!(target_os = "windows") {
        (name.starts_with("x2t-") || name == "x2t.exe") && name.ends_with(".exe")
    } else {
        (name.starts_with("x2t-") && !name.contains('.')) || name == "x2t"
    }
}

// The x2t the PDF export and the font scan run: the first match in binaries/,
// then its parent, then Contents/MacOS, then `exe_dir` (the folder of the
// app's executable). On Linux and macOS binaries/ holds its own copy of x2t,
// so these two never run the sidecar that the other conversions use.
pub fn find_x2t(
    binaries_dir: &std::path::Path,
    exe_dir: Option<&std::path::Path>,
) -> Result<std::path::PathBuf, String> {
    let mut search_dirs = vec![
        binaries_dir.to_path_buf(),
        binaries_dir.parent().unwrap_or(binaries_dir).to_path_buf(),
    ];
    if let Some(resources) = binaries_dir.parent() {
        if let Some(contents) = resources.parent() {
            let macos_dir = contents.join("MacOS");
            if macos_dir.exists() {
                search_dirs.push(macos_dir);
            }
        }
    }
    if let Some(exe_dir) = exe_dir {
        search_dirs.push(exe_dir.to_path_buf());
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

/// Where to write the params file x2t is invoked with. Every path inside the
/// XML is absolute, so the file does not need to sit next to the output; the
/// print path has always written it into temp_dir and works on all three
/// platforms. Keeping it there is what stops a PDF export from leaving
/// `x2t_params.xml` behind in the user's folder (#35), which the cleanup after
/// the run cannot guarantee on its own (a crash or a kill skips it). The name
/// is unique per call so two conversions in the same session cannot clobber
/// each other's params.
pub fn params_path(temp_dir: &std::path::Path, tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    temp_dir.join(format!(
        "x2t_params_{}_{}_{}.xml",
        tag,
        std::process::id(),
        unique
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "eo-x2t-task-{}-{}-{}",
            name,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    // The name is_x2t_binary accepts for an installed x2t on this platform.
    fn x2t_name() -> &'static str {
        if cfg!(target_os = "windows") {
            "x2t.exe"
        } else {
            "x2t"
        }
    }

    // The #35 regression: convert_to_pdf used to drop the params file next to
    // the PDF it was writing, so every export left an x2t_params.xml in the
    // user's folder. The params file belongs in the app temp directory.
    #[test]
    fn params_stay_out_of_the_output_folder() {
        let temp_dir = std::path::Path::new("/tmp/eo-app");
        let output_dir = std::path::Path::new("/home/user/Documents");

        for tag in ["pdf", "convert"] {
            let path = params_path(temp_dir, tag);

            assert_eq!(
                path.parent(),
                Some(temp_dir),
                "params must be written to temp_dir, never beside the output (#35)"
            );
            assert_ne!(
                path.parent(),
                Some(output_dir),
                "params must never land in the folder the user picked (#35)"
            );
        }
    }

    // Two exports in the same session must not share a params file: the second
    // would overwrite the first while x2t is still reading it.
    #[test]
    fn params_path_is_unique_per_call() {
        let temp_dir = std::path::Path::new("/tmp/eo-app");

        let first = params_path(temp_dir, "pdf");
        let second = params_path(temp_dir, "pdf");

        assert_ne!(first, second, "each conversion needs its own params file");
    }

    // Plain strings, so it runs anywhere: only the spelling is under test.
    #[test]
    fn the_extended_prefix_is_dropped_only_where_a_plain_form_exists() {
        use std::path::{Path, PathBuf};
        let cases = [
            (r"\\?\C:\App\binaries", r"C:\App\binaries"),
            (
                r"\\?\UNC\server\share\App\binaries",
                r"\\server\share\App\binaries",
            ),
            (
                r"\\?\Volume{0b1c2d3e-0000-0000-0000-000000000000}\App\binaries",
                r"\\?\Volume{0b1c2d3e-0000-0000-0000-000000000000}\App\binaries",
            ),
            (r"C:\App\binaries", r"C:\App\binaries"),
        ];
        let wrong: Vec<(&str, PathBuf)> = cases
            .iter()
            .map(|(given, expected)| (*given, *expected, strip_extended_prefix(Path::new(given))))
            .filter(|(_, expected, got)| got != &PathBuf::from(expected))
            .map(|(given, _, got)| (given, got))
            .collect();
        assert!(wrong.is_empty(), "wrong spellings: {:?}", wrong);
    }

    #[test]
    fn x2t_names_and_decoys() {
        let windows = cfg!(target_os = "windows");
        assert_eq!(is_x2t_binary("x2t.exe"), windows);
        assert_eq!(is_x2t_binary("x2t-x86_64-pc-windows-msvc.exe"), windows);
        assert_eq!(is_x2t_binary("x2t"), !windows);
        assert_eq!(is_x2t_binary("x2t-x86_64-unknown-linux-gnu"), !windows);
        for decoy in ["x2t.dll", "x2t.pdb", "x2t.exe.config", "libx2t.so", "x2tool"] {
            assert!(!is_x2t_binary(decoy), "{} is not x2t", decoy);
        }
    }

    // .deb: the PDF export and the font scan run binaries/x2t, never the
    // sidecar in usr/bin next to the app.
    #[test]
    fn linux_layout_finds_the_copy_in_binaries() {
        let root = scratch_dir("deb");
        let binaries = root.join("usr/lib/Euro-Office-Lite/binaries");
        let exe_dir = root.join("usr/bin");
        touch(&binaries.join(x2t_name()));
        touch(&binaries.join("libkernel.so"));
        touch(&exe_dir.join(x2t_name()));
        touch(&exe_dir.join("euro-office-lite"));

        assert_eq!(
            find_x2t(&binaries, Some(&exe_dir)),
            Ok(binaries.join(x2t_name()))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // .app: Contents/Resources/binaries/x2t wins over the sidecar in
    // Contents/MacOS.
    #[test]
    fn macos_layout_finds_the_copy_in_binaries() {
        let root = scratch_dir("app");
        let contents = root.join("Euro-Office-Lite.app/Contents");
        let binaries = contents.join("Resources/binaries");
        let macos = contents.join("MacOS");
        touch(&binaries.join(x2t_name()));
        touch(&macos.join(x2t_name()));
        touch(&macos.join("euro-office-lite"));

        assert_eq!(
            find_x2t(&binaries, Some(&macos)),
            Ok(binaries.join(x2t_name()))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // NSIS: binaries/ holds no x2t, so the one in the install folder is used,
    // the same file the sidecar runs.
    #[test]
    fn windows_layout_finds_the_one_in_the_install_folder() {
        let root = scratch_dir("nsis");
        let binaries = root.join("binaries");
        touch(&binaries.join("kernel.dll"));
        touch(&binaries.join("x2t.dll"));
        touch(&binaries.join("x2t.pdb"));
        touch(&root.join(x2t_name()));
        touch(&root.join("euro-office-lite.exe"));

        assert_eq!(find_x2t(&binaries, Some(&root)), Ok(root.join(x2t_name())));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_search_order_is_binaries_parent_macos_then_exe_dir() {
        let root = scratch_dir("order");
        let contents = root.join("Contents");
        let binaries = contents.join("Resources/binaries");
        let parent = contents.join("Resources");
        let macos = contents.join("MacOS");
        let exe_dir = root.join("exe");
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::create_dir_all(&macos).unwrap();
        touch(&exe_dir.join(x2t_name()));

        assert_eq!(find_x2t(&binaries, Some(&exe_dir)), Ok(exe_dir.join(x2t_name())));
        touch(&macos.join(x2t_name()));
        assert_eq!(find_x2t(&binaries, Some(&exe_dir)), Ok(macos.join(x2t_name())));
        touch(&parent.join(x2t_name()));
        assert_eq!(find_x2t(&binaries, Some(&exe_dir)), Ok(parent.join(x2t_name())));
        touch(&binaries.join(x2t_name()));
        assert_eq!(find_x2t(&binaries, Some(&exe_dir)), Ok(binaries.join(x2t_name())));
        let _ = std::fs::remove_dir_all(&root);
    }

    // A dev build names the sidecar after the target triple.
    #[test]
    fn a_target_triple_name_is_found() {
        let root = scratch_dir("triple");
        let binaries = root.join("binaries");
        let name = if cfg!(target_os = "windows") {
            "x2t-x86_64-pc-windows-msvc.exe"
        } else {
            "x2t-x86_64-unknown-linux-gnu"
        };
        touch(&binaries.join(name));

        assert_eq!(find_x2t(&binaries, None), Ok(binaries.join(name)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_x2t_the_error_is_the_one_the_logs_show() {
        let root = scratch_dir("none");
        let binaries = root.join("binaries");
        touch(&binaries.join("x2t.dll"));
        touch(&binaries.join("x2t.pdb"));

        assert_eq!(
            find_x2t(&binaries, Some(&root.join("missing"))),
            Err("x2t executable not found".to_string())
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
