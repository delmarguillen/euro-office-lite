// The editor a package is for, as x2t tells it. The window opens the editor
// the extension names (_eoDocTypeForPath in bridge.js), and x2t converts a
// package as what it holds, whatever its extension: a workbook named .docx
// would reach the document editor as a spreadsheet.

use std::io::{Read, Seek};
use std::path::Path;

use crate::x2t_task::{detect_format, ContentSample};

// The editors of the app: 'word', 'cell' and 'slide' in bridge.js.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Family {
    Word,
    Cell,
    Slide,
}

// Whether x2t converts the file for the editor its extension opens. A file
// that holds no package, or a package x2t tells no family from, is left to
// x2t.
pub fn matches_extension(path: &Path, content: &ContentSample) -> bool {
    let Some(held) = family_held(path, content) else {
        return true;
    };
    family_of_extension(path) == Some(held)
}

// x2t numbers a format from the base of its family
// (core/Common/OfficeFileFormats.h at v10.0.0.169).
const DOCUMENT_FORMATS: i32 = 0x0040;
const PRESENTATION_FORMATS: i32 = 0x0080;
const SPREADSHEET_FORMATS: i32 = 0x0100;
// The bits of a format above its number inside the family.
const FAMILY_MASK: i32 = !0x003F;

fn family_of_extension(path: &Path) -> Option<Family> {
    match detect_format(path) & FAMILY_MASK {
        DOCUMENT_FORMATS => Some(Family::Word),
        SPREADSHEET_FORMATS => Some(Family::Cell),
        PRESENTATION_FORMATS => Some(Family::Slide),
        _ => None,
    }
}

fn family_held(path: &Path, content: &ContentSample) -> Option<Family> {
    if !content.may_hold_a_package() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    family_of_package(&mut archive)
}

const CONTENT_TYPES_PART: &str = "[Content_Types].xml";
const MIMETYPE_PART: &str = "mimetype";

// x2t reads mimetype only when [Content_Types].xml names no main part it
// looks for (isOfficeFile, core/Common/OfficeFileFormatChecker2.cpp:792-799
// at v10.0.0.169).
fn family_of_package<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Option<Family> {
    let content_types = read_part(archive, CONTENT_TYPES_PART).unwrap_or_default();
    if let Some(family) = first_family_named(&content_types, &OOXML_MAIN_TYPES) {
        return Some(family);
    }
    let mimetype = read_part(archive, MIMETYPE_PART)?;
    first_family_named(&mimetype, &ODF_MIMETYPES)
}

// The content types of a main part, in the order x2t looks for them anywhere
// in [Content_Types].xml (isOOXFormatFile, OfficeFileFormatChecker2.cpp:1027-1128).
const OOXML_MAIN_TYPES: [(&str, Family); 17] = [
    ("application/vnd.openxmlformats-officedocument.wordprocessingml.document.oform", Family::Word),
    ("application/vnd.openxmlformats-officedocument.wordprocessingml.document.docxf", Family::Word),
    ("application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml", Family::Word),
    ("application/vnd.ms-word.document.macroEnabled.main+xml", Family::Word),
    ("application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml", Family::Word),
    ("application/vnd.ms-word.template.macroEnabledTemplate.main+xml", Family::Word),
    ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml", Family::Cell),
    ("application/vnd.ms-excel.sheet.macroEnabled.main+xml", Family::Cell),
    ("application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml", Family::Cell),
    ("application/vnd.ms-excel.template.macroEnabled.main+xml", Family::Cell),
    ("application/vnd.ms-excel.sheet.binary.macroEnabled.main", Family::Cell),
    ("application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml", Family::Slide),
    ("application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml", Family::Slide),
    ("application/vnd.ms-powerpoint.slideshow.macroEnabled.main+xml", Family::Slide),
    ("application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml", Family::Slide),
    ("application/vnd.openxmlformats-officedocument.presentationml.template.main+xml", Family::Slide),
    ("application/vnd.ms-powerpoint.template.macroEnabled.main+xml", Family::Slide),
];

// What x2t looks for in the mimetype part, in its order: the templates first
// (isOpenOfficeFormatFile, OfficeFileFormatChecker2.cpp:1503-1535). It
// converts a drawing as a presentation and an EPUB as a text document.
const ODF_MIMETYPES: [(&str, Family); 11] = [
    ("application/vnd.oasis.opendocument.text-template", Family::Word),
    ("application/vnd.oasis.opendocument.spreadsheet-template", Family::Cell),
    ("application/vnd.oasis.opendocument.presentation-template", Family::Slide),
    ("application/vnd.oasis.opendocument.text", Family::Word),
    ("application/vnd.sun.xml.writer", Family::Word),
    ("application/vnd.oasis.opendocument.spreadsheet", Family::Cell),
    ("application/vnd.sun.xml.calc", Family::Cell),
    ("application/vnd.oasis.opendocument.presentation", Family::Slide),
    ("application/vnd.sun.xml.impress", Family::Slide),
    ("application/vnd.oasis.opendocument.graphics", Family::Slide),
    ("application/epub+zip", Family::Word),
];

// The family of the first name of `names` that `part` has anywhere in it.
fn first_family_named(part: &[u8], names: &[(&str, Family)]) -> Option<Family> {
    let (_, family) = names.iter().find(|(name, _)| contains(part, name.as_bytes()))?;
    Some(*family)
}

fn contains(bytes: &[u8], word: &[u8]) -> bool {
    bytes.windows(word.len()).any(|window| window == word)
}

// The two parts are a few kilobytes in a document; one past this size is not
// read, and tells no family.
const PART_LIMIT: u64 = 16 * 1024 * 1024;

fn read_part<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let part = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    part.take(PART_LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > PART_LIMIT {
        return None;
    }
    Some(bytes)
}
