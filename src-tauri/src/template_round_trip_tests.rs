// Saves a document as a template with a real x2t, opens the template and saves
// it as a document again. Ignored by default: it needs x2t and its libraries in
// EO_X2T_BINARIES (the folder the app ships as binaries/), as get-x2t-ci.sh
// unpacks them.
// Run: EO_X2T_BINARIES=/path/binaries cargo test -- --ignored real_x2t
//
// Its own module, not inside x2t_task.rs: tools/corpus builds that file too,
// and has no zip crate.
use crate::x2t_task::{
    convert_env, convert_params_xml, detect_format, is_x2t_binary, params_path, write_file,
};
use std::io::Read;
use std::path::{Path, PathBuf};

const EDITOR_BIN: i32 = 8192;

struct Family {
    // A document of tests/corpus.
    source: &'static str,
    main_part: &'static str,
    template_ext: &'static str,
    template_type: &'static str,
    document_ext: &'static str,
    document_type: &'static str,
}

const FAMILIES: [Family; 3] = [
    Family {
        source: "rich.docx",
        main_part: "/word/document.xml",
        template_ext: "dotx",
        template_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
        document_ext: "docx",
        document_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    },
    Family {
        source: "sheet.xlsx",
        main_part: "/xl/workbook.xml",
        template_ext: "xltx",
        template_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
        document_ext: "xlsx",
        document_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
    },
    Family {
        source: "slides.pptx",
        main_part: "/ppt/presentation.xml",
        template_ext: "potx",
        template_type: "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml",
        document_ext: "pptx",
        document_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
    },
];

fn corpus_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus").join(name)
}

fn is_x2t(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    is_x2t_binary(&name)
}

pub(crate) fn x2t_in(binaries_dir: &Path) -> PathBuf {
    let entries = std::fs::read_dir(binaries_dir).expect("EO_X2T_BINARIES is not a folder");
    for entry in entries.flatten() {
        let path = entry.path();
        if is_x2t(&path) {
            return path;
        }
    }
    panic!("no x2t in EO_X2T_BINARIES");
}

// One x2t run with the parameters and the environment the app gives it.
fn convert(binaries_dir: &Path, from: &Path, to: &Path, format_to: i32, temp_dir: &Path) {
    let params = params_path(temp_dir, "convert");
    let xml = convert_params_xml(
        &from.to_string_lossy(),
        &to.to_string_lossy(),
        format_to,
        &temp_dir.to_string_lossy(),
        "",
    );
    write_file(&params, &xml).unwrap();
    let status = std::process::Command::new(x2t_in(binaries_dir))
        .current_dir(binaries_dir)
        .arg(&params)
        .envs(convert_env(binaries_dir))
        .status()
        .expect("x2t did not start");
    assert!(status.success(), "{} -> {}: {}", from.display(), to.display(), status);
}

// Converts `from` to Editor.bin in a temp dir of its own and returns both.
fn open(binaries_dir: &Path, from: &Path, temp_dir: PathBuf) -> (PathBuf, PathBuf) {
    std::fs::create_dir_all(&temp_dir).unwrap();
    let editor_bin = temp_dir.join("Editor.bin");
    convert(binaries_dir, from, &editor_bin, EDITOR_BIN, &temp_dir);
    (editor_bin, temp_dir)
}

fn read_part(package: &Path, name: &str) -> String {
    let file = std::fs::File::open(package).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut text = String::new();
    archive.by_name(name).unwrap().read_to_string(&mut text).unwrap();
    text
}

// The ContentType of the Override that names `main_part`.
fn main_content_type(package: &Path, main_part: &str) -> String {
    let types = read_part(package, "[Content_Types].xml");
    let part_name = format!("PartName=\"{main_part}\"");
    let element = types
        .split('<')
        .find(|element| element.contains(&part_name))
        .unwrap_or_else(|| panic!("no Override for {main_part}"));
    let value = element.split("ContentType=\"").nth(1).expect("no ContentType");
    value.split('"').next().unwrap_or_default().to_string()
}

#[test]
#[ignore]
fn real_x2t_writes_a_template_and_a_document_from_it() {
    let Ok(binaries) = std::env::var("EO_X2T_BINARIES") else {
        panic!("EO_X2T_BINARIES is not set");
    };
    let binaries_dir = PathBuf::from(binaries);
    let root = std::env::temp_dir().join(format!("eo-real-x2t-template-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    for family in &FAMILIES {
        let dir = root.join(family.template_ext);
        let (editor_bin, temp_dir) = open(&binaries_dir, &corpus_file(family.source), dir.join("open"));

        let template = dir.join(format!("saved.{}", family.template_ext));
        convert(&binaries_dir, &editor_bin, &template, detect_format(&template), &temp_dir);
        assert_eq!(main_content_type(&template, family.main_part), family.template_type);

        let (editor_bin, temp_dir) = open(&binaries_dir, &template, dir.join("reopen"));
        let document = dir.join(format!("new.{}", family.document_ext));
        convert(&binaries_dir, &editor_bin, &document, detect_format(&document), &temp_dir);
        assert_eq!(main_content_type(&document, family.main_part), family.document_type);

        let again = dir.join(format!("again.{}", family.template_ext));
        convert(&binaries_dir, &editor_bin, &again, detect_format(&again), &temp_dir);
        assert_eq!(main_content_type(&again, family.main_part), family.template_type);
        let main_part = family.main_part.trim_start_matches('/');
        assert_eq!(read_part(&again, main_part), read_part(&template, main_part), "{main_part}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
