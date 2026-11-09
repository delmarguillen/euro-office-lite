// The relationships an OOXML package declares in its .rels parts. The content
// of a document has to match its format, and a package holds its own parts:
// what it points to outside itself has to be a URL.

use std::io::{Read, Seek};
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

const ZIP_SIGNATURE: &[u8] = b"PK\x03\x04";
const ALLOWED_SCHEMES: [&str; 5] = ["http", "https", "mailto", "ftp", "file"];

// x2t 10.0.0.169 only follows image relationships: these two Types never read
// their target, and a hyperlink to another file is an ordinary thing to have.
const HYPERLINK_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/hyperlink",
];

struct Limits {
    entries: usize,
    rels_bytes: u64,
    total_rels_bytes: u64,
}

const LIMITS: Limits = Limits {
    entries: 20_000,
    rels_bytes: 16 * 1024 * 1024,
    total_rels_bytes: 64 * 1024 * 1024,
};

// Whether the file's relationships are all acceptable. A file that is not a ZIP
// has none, and a ZIP whose central directory cannot be opened is left to x2t.
pub fn relationships_are_allowed(path: &Path, head: &[u8]) -> bool {
    if !head.starts_with(ZIP_SIGNATURE) {
        return true;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return true;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return true;
    };
    all_rels_allowed(&mut archive, &LIMITS)
}

fn all_rels_allowed<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, limits: &Limits) -> bool {
    if archive.len() > limits.entries {
        return false;
    }
    let mut total = 0;
    for index in 0..archive.len() {
        let is_rels = archive
            .name_for_index(index)
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".rels"));
        if !is_rels {
            continue;
        }
        let Some(xml) = read_rels(archive, index, limits.rels_bytes) else {
            return false;
        };
        total += xml.len() as u64;
        if total > limits.total_rels_bytes || !rels_part_allowed(&xml) {
            return false;
        }
    }
    true
}

// The bytes of one .rels part, at most `limit` of them. None past the limit, or
// when the part cannot be read.
fn read_rels<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    index: usize,
    limit: u64,
) -> Option<Vec<u8>> {
    let entry = archive.by_index(index).ok()?;
    let mut xml = Vec::new();
    entry.take(limit + 1).read_to_end(&mut xml).ok()?;
    if xml.len() as u64 > limit {
        return None;
    }
    Some(xml)
}

// Only UTF-8 is read, and a part that cannot be analysed is not allowed.
fn rels_part_allowed(xml: &[u8]) -> bool {
    if xml.contains(&0) {
        return false;
    }
    let Ok(text) = std::str::from_utf8(xml) else {
        return false;
    };
    let mut reader = Reader::from_str(text.strip_prefix('\u{FEFF}').unwrap_or(text));
    let mut depth = 0usize;
    let mut saw_element = false;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return depth == 0 && saw_element,
            Ok(Event::Decl(declaration)) => {
                if !declares_utf8(&declaration) {
                    return false;
                }
            }
            Ok(Event::Start(element)) => {
                if !element_allowed(&element) {
                    return false;
                }
                depth += 1;
                saw_element = true;
            }
            Ok(Event::Empty(element)) => {
                if !element_allowed(&element) {
                    return false;
                }
                saw_element = true;
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Text(text)) => {
                if depth == 0 && !text.iter().all(u8::is_ascii_whitespace) {
                    return false;
                }
            }
            Ok(Event::Comment(_)) | Ok(Event::PI(_)) => {}
            Ok(_) => return false,
            Err(_) => return false,
        }
    }
}

fn declares_utf8(declaration: &quick_xml::events::BytesDecl) -> bool {
    match declaration.encoding() {
        None => true,
        Some(Ok(encoding)) => encoding.eq_ignore_ascii_case(b"utf-8"),
        Some(Err(_)) => false,
    }
}

fn element_allowed(element: &BytesStart) -> bool {
    element.local_name().as_ref() != b"Relationship" || relationship_allowed(element)
}

fn relationship_allowed(element: &BytesStart) -> bool {
    let mut targets = Vec::new();
    let mut modes = Vec::new();
    let mut types = Vec::new();
    for attribute in element.attributes() {
        let Ok(attribute) = attribute else {
            return false;
        };
        let Ok(value) = attribute.unescape_value() else {
            return false;
        };
        match attribute.key.local_name().as_ref() {
            b"Target" => targets.push(value.into_owned()),
            b"TargetMode" => modes.push(value.into_owned()),
            b"Type" => types.push(value.into_owned()),
            _ => {}
        }
    }
    if modes.iter().all(|mode| mode == "Internal") {
        return true;
    }
    if !types.is_empty() && types.iter().all(|kind| HYPERLINK_TYPES.contains(&kind.as_str())) {
        return true;
    }
    !targets.is_empty() && targets.iter().all(|target| has_allowed_scheme(target))
}

// The target, with its XML entities already decoded, is percent-decoded before
// its scheme is read. A Windows drive letter is not a scheme on the list.
fn has_allowed_scheme(target: &str) -> bool {
    let decoded = percent_decoded(target);
    let Some((scheme, _)) = decoded.split_once(':') else {
        return false;
    };
    ALLOWED_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str())
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match percent_byte(&bytes[index..]) {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// The byte a "%XX" at the start of `bytes` stands for.
fn percent_byte(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 3 || bytes[0] != b'%' {
        return None;
    }
    let high = (bytes[1] as char).to_digit(16)?;
    let low = (bytes[2] as char).to_digit(16)?;
    Some((high * 16 + low) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    const NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
    const ABS: &str = "/home/user/picture.png";

    fn package(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in parts {
            writer.start_file(*name, SimpleFileOptions::default()).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn allowed(bytes: Vec<u8>) -> bool {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        all_rels_allowed(&mut archive, &LIMITS)
    }

    fn rels_xml(body: &str) -> String {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"{NS}\">{body}</Relationships>")
    }

    fn external(kind: &str, target: &str) -> String {
        rels_xml(&format!(
            "<Relationship Id=\"rId1\" Type=\"{kind}\" Target=\"{target}\" TargetMode=\"External\"/>"
        ))
    }

    fn rels_allowed(xml: &str) -> bool {
        rels_part_allowed(xml.as_bytes())
    }

    fn image_target_allowed(target: &str) -> bool {
        rels_allowed(&external(IMAGE, target))
    }

    #[test]
    fn urls_with_a_listed_scheme_are_allowed() {
        for target in [
            "http://example.com/a.png",
            "https://example.com/a.png",
            "mailto:someone@example.com",
            "ftp://example.com/a.png",
            "file:///C:/images/a.png",
            "HTTP://EXAMPLE.COM/A.PNG",
            "File:///home/a.png",
        ] {
            assert!(image_target_allowed(target), "{target}");
        }
    }

    #[test]
    fn local_targets_are_not_allowed() {
        for target in [
            ABS,
            "C:\\images\\a.png",
            "C:/images/a.png",
            "\\\\server\\share\\a.png",
            "//server/share/a.png",
            "..\\images\\a.png",
            "../images/a.png",
            "picture.png",
            "",
        ] {
            assert!(!image_target_allowed(target), "{target}");
        }
    }

    #[test]
    fn a_relationship_without_a_target_mode_or_marked_internal_is_not_external() {
        let plain = format!("<Relationship Id=\"r\" Type=\"{IMAGE}\" Target=\"media/a.png\"/>");
        let internal =
            format!("<Relationship Id=\"r\" Type=\"{IMAGE}\" Target=\"media/a.png\" TargetMode=\"Internal\"/>");
        assert!(rels_allowed(&rels_xml(&plain)));
        assert!(rels_allowed(&rels_xml(&internal)));
    }

    #[test]
    fn a_target_mode_other_than_internal_needs_a_listed_scheme() {
        for mode in ["external", "EXTERNAL", "Exte&#114;nal", "anything", "internal"] {
            let body = format!("<Relationship Id=\"r\" Type=\"{IMAGE}\" Target=\"{ABS}\" TargetMode=\"{mode}\"/>");
            assert!(!rels_allowed(&rels_xml(&body)), "{mode}");
        }
    }

    #[test]
    fn an_external_relationship_without_a_target_is_not_allowed() {
        let body = format!("<Relationship Id=\"r\" Type=\"{IMAGE}\" TargetMode=\"External\"/>");
        assert!(!rels_allowed(&rels_xml(&body)));
    }

    #[test]
    fn xml_entities_are_decoded_before_the_scheme_is_read() {
        assert!(!image_target_allowed("&#47;home&#47;user&#47;a.png"));
        assert!(!image_target_allowed("&#x2F;home/a.png"));
        assert!(image_target_allowed("http://example.com/?a=1&amp;b=2"));
        assert!(!image_target_allowed("/a&amp;b.png"));
        assert!(image_target_allowed("&#102;ile:///home/a.png"));
    }

    #[test]
    fn percent_encoding_is_decoded_after_the_entities() {
        assert!(!image_target_allowed("%2Fhome%2Fa.png"));
        assert!(!image_target_allowed("&#37;2Fhome/a.png"));
        assert!(image_target_allowed("file%3A///home/a.png"));
        assert_eq!(percent_decoded("%2F%2f%zz%2"), "//%zz%2");
    }

    #[test]
    fn only_the_two_official_hyperlink_types_are_exempt() {
        for kind in HYPERLINK_TYPES {
            assert!(rels_allowed(&external(kind, ABS)), "{kind}");
        }
        for kind in [
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/Hyperlink",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlinks",
            "http://schemas.openxmlformats.org/officedocument/2006/relationships/hyperlink",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink ",
            "xhttp://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
            "hyperlink",
            "made-up",
            IMAGE,
        ] {
            assert!(!rels_allowed(&external(kind, ABS)), "{kind}");
        }
    }

    // The relationship is judged the same way however the picture refers to it.
    fn picture_package(picture: &str, kind: &str) -> Vec<u8> {
        let rels = external(kind, ABS);
        package(&[
            ("[Content_Types].xml", b"<Types/>"),
            ("word/document.xml", picture.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
        ])
    }

    #[test]
    fn a_picture_on_a_near_hyperlink_type_is_refused_by_link_and_by_embed() {
        let near = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/Hyperlink";
        assert!(!allowed(picture_package("<a:blip r:link=\"rId1\"/>", near)));
        assert!(!allowed(picture_package("<a:blip r:embed=\"rId1\"/>", near)));
        assert!(!allowed(picture_package("<a:blip r:link=\"rId1\"/>", IMAGE)));
        assert!(!allowed(picture_package("<a:blip r:embed=\"rId1\"/>", IMAGE)));
    }

    #[test]
    fn a_vml_picture_is_refused_like_any_other() {
        let vml = "<v:imagedata r:id=\"rId1\" o:title=\"\"/>";
        assert!(!allowed(picture_package(vml, IMAGE)));
    }

    #[test]
    fn names_are_compared_by_local_name_in_any_order() {
        let prefixed = format!(
            "<p:Relationships xmlns:p=\"{NS}\"><p:Relationship Id=\"r\" Type=\"{IMAGE}\" Target=\"{ABS}\" TargetMode=\"External\"/></p:Relationships>"
        );
        assert!(!rels_allowed(&prefixed));
        let prefixed_attributes = format!(
            "<Relationships xmlns=\"{NS}\" xmlns:x=\"urn:x\"><Relationship x:TargetMode=\"External\" x:Target=\"{ABS}\" Id=\"r\" x:Type=\"{IMAGE}\"/></Relationships>"
        );
        assert!(!rels_allowed(&prefixed_attributes));
        let reordered = format!(
            "<Relationship TargetMode=\"External\" Target=\"{ABS}\" Id=\"r\" Type=\"{IMAGE}\"/>"
        );
        assert!(!rels_allowed(&rels_xml(&reordered)));
        let single_quotes = format!("<Relationship Id='r' Type='{IMAGE}' Target='{ABS}' TargetMode='External'/>");
        assert!(!rels_allowed(&rels_xml(&single_quotes)));
        let allowed_reordered = format!(
            "<Relationship Target=\"http://e/a\" TargetMode=\"External\" Id=\"r\" Type=\"{IMAGE}\"/>"
        );
        assert!(rels_allowed(&rels_xml(&allowed_reordered)));
    }

    #[test]
    fn a_part_that_cannot_be_analysed_is_not_allowed() {
        let image = format!("<Relationship Id=\"r\" Type=\"{IMAGE}\" Target=\"http://e/a\" TargetMode=\"External\"/>");
        for xml in [
            String::new(),
            "<Relationships>".to_string(),
            format!("<Relationships>{image}"),
            "<a></b>".to_string(),
            format!("<!DOCTYPE x><Relationships>{image}</Relationships>"),
            format!("<?xml version=\"1.0\" encoding=\"UTF-16\"?><Relationships>{image}</Relationships>"),
            format!("<Relationships>{image}</Relationships>trailing"),
            "<Relationship Target=\"&nope;x\" TargetMode=\"External\"/>".to_string(),
            "<Relationship Target=\"a\" Target=\"b\"/>".to_string(),
            "<Relationships>&nope;</Relationships>".to_string(),
        ] {
            assert!(!rels_allowed(&xml), "{xml}");
        }
    }

    #[test]
    fn only_utf8_is_read() {
        let xml = external(IMAGE, "http://example.com/a.png");
        let mut utf16: Vec<u8> = vec![0xFF, 0xFE];
        utf16.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        assert!(!rels_part_allowed(&utf16));
        let no_bom: Vec<u8> = xml.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(!rels_part_allowed(&no_bom));
        let mut with_bom = "\u{FEFF}".as_bytes().to_vec();
        with_bom.extend_from_slice(xml.as_bytes());
        assert!(rels_part_allowed(&with_bom));
        let lower = xml.replace("UTF-8", "utf-8");
        assert!(rels_allowed(&lower));
    }

    fn with_limits(bytes: Vec<u8>, limits: Limits) -> bool {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        all_rels_allowed(&mut archive, &limits)
    }

    #[test]
    fn going_over_a_limit_is_not_allowed() {
        let rels = rels_xml("");
        let two: &[(&str, &[u8])] = &[("a.rels", rels.as_bytes()), ("b.rels", rels.as_bytes())];
        let size = rels.len() as u64;
        let roomy = || Limits { entries: 10, rels_bytes: size, total_rels_bytes: size * 2 };
        assert!(with_limits(package(two), roomy()));
        assert!(!with_limits(package(two), Limits { entries: 1, ..roomy() }));
        assert!(!with_limits(package(two), Limits { rels_bytes: size - 1, ..roomy() }));
        assert!(!with_limits(package(two), Limits { total_rels_bytes: size * 2 - 1, ..roomy() }));
    }

    #[test]
    fn the_extension_of_a_rels_part_is_compared_without_case() {
        let rels = external(IMAGE, ABS);
        assert!(!allowed(package(&[("word/_rels/document.xml.RELS", rels.as_bytes())])));
    }

    #[test]
    fn other_parts_are_not_read() {
        let content = external(IMAGE, ABS);
        assert!(allowed(package(&[("word/document.xml", content.as_bytes())])));
    }

    fn scratch_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("eo-package-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_zip_whose_central_directory_cannot_be_opened_is_left_to_x2t() {
        let bytes = b"PK\x03\x04 not a real archive";
        let path = scratch_file("broken.docx", bytes);
        assert!(relationships_are_allowed(&path, bytes));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_file_that_is_not_a_zip_has_no_relationships_to_check() {
        let bytes = b"\xD0\xCF\x11\xE0 something";
        let path = scratch_file("old.doc", bytes);
        assert!(relationships_are_allowed(&path, bytes));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_package_is_checked_whatever_its_extension() {
        let rels = external(IMAGE, ABS);
        let bytes = package(&[("word/_rels/document.xml.rels", rels.as_bytes())]);
        let path = scratch_file("renamed.doc", &bytes);
        assert!(!relationships_are_allowed(&path, &bytes[..256.min(bytes.len())]));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn corpus_documents(folder: &Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                corpus_documents(&path, found);
                continue;
            }
            let name = path.to_string_lossy().to_ascii_lowercase();
            if [".docx", ".xlsx", ".pptx", ".docm", ".xlsm", ".pptm"].iter().any(|ext| name.ends_with(ext)) {
                found.push(path);
            }
        }
    }

    #[test]
    fn the_corpus_has_no_relationship_outside_the_list() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests");
        let mut documents = Vec::new();
        corpus_documents(&root, &mut documents);
        assert!(!documents.is_empty(), "no corpus found at {}", root.display());
        for document in documents {
            let bytes = std::fs::read(&document).unwrap();
            assert!(relationships_are_allowed(&document, &bytes), "{}", document.display());
        }
    }
}
