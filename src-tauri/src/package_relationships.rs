// The relationships an OOXML package declares in its .rels parts. The content
// of a document has to match its format, and a package holds its own parts:
// what it points to outside itself has to be a URL.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::x2t_task::{ContentSample, ZIP_END_SEARCH_LEN};

const ALLOWED_SCHEMES: [&str; 5] = ["http", "https", "mailto", "ftp", "file"];

// x2t 10.0.0.169 only follows image relationships: these two Types never read
// their target, and a hyperlink to another file is an ordinary thing to have.
const HYPERLINK_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/hyperlink",
];

// A relationship of these types brings another whole document into this one,
// stored in a part of the package or not: a document only uses its own parts.
const ALT_CHUNK_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/aFChunk",
];

// Office writes this target for a linked picture or media file it has lost: it
// names no part and no URL. x2t takes a video or sound with this target for one
// without a linked file (Pic::GetMediaLink, PPTXFormat/Logic/Pic.cpp at
// v10.0.0.169).
const MISSING_TARGET: &str = "NULL";

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

// Whether the file's relationships are all acceptable, for any file the zip
// library opens as a package, whatever it starts like. A file that cannot hold a
// package has none. One that starts like a ZIP and does not open is refused by
// the content check (converter.rs) whatever this says. One that starts like a
// CFB, has the end of a central directory in its tail and does not open is not
// accepted: its parts cannot be checked.
pub fn relationships_are_allowed(path: &Path, content: &ContentSample) -> bool {
    if !content.may_hold_a_package() {
        return true;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return true;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return !content.starts_like_a_cfb();
    };
    let Ok(mut raw) = std::fs::File::open(path) else {
        return false;
    };
    part_names_are_unique(&mut raw, archive.central_directory_start())
        && all_rels_allowed(&mut archive, &LIMITS)
}

// Each part of a package has a name of its own. x2t extracts a package into a
// folder (OfficeUtils/src/ZipUtilsCP.cpp), where on Windows and macOS two names
// that differ only in case, or in "/" and "\", are one file; and the zip library
// keeps one entry per name (SharedBuilder::build in zip 2.4.2). So the names are
// read here from the central directory minizip reads (unzOpenInternal in
// OfficeUtils/src/zlib-1.2.11/contrib/minizip/unzip.c): the one that ends where
// the end record in the last 0xFFFF bytes begins, the Zip64 one first. It has to
// be the one the zip library read, `library_start` being where that one starts.
pub(crate) fn part_names_are_unique<R: Read + Seek>(reader: &mut R, library_start: u64) -> bool {
    unique_names_within(reader, library_start, &DIRECTORY_LIMITS)
}

const END_SIGNATURE: &[u8] = b"PK\x05\x06";
const ZIP64_LOCATOR_SIGNATURE: &[u8] = b"PK\x06\x07";
const ZIP64_END_SIGNATURE: &[u8] = b"PK\x06\x06";
const ENTRY_SIGNATURE: &[u8] = b"PK\x01\x02";
const END_LEN: usize = 22;
const ZIP64_LOCATOR_LEN: usize = 20;
const ZIP64_END_LEN: usize = 56;
const ENTRY_LEN: usize = 46;

struct DirectoryLimits {
    entries: u64,
    bytes: u64,
}

const DIRECTORY_LIMITS: DirectoryLimits = DirectoryLimits { entries: 20_000, bytes: 16 * 1024 * 1024 };

// Where a central directory ends, its size and how many entries it says it has.
struct DirectoryEnd {
    end: u64,
    size: u64,
    entries: u64,
}

fn unique_names_within<R: Read + Seek>(reader: &mut R, library_start: u64, limits: &DirectoryLimits) -> bool {
    let Some(end) = directory_end(reader) else {
        return false;
    };
    if end.size > limits.bytes || end.entries > limits.entries {
        return false;
    }
    if end.end.checked_sub(end.size) != Some(library_start) {
        return false;
    }
    let Some(directory) = read_at(reader, library_start, end.size as usize) else {
        return false;
    };
    let Some(names) = entry_names(&directory) else {
        return false;
    };
    names.len() as u64 == end.entries && no_two_alike(&names)
}

// The end of the central directory minizip finds. A tail with more than one end
// record, or with a Zip64 locator anywhere but right before it, could lead the
// zip library and minizip to different directories, so it has none.
fn directory_end<R: Read + Seek>(reader: &mut R) -> Option<DirectoryEnd> {
    let len = reader.seek(SeekFrom::End(0)).ok()?;
    let tail_start = len.saturating_sub(ZIP_END_SEARCH_LEN as u64);
    let tail = read_at(reader, tail_start, (len - tail_start) as usize)?;
    let ends = positions(&tail, END_SIGNATURE);
    let locators = positions(&tail, ZIP64_LOCATOR_SIGNATURE);
    let [end] = ends[..] else {
        return None;
    };
    match locators[..] {
        [] => zip32_end(&tail[end..], tail_start + end as u64),
        [locator] if locator + ZIP64_LOCATOR_LEN == end => zip64_end(reader, &tail[locator..end]),
        _ => None,
    }
}

fn zip32_end(record: &[u8], at: u64) -> Option<DirectoryEnd> {
    if record.len() < END_LEN {
        return None;
    }
    Some(DirectoryEnd { end: at, size: u32_at(record, 12) as u64, entries: u16_at(record, 10) as u64 })
}

// minizip reads the Zip64 end record where the locator says, from the start of
// the file.
fn zip64_end<R: Read + Seek>(reader: &mut R, locator: &[u8]) -> Option<DirectoryEnd> {
    let at = u64_at(locator, 8);
    let record = read_at(reader, at, ZIP64_END_LEN)?;
    if !record.starts_with(ZIP64_END_SIGNATURE) {
        return None;
    }
    Some(DirectoryEnd { end: at, size: u64_at(&record, 40), entries: u64_at(&record, 32) })
}

// The name of every entry of a central directory, in order. None when it holds
// anything but whole entries.
fn entry_names(directory: &[u8]) -> Option<Vec<&[u8]>> {
    let mut names = Vec::new();
    let mut rest = directory;
    while !rest.is_empty() {
        if rest.len() < ENTRY_LEN || !rest.starts_with(ENTRY_SIGNATURE) {
            return None;
        }
        let name_len = u16_at(rest, 28) as usize;
        let entry_len = ENTRY_LEN + name_len + u16_at(rest, 30) as usize + u16_at(rest, 32) as usize;
        if rest.len() < entry_len {
            return None;
        }
        names.push(&rest[ENTRY_LEN..ENTRY_LEN + name_len]);
        rest = &rest[entry_len..];
    }
    Some(names)
}

fn no_two_alike(names: &[&[u8]]) -> bool {
    let mut seen = HashSet::new();
    names.iter().all(|name| seen.insert(folded_name(name)))
}

// A name as a case-insensitive file system sees it. A name that is not UTF-8 is
// compared byte by byte, ASCII letters folded.
fn folded_name(name: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(name) else {
        return name.iter().map(|byte| if *byte == b'\\' { b'/' } else { byte.to_ascii_lowercase() }).collect();
    };
    text.to_lowercase().replace('\\', "/").into_bytes()
}

fn read_at<R: Read + Seek>(reader: &mut R, at: u64, len: usize) -> Option<Vec<u8>> {
    reader.seek(SeekFrom::Start(at)).ok()?;
    let mut bytes = Vec::with_capacity(len);
    reader.take(len as u64).read_to_end(&mut bytes).ok()?;
    (bytes.len() == len).then_some(bytes)
}

fn positions(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let windows = haystack.windows(needle.len()).enumerate();
    windows.filter(|(_, window)| *window == needle).map(|(at, _)| at).collect()
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn all_rels_allowed<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, limits: &Limits) -> bool {
    if archive.len() > limits.entries {
        return false;
    }
    let mut parts = Vec::new();
    let mut total = 0;
    for index in 0..archive.len() {
        let Some(name) = archive.name_for_index(index).map(String::from) else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".rels") {
            continue;
        }
        let Some(xml) = read_rels(archive, index, limits.rels_bytes) else {
            return false;
        };
        total += xml.len() as u64;
        if total > limits.total_rels_bytes {
            return false;
        }
        let Some(relationships) = read_relationships(&xml) else {
            return false;
        };
        parts.push(RelsPart { name, relationships });
    }
    let entries: HashSet<&str> = archive.file_names().collect();
    parts.iter().all(|part| {
        let folder = source_folder(&part.name);
        part.relationships.iter().all(|relationship| relationship_allowed(relationship, folder, &entries))
    })
}

// A .rels part and the relationships it declares.
struct RelsPart {
    name: String,
    relationships: Vec<Relationship>,
}

// The attributes of a Relationship element, by local name: a prefixed one counts
// as well. x2t only takes it for external when it has a TargetMode attribute,
// without a prefix, of exactly "External" (CRelationShip::IsExternal and
// ReadAttributes, OOXML/DocxFormat/Rels.cpp).
#[derive(Default)]
struct Relationship {
    kinds: Vec<String>,
    targets: Vec<String>,
    modes: Vec<String>,
    x2t_external: bool,
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

// Only UTF-8 is read, and a part that cannot be analysed has None.
fn read_relationships(xml: &[u8]) -> Option<Vec<Relationship>> {
    if xml.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(xml).ok()?;
    let mut reader = Reader::from_str(text.strip_prefix('\u{FEFF}').unwrap_or(text));
    let mut relationships = Vec::new();
    let mut depth = 0usize;
    let mut saw_element = false;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return (depth == 0 && saw_element).then_some(relationships),
            Ok(Event::Decl(declaration)) => {
                if !declares_utf8(&declaration) {
                    return None;
                }
            }
            Ok(Event::Start(element)) => {
                relationships.extend(relationship(&element)?);
                depth += 1;
                saw_element = true;
            }
            Ok(Event::Empty(element)) => {
                relationships.extend(relationship(&element)?);
                saw_element = true;
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Text(text)) => {
                if depth == 0 && !text.iter().all(u8::is_ascii_whitespace) {
                    return None;
                }
            }
            Ok(Event::Comment(_)) | Ok(Event::PI(_)) => {}
            Ok(_) => return None,
            Err(_) => return None,
        }
    }
}

pub(crate) fn declares_utf8(declaration: &quick_xml::events::BytesDecl) -> bool {
    match declaration.encoding() {
        None => true,
        Some(Ok(encoding)) => encoding.eq_ignore_ascii_case(b"utf-8"),
        Some(Err(_)) => false,
    }
}

// Some(None) for an element that is not a Relationship, None when its
// attributes cannot be read.
fn relationship(element: &BytesStart) -> Option<Option<Relationship>> {
    if element.local_name().as_ref() != b"Relationship" {
        return Some(None);
    }
    let mut relationship = Relationship::default();
    for attribute in element.attributes() {
        let attribute = attribute.ok()?;
        let value = attribute.unescape_value().ok()?.into_owned();
        if attribute.key.as_ref() == b"TargetMode" && value == "External" {
            relationship.x2t_external = true;
        }
        match attribute.key.local_name().as_ref() {
            b"Target" => relationship.targets.push(value),
            b"TargetMode" => relationship.modes.push(value),
            b"Type" => relationship.kinds.push(value),
            _ => {}
        }
    }
    Some(Some(relationship))
}

fn relationship_allowed(relationship: &Relationship, folder: &str, entries: &HashSet<&str>) -> bool {
    let Relationship { kinds, targets, modes, x2t_external } = relationship;
    if kinds.iter().any(|kind| is_alt_chunk(kind)) {
        return false;
    }
    let in_package = |target: &String| target_stays_in_package(folder, target, entries);
    if !x2t_external && !targets.iter().all(in_package) {
        return false;
    }
    if modes.iter().all(|mode| mode == "Internal") {
        return true;
    }
    if !kinds.is_empty() && kinds.iter().all(|kind| HYPERLINK_TYPES.contains(&kind.as_str())) {
        return true;
    }
    !targets.is_empty() && targets.iter().all(|target| target == MISSING_TARGET || has_allowed_scheme(target))
}

// The folder of the part a .rels part belongs to: "word" for
// word/_rels/document.xml.rels, the package root for _rels/.rels.
fn source_folder(rels_name: &str) -> &str {
    let folder = rels_name.rsplit_once(['/', '\\']).map_or("", |(folder, _)| folder);
    let (parent, last) = folder.rsplit_once(['/', '\\']).unwrap_or(("", folder));
    if last.eq_ignore_ascii_case("_rels") {
        parent
    } else {
        folder
    }
}

// x2t joins an internal target to the folder of its part, or to the package
// root when it starts with "/" (CreateFile, OOXML/DocxFormat/FileFactory.cpp).
// Presentations and themes look for a part that is not in that folder from the
// package root too (FileContainer::CorrectPathRels,
// OOXML/PPTXFormat/FileContainer.cpp).
fn target_stays_in_package(folder: &str, target: &str, entries: &HashSet<&str>) -> bool {
    let Some(part) = resolved_target(folder, target) else {
        return false;
    };
    entries.contains(part.as_str()) || resolved_target("", target).is_some()
}

// The path in the package x2t reads for an internal target, None when it is
// outside the package. CPath::Normalize (OOXML/SystemUtility/SystemUtility.cpp)
// takes "/" and "\\" for separators and drops empty segments; a ".." followed by
// a separator removes the segment before it, "." included. The file system then
// resolves the "." and the last ".." left.
fn resolved_target(folder: &str, target: &str) -> Option<String> {
    let base = if target.starts_with('/') { "" } else { folder };
    let joined = format!("{base}/{target}");
    let mut normalized: Vec<&str> = Vec::new();
    let mut segments = joined.split(['/', '\\']).peekable();
    while let Some(segment) = segments.next() {
        let followed_by_separator = segments.peek().is_some();
        match segment {
            "" => {}
            ".." if followed_by_separator => {
                normalized.pop()?;
            }
            _ => normalized.push(segment),
        }
    }
    let mut resolved = Vec::new();
    for segment in normalized {
        match segment {
            "." => {}
            ".." => {
                resolved.pop()?;
            }
            _ => resolved.push(segment),
        }
    }
    Some(resolved.join("/"))
}

fn is_alt_chunk(kind: &str) -> bool {
    ALT_CHUNK_TYPES.iter().any(|alt_chunk| kind.eq_ignore_ascii_case(alt_chunk))
}

// The target, with its XML entities already decoded, is percent-decoded before
// its scheme is read. A Windows drive letter is not a scheme on the list.
pub(crate) fn has_allowed_scheme(target: &str) -> bool {
    let decoded = percent_decoded(target);
    let Some((scheme, _)) = decoded.split_once(':') else {
        return false;
    };
    ALLOWED_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str())
}

pub(crate) fn percent_decoded(text: &str) -> String {
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

    // One .rels part, judged in a package of its own.
    fn rels_part_allowed(xml: &[u8]) -> bool {
        allowed(package(&[("word/_rels/document.xml.rels", xml)]))
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

    #[test]
    fn a_lost_linked_picture_written_as_null_is_accepted() {
        let video = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/video";
        assert!(image_target_allowed("NULL"));
        assert!(rels_allowed(&external(video, "NULL")));
        for target in ["null", "NULL ", "NULL/a.png", "/NULL"] {
            assert!(!image_target_allowed(target), "{target}");
        }
    }

    fn internal(target: &str, mode: &str) -> String {
        rels_xml(&format!("<Relationship Id=\"rId1\" Type=\"{IMAGE}\" Target=\"{target}\"{mode}/>"))
    }

    const SLIDE_RELS: &str = "ppt/slides/_rels/slide1.xml.rels";
    const SLIDE_PICTURE: (&str, &[u8]) = ("ppt/media/image1.png", b"png");

    #[test]
    fn an_internal_target_outside_the_package_is_refused() {
        let modes = ["", " TargetMode=\"Internal\"", " TargetMode=\"external\"", " x:TargetMode=\"External\""];
        for target in [
            "../../outside.png",
            "/../outside.png",
            "..\\..\\outside.png",
            "media/../../../outside.png",
            "../word/../../outside.png",
            "file:///../../../outside.png",
        ] {
            for mode in modes {
                let rels = internal(target, mode).replace("<Relationships ", "<Relationships xmlns:x=\"urn:x\" ");
                let bytes = package(&[("word/_rels/document.xml.rels", rels.as_bytes()), ("outside.png", b"png")]);
                assert!(!allowed(bytes), "{target}{mode}");
            }
        }
    }

    #[test]
    fn a_picture_in_the_media_folder_of_a_presentation_is_accepted() {
        for target in ["../media/image1.png", "/ppt/media/image1.png", "..\\media\\image1.png", "./../media/image1.png"] {
            let rels = internal(target, "");
            let bytes = package(&[(SLIDE_RELS, rels.as_bytes()), SLIDE_PICTURE]);
            assert!(allowed(bytes), "{target}");
        }
    }

    #[test]
    fn a_part_that_is_not_there_is_also_looked_for_from_the_package_root() {
        let missing = internal("../media/image2.png", "");
        assert!(!allowed(package(&[(SLIDE_RELS, missing.as_bytes()), SLIDE_PICTURE])));
        let deep = internal("../../../../home/picture.png", "");
        assert!(!allowed(package(&[("a/b/c/d/e/f/_rels/x.xml.rels", deep.as_bytes())])));
        let from_root = internal("media/image2.png", "");
        assert!(allowed(package(&[(SLIDE_RELS, from_root.as_bytes()), SLIDE_PICTURE])));
    }

    // CPath::Normalize removes "./" with the ".." after it, as it would a folder.
    #[test]
    fn a_dot_segment_is_a_folder_for_the_dot_dot_after_it() {
        let rels = internal("./../../outside.png", "");
        assert!(allowed(package(&[("word/_rels/document.xml.rels", rels.as_bytes()), ("outside.png", b"png")])));
        assert!(!allowed(package(&[("word/_rels/document.xml.rels", rels.as_bytes())])));
    }

    #[test]
    fn an_external_target_is_not_resolved_in_the_package() {
        let hyperlink = HYPERLINK_TYPES[0];
        assert!(rels_allowed(&external(hyperlink, "../../../Reports/other.docx")));
        assert!(rels_allowed(&external(IMAGE, "file:///../../../a.png")));
    }

    // The relationship is judged the same way however the picture refers to it.
    #[test]
    fn a_document_with_an_alt_chunk_relationship_is_not_allowed() {
        for kind in ALT_CHUNK_TYPES {
            let internal = format!("<Relationship Id=\"r\" Type=\"{kind}\" Target=\"afchunk.mht\"/>");
            let rels = rels_xml(&internal);
            assert!(!rels_allowed(&rels), "{kind}");
            let docx = package(&[
                ("word/document.xml", b"<w:document/>"),
                ("word/afchunk.mht", b"MIME-Version: 1.0"),
                ("word/_rels/document.xml.rels", rels.as_bytes()),
            ]);
            assert!(!allowed(docx), "{kind}");
        }
    }

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
        assert!(relationships_are_allowed(&path, &ContentSample::of(bytes)));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_file_that_is_not_a_zip_has_no_relationships_to_check() {
        let bytes = b"\xD0\xCF\x11\xE0 something";
        let path = scratch_file("old.doc", bytes);
        assert!(relationships_are_allowed(&path, &ContentSample::of(bytes)));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_package_is_checked_whatever_its_extension() {
        let rels = external(IMAGE, ABS);
        let bytes = package(&[("word/_rels/document.xml.rels", rels.as_bytes())]);
        let path = scratch_file("renamed.doc", &bytes);
        assert!(!relationships_are_allowed(&path, &ContentSample::of(&bytes)));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    // A file of its own for each call: tests run side by side.
    fn unique_scratch_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        scratch_file(&format!("{}-{}", COUNTER.fetch_add(1, Ordering::Relaxed), name), bytes)
    }

    // The whole check, on a file holding `bytes`.
    fn package_accepted(bytes: &[u8]) -> bool {
        let path = unique_scratch_file("package.docx", bytes);
        let accepted = relationships_are_allowed(&path, &ContentSample::of(bytes));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        accepted
    }

    fn occurrences(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
        let windows = haystack.windows(needle.len()).enumerate();
        windows.filter(|(_, window)| *window == needle).map(|(at, _)| at).collect()
    }

    // The package with each `from` replaced by `to`, of the same length: the zip
    // library does not write two entries with one name.
    fn renamed(mut bytes: Vec<u8>, from: &str, to: &str) -> Vec<u8> {
        assert_eq!(from.len(), to.len());
        let found = occurrences(&bytes, from.as_bytes());
        assert_eq!(found.len(), 2, "a local header and a directory entry");
        for at in found {
            bytes[at..at + to.len()].copy_from_slice(to.as_bytes());
        }
        bytes
    }

    fn opens(bytes: &[u8]) -> bool {
        zip::ZipArchive::new(Cursor::new(bytes.to_vec())).is_ok()
    }

    const DOCUMENT: (&str, &[u8]) = ("word/document.xml", b"<w:document/>");

    #[test]
    fn exact_duplicate_parts_are_refused() {
        let bytes = renamed(package(&[DOCUMENT, ("word/documenX.xml", b"<w:document/>")]), "word/documenX.xml", "word/document.xml");
        let archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
        assert_eq!(archive.len(), 1, "the zip library shows one of the two");
        assert!(!package_accepted(&bytes));
    }

    #[test]
    fn case_colliding_parts_are_refused() {
        for (first, second) in [
            ("word/document.xml", "word/Document.xml"),
            ("word/media/a.png", "WORD/MEDIA/A.PNG"),
            ("word/m\u{e9}dia.png", "word/M\u{c9}DIA.png"),
        ] {
            let bytes = package(&[(first, b"a"), (second, b"b")]);
            assert!(!package_accepted(&bytes), "{second}");
        }
        assert!(package_accepted(&package(&[("word/a.png", b"a"), ("word/b.png", b"b")])));
    }

    #[test]
    fn names_that_only_differ_in_the_separator_are_refused() {
        let bytes = package(&[("word/media/a.png", b"a"), ("word\\media\\a.png", b"b")]);
        assert!(!package_accepted(&bytes));
    }

    #[test]
    fn a_duplicate_does_not_hide_an_unaccepted_relationship() {
        let linked = external(IMAGE, ABS);
        let clean = rels_xml("");
        let parts: &[(&str, &[u8])] = &[
            ("word/_rels/document.xml.rels", linked.as_bytes()),
            ("word/_rels/documenX.xml.rels", clean.as_bytes()),
        ];
        let bytes = renamed(package(parts), "word/_rels/documenX.xml.rels", "word/_rels/document.xml.rels");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
        assert!(all_rels_allowed(&mut archive, &LIMITS), "the zip library reads only one of them");
        assert!(!package_accepted(&bytes));
    }

    fn package_with_comment(comment: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file(DOCUMENT.0, SimpleFileOptions::default()).unwrap();
        writer.write_all(DOCUMENT.1).unwrap();
        writer.set_raw_comment(comment.into());
        writer.finish().unwrap().into_inner()
    }

    const EMPTY_END: &[u8] = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

    #[test]
    fn a_tail_with_more_than_one_end_of_central_directory_is_refused() {
        let clean = package(&[DOCUMENT]);
        let mut twice = clean.clone();
        twice.extend_from_slice(&clean);
        let mut appended = clean.clone();
        appended.extend_from_slice(EMPTY_END);
        let locator = b"PK\x06\x07\0\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0";
        for (case, bytes) in [
            ("an end record in the comment", package_with_comment(EMPTY_END)),
            ("two packages one after the other", twice),
            ("an end record after the package", appended),
            ("a Zip64 locator in the comment", package_with_comment(locator)),
        ] {
            assert!(opens(&bytes), "{case}");
            assert!(!package_accepted(&bytes), "{case}");
        }
        assert!(package_accepted(&package_with_comment(b"a comment")));
    }

    // The end records some writers add to every package, whatever its size.
    fn with_zip64_end(bytes: Vec<u8>) -> Vec<u8> {
        let end = bytes.len() - EMPTY_END.len();
        assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
        let field = |at: usize, len: usize| {
            let mut value = [0u8; 8];
            value[..len].copy_from_slice(&bytes[end + at..end + at + len]);
            u64::from_le_bytes(value)
        };
        let (entries, size, offset) = (field(10, 2), field(12, 4), field(16, 4));
        let mut out = bytes[..end].to_vec();
        let zip64_end_at = out.len() as u64;
        out.extend_from_slice(b"PK\x06\x06");
        out.extend_from_slice(&44u64.to_le_bytes());
        out.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for value in [entries, entries, size, offset] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(b"PK\x06\x07\0\0\0\0");
        out.extend_from_slice(&zip64_end_at.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
        out.extend_from_slice(&[0xFF; 12]);
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn one_end_of_central_directory_is_accepted_with_or_without_zip64() {
        let clean = package(&[DOCUMENT, ("word/styles.xml", b"<w:styles/>")]);
        assert!(package_accepted(&clean));
        let zip64 = with_zip64_end(clean);
        assert!(opens(&zip64));
        assert!(package_accepted(&zip64));
    }

    #[test]
    fn the_directory_minizip_reads_is_the_one_the_zip_library_read() {
        let mut bytes = package(&[DOCUMENT]);
        let size_at = bytes.len() - EMPTY_END.len() + 12;
        bytes[size_at] += 1;
        assert!(opens(&bytes));
        assert!(!package_accepted(&bytes));
    }

    #[test]
    fn going_over_a_directory_limit_is_refused() {
        let bytes = package(&[DOCUMENT, ("word/styles.xml", b"<w:styles/>")]);
        let archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
        let start = archive.central_directory_start();
        let size = (bytes.len() - EMPTY_END.len()) as u64 - start;
        let within = |limits: DirectoryLimits| unique_names_within(&mut Cursor::new(bytes.clone()), start, &limits);
        assert!(within(DirectoryLimits { entries: 2, bytes: size }));
        assert!(!within(DirectoryLimits { entries: 1, bytes: size }));
        assert!(!within(DirectoryLimits { entries: 2, bytes: size - 1 }));
    }

    const WORD97_DOC: &[u8] = include_bytes!("../../tests/fixtures/word97.doc");

    fn followed_by(first: &[u8], then: &[u8]) -> Vec<u8> {
        let mut bytes = first.to_vec();
        bytes.extend_from_slice(then);
        bytes
    }

    #[test]
    fn a_package_after_a_cfb_start_is_checked_like_any_other() {
        let rels = external(IMAGE, ABS);
        let linked = package(&[("word/_rels/document.xml.rels", rels.as_bytes())]);
        let bytes = followed_by(WORD97_DOC, &linked);
        assert!(opens(&bytes));
        assert!(!package_accepted(&bytes));
    }

    #[test]
    fn a_cfb_with_the_end_of_a_package_the_zip_library_cannot_open_is_refused() {
        // An end record for a directory of one entry, with no entry before it.
        let mut end = EMPTY_END.to_vec();
        end[8] = 1;
        end[10] = 1;
        end[12] = 46;
        let bytes = followed_by(WORD97_DOC, &end);
        assert!(!opens(&bytes));
        assert!(!package_accepted(&bytes));
    }

    #[test]
    fn a_word_97_doc_is_accepted_with_or_without_an_embedded_package() {
        assert!(package_accepted(WORD97_DOC));
        let embedded = followed_by(WORD97_DOC, &package(&[("embedded.xml", b"<a/>")]));
        assert!(package_accepted(&embedded));
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
            let content = ContentSample::read(&document).unwrap();
            assert!(relationships_are_allowed(&document, &content), "{}", document.display());
        }
    }
}
