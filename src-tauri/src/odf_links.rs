// The pictures an OpenDocument package refers to. The content of a document has
// to match its format: a picture comes from the package or from a URL. When a
// reference is neither, it is emptied in a copy of the package in the temp
// folder, so the document still opens with everything else it holds. The same
// goes for a flat ODF, a single XML document. The original file is never
// touched.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::{NsReader, Reader, Writer};

use crate::package_relationships::{declares_utf8, has_allowed_scheme, part_names_are_unique, percent_decoded};
use crate::x2t_task::ContentSample;

const MANIFEST: &str = "META-INF/manifest.xml";

struct Limits {
    entries: usize,
    part_bytes: u64,
    total_part_bytes: u64,
}

// A content.xml holds the whole document, so its limits are those of a large
// spreadsheet, not those of a .rels part.
const LIMITS: Limits = Limits {
    entries: 20_000,
    part_bytes: 256 * 1024 * 1024,
    total_part_bytes: 512 * 1024 * 1024,
};

#[derive(Debug, PartialEq, Eq)]
pub enum PackageImages {
    // Nothing to change: x2t reads the original.
    Unchanged,
    // x2t reads this copy instead; the caller removes it afterwards.
    Copy(PathBuf),
    NotAllowed,
}

struct NotAnalysable;

// Any file the zip library opens as a package, whatever it starts like. A ZIP
// whose central directory cannot be opened never gets here: the content check
// (converter.rs) refuses it first.
pub fn prepare(path: &Path, content: &ContentSample, temp_dir: &Path) -> PackageImages {
    if has_flat_odf_extension(path) {
        return prepare_flat(path, temp_dir);
    }
    if !content.may_hold_a_package() {
        return PackageImages::Unchanged;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return PackageImages::Unchanged;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return PackageImages::Unchanged;
    };
    if !has_open_document_extension(path) && !archive.file_names().any(is_open_document) {
        return PackageImages::Unchanged;
    }
    let Ok(mut raw) = std::fs::File::open(path) else {
        return PackageImages::NotAllowed;
    };
    if !part_names_are_unique(&mut raw, archive.central_directory_start()) {
        return PackageImages::NotAllowed;
    }
    let Ok(rewritten) = rewritten_parts(&mut archive, &LIMITS) else {
        return PackageImages::NotAllowed;
    };
    if rewritten.is_empty() {
        return PackageImages::Unchanged;
    }
    let copy = copy_path(path, temp_dir);
    if write_copy(&mut archive, &rewritten, &copy).is_err() {
        let _ = std::fs::remove_file(&copy);
        return PackageImages::NotAllowed;
    }
    PackageImages::Copy(copy)
}

// A flat ODF has no parts: every reference that is not a URL on the list, a
// fragment or empty is emptied. The content check (converter.rs) only lets a
// flat ODF extension through with a flat ODF in it.
fn prepare_flat(path: &Path, temp_dir: &Path) -> PackageImages {
    let Some(xml) = read_file(path, LIMITS.part_bytes) else {
        return PackageImages::NotAllowed;
    };
    let new_xml = match neutralized_xml(&xml, &HashSet::new(), "") {
        Ok(Some(new_xml)) => new_xml,
        Ok(None) => return PackageImages::Unchanged,
        Err(NotAnalysable) => return PackageImages::NotAllowed,
    };
    let copy = copy_path(path, temp_dir);
    if std::fs::write(&copy, new_xml).is_err() {
        let _ = std::fs::remove_file(&copy);
        return PackageImages::NotAllowed;
    }
    PackageImages::Copy(copy)
}

fn read_file(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

fn copy_path(source: &Path, temp_dir: &Path) -> PathBuf {
    let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("odt");
    temp_dir.join(format!("eo-package-copy.{}", extension))
}

// The parts x2t takes for encrypted: the manifest:full-path of each
// manifest:file-entry right under the manifest:manifest root that holds a
// manifest:encryption-data right under it (manifest_entry,
// OdfFile/Reader/Format/office_document.cpp; parse_manifests,
// OdfFile/Reader/Format/odf_document_impl.cpp). x2t goes by the prefix written
// in each name, "manifest" (read_doc_element passes namespacePrefix() to
// CP_CHECK_NAME, OdfFile/Common/readdocelement.cpp); here the prefix also has to
// stand for the manifest namespace. Empty when the manifest cannot be read.
fn encrypted_parts<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> HashSet<String> {
    let Ok(xml) = read_part(archive, MANIFEST, LIMITS.part_bytes) else {
        return HashSet::new();
    };
    let Ok(text) = std::str::from_utf8(&xml) else {
        return HashSet::new();
    };
    let mut reader = NsReader::from_str(text.strip_prefix('\u{FEFF}').unwrap_or(text));
    let mut parts = HashSet::new();
    let mut depth = 0usize;
    let mut in_manifest = false;
    let mut entry = None;
    loop {
        let Ok((namespace, event)) = reader.read_resolved_event() else {
            return HashSet::new();
        };
        match event {
            Event::Eof => return parts,
            Event::Start(element) => {
                match depth {
                    0 => in_manifest = is_manifest_element(&namespace, &element, b"manifest:manifest"),
                    1 => entry = file_entry_path(in_manifest, &namespace, &element),
                    2 if is_manifest_element(&namespace, &element, ENCRYPTION_DATA) => parts.extend(entry.clone()),
                    _ => {}
                }
                depth += 1;
            }
            Event::Empty(element) => {
                if depth == 2 && is_manifest_element(&namespace, &element, ENCRYPTION_DATA) {
                    parts.extend(entry.clone());
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 1 {
                    entry = None;
                }
            }
            _ => {}
        }
    }
}

const MANIFEST_NAMESPACE: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";
const ENCRYPTION_DATA: &[u8] = b"manifest:encryption-data";

fn is_manifest_element(namespace: &ResolveResult, element: &BytesStart, name: &[u8]) -> bool {
    let in_namespace = matches!(namespace, ResolveResult::Bound(Namespace(uri)) if *uri == MANIFEST_NAMESPACE);
    in_namespace && element.name().as_ref() == name
}

// The full-path of a manifest:file-entry, by that exact attribute name.
fn file_entry_path(in_manifest: bool, namespace: &ResolveResult, element: &BytesStart) -> Option<String> {
    if !in_manifest || !is_manifest_element(namespace, element, b"manifest:file-entry") {
        return None;
    }
    let attribute = element.attributes().flatten().find(|attribute| attribute.key.as_ref() == b"manifest:full-path")?;
    attribute.unescape_value().ok().map(|value| value.into_owned())
}

fn last_segment(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

// The parts that can hold a reference, in any folder and whatever their case:
// x2t finds them that way on a case-insensitive file system.
fn is_checked_part(name: &str) -> bool {
    let last = last_segment(name);
    last.eq_ignore_ascii_case("content.xml") || last.eq_ignore_ascii_case("styles.xml")
}

// An OpenDocument package has a mimetype entry or a manifest. An OOXML package
// has neither, and its styles.xml parts are not read here.
fn is_open_document(name: &str) -> bool {
    let normalized = name.replace('\\', "/").to_ascii_lowercase();
    normalized == "mimetype" || normalized == "meta-inf/manifest.xml"
}

fn has_open_document_extension(path: &Path) -> bool {
    has_extension(path, &["odt", "ott", "ods", "ots", "odp", "otp"])
}

fn has_flat_odf_extension(path: &Path) -> bool {
    has_extension(path, &["fodt", "fods", "fodp"])
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    extensions.iter().any(|known| extension.eq_ignore_ascii_case(known))
}

// Every entry of the package and every folder above one.
fn package_paths<R: Read + Seek>(archive: &zip::ZipArchive<R>) -> HashSet<String> {
    let mut paths = HashSet::new();
    for name in archive.file_names() {
        let mut path = name.trim_end_matches('/');
        while !path.is_empty() {
            paths.insert(path.to_string());
            path = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        }
    }
    paths
}

// The new content of each part that has to change. None of them is written yet.
fn rewritten_parts<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    limits: &Limits,
) -> Result<HashMap<String, Vec<u8>>, NotAnalysable> {
    if archive.len() > limits.entries {
        return Err(NotAnalysable);
    }
    let paths = package_paths(archive);
    let encrypted = encrypted_parts(archive);
    let names: Vec<String> = archive.file_names().map(String::from).collect();
    let mut rewritten = HashMap::new();
    let mut total = 0;
    for name in names.iter().filter(|name| is_checked_part(name)) {
        // Only a part the manifest declares encrypted may stay unread.
        let is_encrypted = encrypted.contains(name);
        let xml = match read_part(archive, name, limits.part_bytes) {
            Ok(xml) => xml,
            Err(_) if is_encrypted => continue,
            Err(error) => return Err(error),
        };
        total += xml.len() as u64;
        if total > limits.total_part_bytes {
            return Err(NotAnalysable);
        }
        let folder = name.strip_suffix(last_segment(name)).unwrap_or("").trim_end_matches(['/', '\\']);
        match neutralized_xml(&xml, &paths, folder) {
            Ok(Some(new_xml)) => {
                rewritten.insert(name.clone(), new_xml);
            }
            Ok(None) => {}
            Err(_) if is_encrypted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(rewritten)
}

fn read_part<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, NotAnalysable> {
    let entry = archive.by_name(name).map_err(|_| NotAnalysable)?;
    let mut xml = Vec::new();
    entry.take(limit + 1).read_to_end(&mut xml).map_err(|_| NotAnalysable)?;
    if xml.len() as u64 > limit {
        return Err(NotAnalysable);
    }
    Ok(xml)
}

// The part with the references that are not allowed emptied, or None when it
// has none. Everything else is written back as it was read. `folder` is where
// the part sits: an embedded object's references are relative to its own folder.
fn neutralized_xml(
    xml: &[u8],
    paths: &HashSet<String>,
    folder: &str,
) -> Result<Option<Vec<u8>>, NotAnalysable> {
    let text = std::str::from_utf8(xml).map_err(|_| NotAnalysable)?;
    let mut reader = Reader::from_str(text.strip_prefix('\u{FEFF}').unwrap_or(text));
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut depth = 0usize;
    let mut changed = false;
    loop {
        let event = reader.read_event().map_err(|_| NotAnalysable)?;
        let replaced = match &event {
            Event::Eof => break,
            Event::DocType(_) => return Err(NotAnalysable),
            Event::Decl(declaration) if !declares_utf8(declaration) => return Err(NotAnalysable),
            Event::Start(element) => {
                depth += 1;
                neutralized_element(element, paths, folder)?.map(Event::Start)
            }
            Event::Empty(element) => {
                neutralized_element(element, paths, folder)?.map(Event::Empty)
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or(NotAnalysable)?;
                None
            }
            _ => None,
        };
        changed |= replaced.is_some();
        let result = writer.write_event(replaced.unwrap_or(event));
        result.map_err(|_| NotAnalysable)?;
    }
    if depth != 0 {
        return Err(NotAnalysable);
    }
    Ok(changed.then(|| writer.into_inner()))
}

// x2t goes by the attribute's name, whatever its namespace says, so any href
// counts as a reference.
fn is_href(attribute: &Attribute) -> bool {
    attribute.key.local_name().as_ref() == b"href"
}

// The element with its href emptied, or None when it stays as it is. A hyperlink
// element is left alone: x2t does not read what it points to.
fn neutralized_element(
    element: &BytesStart,
    paths: &HashSet<String>,
    folder: &str,
) -> Result<Option<BytesStart<'static>>, NotAnalysable> {
    if element.local_name().as_ref() == b"a" {
        return Ok(None);
    }
    let mut attributes = Vec::new();
    let mut emptied = false;
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|_| NotAnalysable)?;
        let mut value = attribute.value.clone().into_owned();
        if is_href(&attribute) {
            let decoded = attribute.unescape_value().map_err(|_| NotAnalysable)?;
            if !href_allowed(&decoded, paths, folder) {
                value.clear();
                emptied = true;
            }
        }
        attributes.push((attribute.key.as_ref().to_vec(), value));
    }
    if !emptied {
        return Ok(None);
    }
    let name = std::str::from_utf8(element.name().as_ref()).map_err(|_| NotAnalysable)?.to_string();
    let mut fresh = BytesStart::new(name);
    for (key, value) in &attributes {
        fresh.push_attribute(Attribute {
            key: quick_xml::name::QName(key),
            value: std::borrow::Cow::Borrowed(value),
        });
    }
    Ok(Some(fresh))
}

fn href_allowed(href: &str, paths: &HashSet<String>, folder: &str) -> bool {
    if href.is_empty() || href.starts_with('#') || has_allowed_scheme(href) {
        return true;
    }
    let decoded = percent_decoded(href);
    if !is_clean_relative(href) || !is_clean_relative(&decoded) {
        return false;
    }
    in_package(href, paths, folder) || in_package(&decoded, paths, folder)
}

// An entry of the package can have any name, so a reference only counts as one
// of them when it is a plain relative path.
fn is_clean_relative(href: &str) -> bool {
    let path = href.strip_prefix("./").unwrap_or(href);
    !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|segment| segment != "..")
}

fn in_package(href: &str, paths: &HashSet<String>, folder: &str) -> bool {
    let path = href.strip_prefix("./").unwrap_or(href).trim_end_matches('/');
    paths.contains(path) || paths.contains(&format!("{folder}/{path}"))
}

// Parts that do not change are copied as they are stored, in the same order: a
// package starts with its mimetype entry, uncompressed, and stays that way.
fn write_copy<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    rewritten: &HashMap<String, Vec<u8>>,
    copy: &Path,
) -> zip::result::ZipResult<()> {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(copy)?);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for index in 0..archive.len() {
        let name = archive.name_for_index(index).unwrap_or_default().to_string();
        match rewritten.get(&name) {
            Some(xml) => {
                writer.start_file(name, deflated)?;
                writer.write_all(xml)?;
            }
            None => writer.raw_copy_file(archive.by_index_raw(index)?)?,
        }
    }
    writer.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;

    const XLINK: &str = "xmlns:xlink=\"http://www.w3.org/1999/xlink\"";
    const MIMETYPE: &[u8] = b"application/vnd.oasis.opendocument.text";

    fn paths(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn with_image(href: &str) -> String {
        format!("<office:text {XLINK} xmlns:office=\"o\" xmlns:draw=\"d\"><draw:image xlink:href=\"{href}\" xlink:type=\"simple\"/><p>kept text</p></office:text>")
    }

    fn neutralized(xml: &str, names: &[&str]) -> Option<String> {
        let result = neutralized_xml(xml.as_bytes(), &paths(names), "");
        result.unwrap_or_else(|_| panic!("not analysable: {xml}")).map(|bytes| String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn a_reference_outside_the_package_is_emptied_and_the_rest_kept() {
        for href in ["../near.png", "../../images/a.png", "/home/a.png", "C:\\images\\a.png", "\\\\server\\a.png", "near.png"] {
            let out = neutralized(&with_image(href), &["Pictures/a.png"]).unwrap_or_else(|| panic!("{href}"));
            assert_eq!(out, with_image(""), "{href}");
        }
    }

    #[test]
    fn references_inside_the_package_or_to_a_url_stay() {
        let names = ["Pictures/a.png", "Object 1", "Object 1/content.xml", "ObjectReplacements/Object 1"];
        for href in [
            "Pictures/a.png",
            "./Pictures/a.png",
            "Pictures/a%2Epng",
            "./Object 1",
            "Object 1/",
            "#Object 1",
            "",
            "http://example.com/a.png",
            "file:///home/a.png",
            "HTTPS://example.com/a.png",
            "mailto:a@example.com",
            "ftp://example.com/a.png",
        ] {
            assert_eq!(neutralized(&with_image(href), &names), None, "{href}");
        }
    }

    #[test]
    fn dot_dot_inside_a_path_is_not_in_the_package() {
        assert!(neutralized(&with_image("Pictures/../../a.png"), &["Pictures/a.png"]).is_some());
    }

    #[test]
    fn any_attribute_with_the_local_name_href_is_a_reference() {
        for (root, attribute) in [
            ("xmlns:xlink=\"urn:not-xlink\"", "xlink:href"),
            ("xmlns:zz=\"http://www.w3.org/1999/xlink\"", "zz:href"),
            ("", "undeclared:href"),
            ("", "href"),
        ] {
            let xml = format!("<a:root xmlns:a=\"d\" {root}><a:image {attribute}=\"../x.png\"/></a:root>");
            let expected = format!("<a:root xmlns:a=\"d\" {root}><a:image {attribute}=\"\"/></a:root>");
            assert_eq!(neutralized(&xml, &[]).unwrap(), expected, "{attribute}");
        }
        let other = "<a:root xmlns:a=\"d\"><a:image a:name=\"../x.png\"/></a:root>";
        assert_eq!(neutralized(other, &[]), None);
    }

    #[test]
    fn a_reference_named_like_an_odd_entry_is_not_in_the_package() {
        for entry in ["/etc/x", "../x", "C:\\x", "a\\b", "..\\x", "sub/../x"] {
            let names = [entry];
            assert!(neutralized(&with_image(entry), &names).is_some(), "{entry}");
        }
        assert!(neutralized(&with_image("%2Fetc%2Fx"), &["/etc/x"]).is_some());
        assert!(neutralized(&with_image("%2E%2E%2Fx"), &["../x"]).is_some());
        assert_eq!(neutralized(&with_image("Pictures/a.png"), &["Pictures/a.png"]), None);
    }

    #[test]
    fn a_hyperlink_is_left_alone() {
        let xml = format!("<text:p {XLINK} xmlns:text=\"t\"><text:a xlink:href=\"../other.pdf\">link</text:a></text:p>");
        assert_eq!(neutralized(&xml, &[]), None);
    }

    #[test]
    fn entities_and_percent_encoding_are_decoded_first() {
        assert_eq!(neutralized(&with_image("Pictures/a&amp;b.png"), &["Pictures/a&b.png"]), None);
        assert!(neutralized(&with_image("&#46;&#46;/a.png"), &[]).is_some());
        assert!(neutralized(&with_image("%2E%2E/a.png"), &[]).is_some());
    }

    #[test]
    fn a_part_that_cannot_be_analysed_is_not_allowed() {
        for xml in [
            "<a><b></a>".to_string(),
            "<a>".to_string(),
            format!("<!DOCTYPE x>{}", with_image("Pictures/a.png")),
            format!("<?xml version=\"1.0\" encoding=\"UTF-16\"?>{}", with_image("Pictures/a.png")),
            with_image("&nope;"),
        ] {
            assert!(neutralized_xml(xml.as_bytes(), &HashSet::new(), "").is_err(), "{xml}");
        }
        assert!(neutralized_xml(b"\xFF\xFE<\0a\0/\0>\0", &HashSet::new(), "").is_err());
    }

    fn package(parts: &[(&str, &[u8], zip::CompressionMethod)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content, method) in parts {
            writer.start_file(*name, SimpleFileOptions::default().compression_method(*method)).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("eo-odf-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let temp = dir.join("temp");
        std::fs::create_dir_all(&temp).unwrap();
        (dir, temp)
    }

    const STORED: zip::CompressionMethod = zip::CompressionMethod::Stored;
    const DEFLATED: zip::CompressionMethod = zip::CompressionMethod::Deflated;

    fn odt(href: &str) -> Vec<u8> {
        package(&[
            ("mimetype", MIMETYPE, STORED),
            ("META-INF/manifest.xml", b"<manifest/>", DEFLATED),
            ("content.xml", with_image(href).as_bytes(), DEFLATED),
            ("styles.xml", b"<styles/>", DEFLATED),
            ("Pictures/a.png", b"png bytes", STORED),
        ])
    }

    fn prepared(name: &str, bytes: &[u8]) -> (PathBuf, PathBuf, PackageImages) {
        let (dir, temp) = scratch(name);
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let outcome = prepare(&path, &ContentSample::of(bytes), &temp);
        (dir, temp, outcome)
    }

    #[test]
    fn a_package_without_outside_references_is_left_as_it_is() {
        let (dir, temp, outcome) = prepared("clean.odt", &odt("Pictures/a.png"));
        assert_eq!(outcome, PackageImages::Unchanged);
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "no copy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_copy_keeps_every_other_part_and_the_mimetype_first_and_stored() {
        let original = odt("../near.png");
        let (dir, _, outcome) = prepared("linked.odt", &original);
        let PackageImages::Copy(copy) = outcome else {
            panic!("{outcome:?}");
        };
        let mut before = zip::ZipArchive::new(Cursor::new(original)).unwrap();
        let mut after = zip::ZipArchive::new(std::fs::File::open(&copy).unwrap()).unwrap();
        assert_eq!(after.name_for_index(0), Some("mimetype"));
        assert_eq!(after.by_index_raw(0).unwrap().compression(), STORED);
        assert_eq!(before.len(), after.len());
        for index in 0..before.len() {
            let name = before.name_for_index(index).unwrap().to_string();
            let mut was = Vec::new();
            let mut is = Vec::new();
            before.by_index(index).unwrap().read_to_end(&mut was).unwrap();
            after.by_name(&name).unwrap().read_to_end(&mut is).unwrap();
            if name == "content.xml" {
                assert_eq!(String::from_utf8(is).unwrap(), with_image(""));
            } else {
                assert_eq!(was, is, "{name}");
            }
        }
        assert_eq!(copy.file_name().unwrap(), "eo-package-copy.odt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn styles_and_embedded_objects_are_checked_too() {
        let image = with_image("../near.png");
        let parts: [&[(&str, &[u8], zip::CompressionMethod)]; 2] = [
            &[("mimetype", MIMETYPE, STORED), ("content.xml", b"<c/>", DEFLATED), ("styles.xml", image.as_bytes(), DEFLATED)],
            &[("mimetype", MIMETYPE, STORED), ("content.xml", b"<c/>", DEFLATED), ("Object 1/content.xml", image.as_bytes(), DEFLATED)],
        ];
        for (index, part) in parts.iter().enumerate() {
            let (dir, _, outcome) = prepared(&format!("part{index}.odt"), &package(part));
            assert!(matches!(outcome, PackageImages::Copy(_)), "{outcome:?}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_reference_inside_an_embedded_object_is_relative_to_its_folder() {
        let image = with_image("Pictures/a.png");
        let bytes = package(&[
            ("mimetype", MIMETYPE, STORED),
            ("content.xml", b"<c/>", DEFLATED),
            ("Object 1/content.xml", image.as_bytes(), DEFLATED),
            ("Object 1/Pictures/a.png", b"png", STORED),
        ]);
        let (dir, _, outcome) = prepared("object.odt", &bytes);
        assert_eq!(outcome, PackageImages::Unchanged);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_odf_extension_is_enough_without_a_mimetype_or_manifest() {
        let image = with_image("../near.png");
        let bytes = package(&[("content.xml", image.as_bytes(), DEFLATED)]);
        for name in ["only-content.odt", "ONLY-CONTENT.ODS"] {
            let (dir, _, outcome) = prepared(name, &bytes);
            assert!(matches!(outcome, PackageImages::Copy(_)), "{name}: {outcome:?}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_package_without_a_mimetype_or_manifest_is_not_read() {
        let image = with_image("../near.png");
        let bytes = package(&[
            ("word/document.xml", image.as_bytes(), DEFLATED),
            ("word/styles.xml", image.as_bytes(), DEFLATED),
        ]);
        let (dir, _, outcome) = prepared("other.docx", &bytes);
        assert_eq!(outcome, PackageImages::Unchanged);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_part_is_found_in_any_folder_and_any_case() {
        let image = with_image("../near.png");
        for name in ["Content.xml", "CONTENT.XML", "styles.XML", "sub/content.xml", "Object 1/Styles.xml", "sub\\content.xml"] {
            let bytes = package(&[("mimetype", MIMETYPE, STORED), (name, image.as_bytes(), DEFLATED)]);
            let (dir, _, outcome) = prepared("named.odt", &bytes);
            assert!(matches!(outcome, PackageImages::Copy(_)), "{name}: {outcome:?}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn the_mimetype_and_manifest_are_found_in_any_case() {
        let image = with_image("../near.png");
        for gate in ["MIMETYPE", "Meta-Inf/Manifest.XML"] {
            let bytes = package(&[(gate, b"x", DEFLATED), ("content.xml", image.as_bytes(), DEFLATED)]);
            let (dir, _, outcome) = prepared("case.odt", &bytes);
            assert!(matches!(outcome, PackageImages::Copy(_)), "{gate}: {outcome:?}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn the_ooxml_files_of_the_corpus_are_left_as_they_are() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus");
        let mut checked = 0;
        for entry in std::fs::read_dir(&root).unwrap() {
            let path = entry.unwrap().path();
            let is_ooxml = matches!(path.extension().and_then(|e| e.to_str()), Some("docx" | "xlsx" | "pptx"));
            if !is_ooxml {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let (dir, temp) = scratch("corpus-ooxml");
            assert_eq!(prepare(&path, &ContentSample::of(&bytes), &temp), PackageImages::Unchanged, "{}", path.display());
            let _ = std::fs::remove_dir_all(&dir);
            checked += 1;
        }
        assert!(checked >= 3, "{checked}");
    }

    // The package with each `from` replaced by `to`, of the same length: the zip
    // library does not write two entries with one name.
    fn renamed(mut bytes: Vec<u8>, from: &str, to: &str) -> Vec<u8> {
        assert_eq!(from.len(), to.len());
        let found: Vec<usize> = bytes.windows(from.len()).enumerate().filter(|(_, w)| *w == from.as_bytes()).map(|(at, _)| at).collect();
        assert_eq!(found.len(), 2, "a local header and a directory entry");
        for at in found {
            bytes[at..at + to.len()].copy_from_slice(to.as_bytes());
        }
        bytes
    }

    #[test]
    fn parts_with_the_same_name_or_one_only_differing_in_case_are_not_allowed() {
        let linked = with_image("../near.png");
        let clean = with_image("Pictures/a.png");
        let two = package(&[
            ("mimetype", MIMETYPE, STORED),
            ("content.xml", linked.as_bytes(), DEFLATED),
            ("contenX.xml", clean.as_bytes(), DEFLATED),
            ("Pictures/a.png", b"png", STORED),
        ]);
        let same_name = renamed(two, "contenX.xml", "content.xml");
        let other_case = package(&[
            ("mimetype", MIMETYPE, STORED),
            ("content.xml", clean.as_bytes(), DEFLATED),
            ("Content.xml", linked.as_bytes(), DEFLATED),
            ("Pictures/a.png", b"png", STORED),
        ]);
        for (name, bytes) in [("same-name.odt", same_name), ("other-case.odt", other_case)] {
            let (dir, temp, outcome) = prepared(name, &bytes);
            assert_eq!(outcome, PackageImages::NotAllowed, "{name}");
            assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "no copy");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn an_odf_package_after_a_cfb_start_is_neutralized_too() {
        let mut bytes = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec();
        bytes.extend_from_slice(&[0; 504]);
        bytes.extend_from_slice(&odt("../near.png"));
        let (dir, _, outcome) = prepared("behind.odt", &bytes);
        assert!(matches!(outcome, PackageImages::Copy(_)), "{outcome:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn flat(body: &str) -> String {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document {XLINK} xmlns:office=\"o\" xmlns:draw=\"d\" xmlns:text=\"t\" office:version=\"1.3\"><office:body>{body}</office:body></office:document>")
    }

    fn flat_image(href: &str) -> String {
        flat(&format!("<draw:image xlink:href=\"{href}\" xlink:type=\"simple\"/><text:p>kept text</text:p>"))
    }

    #[test]
    fn a_flat_odf_has_its_local_references_emptied_in_a_copy() {
        for href in ["Pictures/a.png", "./Pictures/a.png", "../near.png", "/home/a.png", "C:\\images\\a.png", "Object 1"] {
            for name in ["flat.fodt", "flat.FODS", "flat.fodp"] {
                let (dir, _, outcome) = prepared(name, flat_image(href).as_bytes());
                let PackageImages::Copy(copy) = outcome else {
                    panic!("{name} {href}: {outcome:?}");
                };
                assert_eq!(std::fs::read_to_string(&copy).unwrap(), flat_image(""), "{href}");
                assert_eq!(copy.extension(), Path::new(name).extension());
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }

    #[test]
    fn urls_fragments_and_hyperlinks_stay_in_a_flat_odf() {
        for href in ["http://example.com/a.png", "file:///home/a.png", "#Object 1", ""] {
            let (dir, temp, outcome) = prepared("flat.fodt", flat_image(href).as_bytes());
            assert_eq!(outcome, PackageImages::Unchanged, "{href}");
            assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "no copy");
            let _ = std::fs::remove_dir_all(&dir);
        }
        let hyperlink = flat("<text:p><text:a xlink:href=\"../other.odt\">link</text:a></text:p>");
        let (dir, _, outcome) = prepared("link.fodt", hyperlink.as_bytes());
        assert_eq!(outcome, PackageImages::Unchanged);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_flat_odf_documents_of_the_corpus_are_read() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus/src");
        for name in ["basic.fodt", "basic.fods", "basic.fodp"] {
            let bytes = std::fs::read(root.join(name)).unwrap();
            let (dir, _, outcome) = prepared(name, &bytes);
            assert_ne!(outcome, PackageImages::NotAllowed, "{name}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    fn manifest(entries: &str) -> String {
        format!("<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\">{entries}</manifest:manifest>")
    }

    fn encrypted_entry(part: &str) -> String {
        format!("<manifest:file-entry manifest:full-path=\"{part}\"><manifest:encryption-data manifest:checksum=\"AA==\"><manifest:algorithm/></manifest:encryption-data></manifest:file-entry>")
    }

    const CIPHERTEXT: &[u8] = b"\x00\x01\xF0 ciphertext";

    #[test]
    fn an_encrypted_package_is_left_to_x2t() {
        let entries = encrypted_entry("content.xml") + &encrypted_entry("styles.xml");
        let bytes = package(&[
            ("META-INF/manifest.xml", manifest(&entries).as_bytes(), DEFLATED),
            ("content.xml", CIPHERTEXT, DEFLATED),
            ("styles.xml", CIPHERTEXT, DEFLATED),
        ]);
        let (dir, _, outcome) = prepared("locked.odt", &bytes);
        assert_eq!(outcome, PackageImages::Unchanged);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_mark_in_a_comment_does_not_make_a_part_encrypted() {
        let comment = manifest("<!-- manifest:encryption-data --><manifest:file-entry manifest:full-path=\"content.xml\"/>");
        let unreadable = package(&[
            ("META-INF/manifest.xml", comment.as_bytes(), DEFLATED),
            ("content.xml", CIPHERTEXT, DEFLATED),
        ]);
        let (dir, _, outcome) = prepared("comment.odt", &unreadable);
        assert_eq!(outcome, PackageImages::NotAllowed);
        let _ = std::fs::remove_dir_all(&dir);
        let readable = with_image("../near.png");
        let linked = package(&[
            ("META-INF/manifest.xml", comment.as_bytes(), DEFLATED),
            ("content.xml", readable.as_bytes(), DEFLATED),
        ]);
        let (dir, _, outcome) = prepared("comment-linked.odt", &linked);
        assert!(matches!(outcome, PackageImages::Copy(_)), "{outcome:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_mark_on_another_entry_does_not_make_a_part_encrypted() {
        let other = manifest(&encrypted_entry("other.bin"));
        let unreadable = package(&[
            ("META-INF/manifest.xml", other.as_bytes(), DEFLATED),
            ("content.xml", CIPHERTEXT, DEFLATED),
        ]);
        let (dir, _, outcome) = prepared("other.odt", &unreadable);
        assert_eq!(outcome, PackageImages::NotAllowed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_readable_part_is_neutralized_even_when_declared_encrypted() {
        let declared = manifest(&encrypted_entry("content.xml"));
        let readable = with_image("../near.png");
        let bytes = package(&[
            ("META-INF/manifest.xml", declared.as_bytes(), DEFLATED),
            ("content.xml", readable.as_bytes(), DEFLATED),
        ]);
        let (dir, _, outcome) = prepared("declared.odt", &bytes);
        let PackageImages::Copy(copy) = outcome else {
            panic!("{outcome:?}");
        };
        let mut after = zip::ZipArchive::new(std::fs::File::open(&copy).unwrap()).unwrap();
        let mut content = String::new();
        after.by_name("content.xml").unwrap().read_to_string(&mut content).unwrap();
        assert_eq!(content, with_image(""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_encrypted_part_does_not_stop_the_others_from_being_neutralized() {
        let declared = manifest(&encrypted_entry("styles.xml"));
        let readable = with_image("../near.png");
        let bytes = package(&[
            ("META-INF/manifest.xml", declared.as_bytes(), DEFLATED),
            ("content.xml", readable.as_bytes(), DEFLATED),
            ("styles.xml", CIPHERTEXT, DEFLATED),
        ]);
        let (dir, _, outcome) = prepared("mixed.odt", &bytes);
        let PackageImages::Copy(copy) = outcome else {
            panic!("{outcome:?}");
        };
        let mut after = zip::ZipArchive::new(std::fs::File::open(&copy).unwrap()).unwrap();
        let mut content = String::new();
        after.by_name("content.xml").unwrap().read_to_string(&mut content).unwrap();
        assert_eq!(content, with_image(""));
        let mut styles = Vec::new();
        after.by_name("styles.xml").unwrap().read_to_end(&mut styles).unwrap();
        assert_eq!(styles, CIPHERTEXT, "an encrypted part is copied as it is");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_part_declared_encrypted_otherwise_than_x2t_reads_it_is_not_allowed_when_unreadable() {
        let ns = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";
        let data = "<manifest:encryption-data manifest:checksum=\"AA==\"/>";
        for declared in [
            format!("<m:manifest xmlns:m=\"{ns}\"><m:file-entry m:full-path=\"content.xml\"><m:encryption-data/></m:file-entry></m:manifest>"),
            format!("<manifest:manifest xmlns:manifest=\"urn:other\"><manifest:file-entry manifest:full-path=\"content.xml\">{data}</manifest:file-entry></manifest:manifest>"),
            manifest(&format!("<manifest:file-entry manifest:full-path=\"content.xml\"><manifest:x>{data}</manifest:x></manifest:file-entry>")),
            manifest(&format!("<manifest:x><manifest:file-entry manifest:full-path=\"content.xml\">{data}</manifest:file-entry></manifest:x>")),
            manifest(&format!("<manifest:file-entry xmlns:m=\"{ns}\" m:full-path=\"content.xml\">{data}</manifest:file-entry>")),
            format!("<manifest:other xmlns:manifest=\"{ns}\"><manifest:file-entry manifest:full-path=\"content.xml\">{data}</manifest:file-entry></manifest:other>"),
        ] {
            let bytes = package(&[
                ("META-INF/manifest.xml", declared.as_bytes(), DEFLATED),
                ("content.xml", CIPHERTEXT, DEFLATED),
            ]);
            let (dir, _, outcome) = prepared("declared.odt", &bytes);
            assert_eq!(outcome, PackageImages::NotAllowed, "{declared}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_manifest_that_cannot_be_read_declares_nothing_encrypted() {
        let bytes = package(&[
            ("META-INF/manifest.xml", b"<a><b></a>", DEFLATED),
            ("content.xml", CIPHERTEXT, DEFLATED),
        ]);
        let (dir, _, outcome) = prepared("bad-manifest.odt", &bytes);
        assert_eq!(outcome, PackageImages::NotAllowed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zip_whose_central_directory_cannot_be_opened_is_left_to_x2t() {
        let (dir, _, outcome) = prepared("broken.odt", b"PK\x03\x04 nothing else");
        assert_eq!(outcome, PackageImages::Unchanged);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_part_that_cannot_be_analysed_means_the_package_is_not_allowed() {
        let bytes = package(&[("mimetype", MIMETYPE, STORED), ("content.xml", b"<a><b></a>", DEFLATED)]);
        let (dir, temp, outcome) = prepared("broken.odt", &bytes);
        assert_eq!(outcome, PackageImages::NotAllowed);
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "no copy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_that_cannot_be_written_means_the_package_is_not_allowed() {
        let (dir, temp) = scratch("no-room");
        let path = dir.join("linked.odt");
        let bytes = odt("../near.png");
        std::fs::write(&path, &bytes).unwrap();
        let missing = temp.join("gone");
        assert_eq!(prepare(&path, &ContentSample::of(&bytes), &missing), PackageImages::NotAllowed);
        assert!(!missing.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn analysed(bytes: Vec<u8>, limits: Limits) -> bool {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        rewritten_parts(&mut archive, &limits).is_ok()
    }

    #[test]
    fn going_over_a_limit_means_the_package_is_not_allowed() {
        let xml = with_image("Pictures/a.png");
        let parts: &[(&str, &[u8], zip::CompressionMethod)] =
            &[("content.xml", xml.as_bytes(), DEFLATED), ("styles.xml", xml.as_bytes(), DEFLATED)];
        let size = xml.len() as u64;
        let roomy = || Limits { entries: 10, part_bytes: size, total_part_bytes: size * 2 };
        assert!(analysed(package(parts), roomy()));
        assert!(!analysed(package(parts), Limits { entries: 1, ..roomy() }));
        assert!(!analysed(package(parts), Limits { part_bytes: size - 1, ..roomy() }));
        assert!(!analysed(package(parts), Limits { total_part_bytes: size * 2 - 1, ..roomy() }));
    }

    #[test]
    fn the_odf_files_of_the_corpus_are_left_as_they_are() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus");
        let mut checked = 0;
        for entry in std::fs::read_dir(&root).unwrap() {
            let path = entry.unwrap().path();
            let is_odf = matches!(path.extension().and_then(|e| e.to_str()), Some("odt" | "ods" | "odp"));
            if !is_odf {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let (dir, temp) = scratch("corpus");
            assert_eq!(prepare(&path, &ContentSample::of(&bytes), &temp), PackageImages::Unchanged, "{}", path.display());
            let _ = std::fs::remove_dir_all(&dir);
            checked += 1;
        }
        assert!(checked >= 3, "{checked}");
    }
}
