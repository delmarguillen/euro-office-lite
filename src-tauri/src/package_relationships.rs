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

// A link to another workbook, and the template a document is attached to, are
// ordinary things for a document to have. Their relationships are accepted
// whatever their target, and only in the .rels part of the part that holds them
// while that part plays its usual role (LinkedFileRole). x2t keeps the target of
// the workbook ones as it is (OOX::Spreadsheet::CreateFile,
// OOXML/XlsxFormat/FileFactory_Spreadsheet.cpp) and does not read the template
// one (OOX::CreateFile, OOXML/DocxFormat/FileFactory.cpp, falls to
// UnknowTypeFile). The Microsoft ones are those of MS-OI29500 2.1.13, also in
// OOXML/DocxFormat/FileTypes.cpp.
const EXTERNAL_LINK_PATH_TYPES: [&str; 6] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/externalLinkPath",
    "http://schemas.microsoft.com/office/2006/relationships/xlExternalLinkPath/xlPathMissing",
    "http://schemas.microsoft.com/office/2006/relationships/xlExternalLinkPath/xlLibrary",
    "http://schemas.microsoft.com/office/2006/relationships/xlExternalLinkPath/xlStartup",
    "http://schemas.microsoft.com/office/2006/relationships/xlExternalLinkPath/xlAlternateStartup",
];
const ATTACHED_TEMPLATE_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/attachedTemplate",
];
// The relationships that give those parts their role.
const EXTERNAL_LINK_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/externalLink",
];
const SETTINGS_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/settings",
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

// Where a central directory ends, its size, where it says it starts and how
// many entries it says it has. `strays` are the other end records and Zip64
// locators in the tail, where they are in the file.
struct DirectoryEnd {
    end: u64,
    size: u64,
    offset: u64,
    entries: u64,
    strays: Vec<u64>,
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
    let Some(entries) = directory_entries(&directory) else {
        return false;
    };
    if !entries.iter().all(|entry| unicode_path_is_the_name(entry)) {
        return false;
    }
    let names: Vec<&[u8]> = entries.iter().map(|entry| entry_name(entry)).collect();
    if !names.iter().all(|name| is_kept_as_written(name)) {
        return false;
    }
    if names.len() as u64 != end.entries || !no_two_alike(&names) {
        return false;
    }
    end.strays.is_empty() || strays_are_inside_entries(reader, &entries, &end)
}

// The end of the central directory minizip finds: it looks back from the end of
// the file and takes the first end record it meets, and the first Zip64
// locator it meets the same way (unz64local_SearchCentralDir and
// unz64local_SearchCentralDir64 in minizip's unzip.c). That locator has to be
// right before that end record.
fn directory_end<R: Read + Seek>(reader: &mut R) -> Option<DirectoryEnd> {
    let len = reader.seek(SeekFrom::End(0)).ok()?;
    let tail_start = len.saturating_sub(ZIP_END_SEARCH_LEN as u64);
    let tail = read_at(reader, tail_start, (len - tail_start) as usize)?;
    let ends = positions(&tail, END_SIGNATURE);
    let locators = positions(&tail, ZIP64_LOCATOR_SIGNATURE);
    let (&end, other_ends) = ends.split_last()?;
    let mut directory = match locators.split_last() {
        None => zip32_end(&tail[end..], tail_start + end as u64)?,
        Some((&locator, _)) if locator + ZIP64_LOCATOR_LEN == end => zip64_end(reader, &tail[locator..end])?,
        Some(_) => return None,
    };
    let other_locators = locators.split_last().map_or(&[][..], |(_, others)| others);
    let strays = other_ends.iter().chain(other_locators);
    directory.strays = strays.map(|at| tail_start + *at as u64).collect();
    Some(directory)
}

fn zip32_end(record: &[u8], at: u64) -> Option<DirectoryEnd> {
    if record.len() < END_LEN {
        return None;
    }
    Some(DirectoryEnd {
        end: at,
        size: u32_at(record, 12) as u64,
        offset: u32_at(record, 16) as u64,
        entries: u16_at(record, 10) as u64,
        strays: Vec::new(),
    })
}

// minizip reads the Zip64 end record where the locator says, from the start of
// the file.
fn zip64_end<R: Read + Seek>(reader: &mut R, locator: &[u8]) -> Option<DirectoryEnd> {
    let at = u64_at(locator, 8);
    let record = read_at(reader, at, ZIP64_END_LEN)?;
    if !record.starts_with(ZIP64_END_SIGNATURE) {
        return None;
    }
    Some(DirectoryEnd {
        end: at,
        size: u64_at(&record, 40),
        offset: u64_at(&record, 48),
        entries: u64_at(&record, 32),
        strays: Vec::new(),
    })
}

// Every entry of a central directory, whole, in order. None when it holds
// anything but whole entries.
fn directory_entries(directory: &[u8]) -> Option<Vec<&[u8]>> {
    let mut entries = Vec::new();
    let mut rest = directory;
    while !rest.is_empty() {
        if rest.len() < ENTRY_LEN || !rest.starts_with(ENTRY_SIGNATURE) {
            return None;
        }
        let entry_len = ENTRY_LEN + u16_at(rest, 28) as usize + u16_at(rest, 30) as usize + u16_at(rest, 32) as usize;
        if rest.len() < entry_len {
            return None;
        }
        entries.push(&rest[..entry_len]);
        rest = &rest[entry_len..];
    }
    Some(entries)
}

fn entry_name(entry: &[u8]) -> &[u8] {
    &entry[ENTRY_LEN..ENTRY_LEN + u16_at(entry, 28) as usize]
}

// Whether every file system x2t runs on creates a file of this very name.
// Windows drops the "." and " " that end a segment of a path, and writes what
// follows a ":" to a stream of the file ("name::$DATA" being the file itself).
// minizip gives the name to x2t as a C string, which ends at its first NUL. So
// none of these is accepted, on any system, nor any other control character.
fn is_kept_as_written(name: &[u8]) -> bool {
    if name.iter().any(|byte| *byte == b':' || byte.is_ascii_control()) {
        return false;
    }
    let mut segments = name.split(|byte| *byte == b'/' || *byte == b'\\');
    segments.all(|segment| !segment.ends_with(b".") && !segment.ends_with(b" "))
}

const UNICODE_PATH_EXTRA_ID: u16 = 0x7075;

// x2t extracts each entry under the name stored in its central directory entry
// (unzGetCurrentFileInfo in do_extract_currentfile, OfficeUtils/src/ZipUtilsCP.cpp),
// and minizip reads no Info-ZIP Unicode Path extra field (0x7075). The zip
// library takes the name in that field instead when its CRC-32 matches the
// stored name (parse_single_extra_field in zip 2.4.2). So the name in such a
// field has to be the stored name, byte for byte. The field holds a version
// byte, that CRC-32 and the name.
fn unicode_path_is_the_name(entry: &[u8]) -> bool {
    let name = entry_name(entry);
    let mut unicode_paths = extra_blocks(entry).into_iter().filter(|(id, _)| *id == UNICODE_PATH_EXTRA_ID);
    unicode_paths.all(|(_, block)| block.get(5..) == Some(name))
}

// A document can hold another package, stored as it is, with an end record of
// its own. An end record or Zip64 locator found inside an entry is that entry's
// data: minizip and the zip library both look back from the end of the file and
// meet the one after the central directory first. Any other one is not
// accepted, nor an entry that does not fit between the start of the file and
// the central directory without overlapping another.
fn strays_are_inside_entries<R: Read + Seek>(reader: &mut R, entries: &[&[u8]], end: &DirectoryEnd) -> bool {
    let directory_start = end.end - end.size;
    // minizip adds what comes before the package to every offset
    // (byte_before_the_zipfile in unzOpenInternal).
    let Some(before) = directory_start.checked_sub(end.offset) else {
        return false;
    };
    let mut ranges = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(range) = entry_range(reader, entry, before) else {
            return false;
        };
        ranges.push(range);
    }
    ranges.sort_by_key(|range| range.start);
    if !ranges_fit_before(&ranges, directory_start) {
        return false;
    }
    end.strays.iter().all(|at| ranges.iter().any(|range| range.start <= *at && *at + 4 <= range.end))
}

fn ranges_fit_before(sorted: &[std::ops::Range<u64>], limit: u64) -> bool {
    let overlapping = sorted.windows(2).any(|pair| pair[0].end > pair[1].start);
    let past_limit = sorted.last().is_some_and(|last| last.end > limit);
    !overlapping && !past_limit
}

const LOCAL_SIGNATURE: &[u8] = b"PK\x03\x04";
const DESCRIPTOR_SIGNATURE: &[u8] = b"PK\x07\x08";
const LOCAL_LEN: usize = 30;
const ZIP64_EXTRA_ID: u16 = 0x0001;
const IN_ZIP64: u32 = 0xFFFF_FFFF;
const HAS_DESCRIPTOR: u16 = 1 << 3;

// Where an entry lies in the file: from its local header to the end of its
// data, and of its data descriptor when it has one. The sizes are those of the
// central directory, or of its Zip64 extra field, as minizip reads them
// (unz64local_GetCurrentFileInfoInternal); the data starts after the name and
// extra field of the local header (unz64local_CheckCurrentFileCoherencyHeader).
fn entry_range<R: Read + Seek>(reader: &mut R, entry: &[u8], before: u64) -> Option<std::ops::Range<u64>> {
    let sizes = entry_sizes(entry)?;
    let start = before.checked_add(sizes.local_offset)?;
    let header = read_at(reader, start, LOCAL_LEN)?;
    if !header.starts_with(LOCAL_SIGNATURE) {
        return None;
    }
    let header_len = (LOCAL_LEN + u16_at(&header, 26) as usize + u16_at(&header, 28) as usize) as u64;
    let data_end = start.checked_add(header_len)?.checked_add(sizes.compressed)?;
    if u16_at(entry, 8) & HAS_DESCRIPTOR == 0 {
        return Some(start..data_end);
    }
    let end = data_end.checked_add(descriptor_len(reader, data_end, sizes.zip64)?)?;
    Some(start..end)
}

struct EntrySizes {
    compressed: u64,
    local_offset: u64,
    zip64: bool,
}

// The Zip64 extra field holds, in this order, each of the uncompressed size,
// the compressed size and the local header offset whose field in the entry is
// 0xFFFFFFFF.
fn entry_sizes(entry: &[u8]) -> Option<EntrySizes> {
    let uncompressed = u32_at(entry, 24);
    let compressed = u32_at(entry, 20);
    let local_offset = u32_at(entry, 42);
    let zip64 = uncompressed == IN_ZIP64 || compressed == IN_ZIP64;
    if !zip64 && local_offset != IN_ZIP64 {
        return Some(EntrySizes { compressed: compressed as u64, local_offset: local_offset as u64, zip64 });
    }
    let mut values = zip64_extra(entry)?.chunks_exact(8).map(|value| u64_at(value, 0));
    if uncompressed == IN_ZIP64 {
        values.next()?;
    }
    let compressed = if compressed == IN_ZIP64 { values.next()? } else { compressed as u64 };
    let local_offset = if local_offset == IN_ZIP64 { values.next()? } else { local_offset as u64 };
    Some(EntrySizes { compressed, local_offset, zip64 })
}

fn zip64_extra(entry: &[u8]) -> Option<&[u8]> {
    let mut blocks = extra_blocks(entry).into_iter();
    blocks.find(|(id, _)| *id == ZIP64_EXTRA_ID).map(|(_, block)| block)
}

// The blocks of an entry's extra field, with their ids, up to the first one
// that does not fit in it: the zip library does not apply that one either.
fn extra_blocks(entry: &[u8]) -> Vec<(u16, &[u8])> {
    let name_len = u16_at(entry, 28) as usize;
    let extra_len = u16_at(entry, 30) as usize;
    let mut rest = &entry[ENTRY_LEN + name_len..ENTRY_LEN + name_len + extra_len];
    let mut blocks = Vec::new();
    while rest.len() >= 4 {
        let id = u16_at(rest, 0);
        let len = u16_at(rest, 2) as usize;
        let Some(block) = rest.get(4..4 + len) else {
            break;
        };
        blocks.push((id, block));
        rest = &rest[4 + len..];
    }
    blocks
}

// A data descriptor: an optional signature, the CRC and both sizes, of 8 bytes
// each for a Zip64 entry.
fn descriptor_len<R: Read + Seek>(reader: &mut R, at: u64, zip64: bool) -> Option<u64> {
    let signature = read_at(reader, at, DESCRIPTOR_SIGNATURE.len())?;
    let signature_len = if signature == DESCRIPTOR_SIGNATURE { 4 } else { 0 };
    let size_len = if zip64 { 8 } else { 4 };
    Some(signature_len + 4 + 2 * size_len)
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
    let package = Package { entries: archive.file_names().collect(), links: internal_links(&parts) };
    parts.iter().all(|part| {
        part.relationships.iter().all(|relationship| relationship_allowed(relationship, part, &package))
    })
}

// What a relationship is judged against: the entries of the package and every
// internal relationship in it.
struct Package<'a> {
    entries: HashSet<&'a str>,
    links: Vec<Link<'a>>,
}

// An internal relationship and a part it reaches, its name in lower case as a
// case-insensitive file system sees it.
struct Link<'a> {
    from: &'a str,
    kinds: &'a [String],
    to: String,
}

// Each target counts from the folder of its part and from the package root,
// where presentations look too (target_stays_in_package).
fn internal_links(parts: &[RelsPart]) -> Vec<Link<'_>> {
    let mut links = Vec::new();
    for part in parts {
        let folder = source_folder(&part.name);
        for relationship in part.relationships.iter().filter(|relationship| !relationship.x2t_external) {
            for target in &relationship.targets {
                for base in [folder, ""] {
                    let Some(to) = resolved_target(base, target) else {
                        continue;
                    };
                    links.push(Link { from: &part.name, kinds: &relationship.kinds, to: to.to_lowercase() });
                }
            }
        }
    }
    links
}

// A part whose .rels may hold the relationships of a few Types to another file,
// and how it is reached when it plays its usual role.
struct LinkedFileRole {
    part: String,
    kinds: &'static [&'static str],
    reached_from: &'static str,
    reached_as: &'static [&'static str],
    // Excel keeps a link part the workbook no longer uses, and nothing reads it.
    may_be_unreached: bool,
}

// The role of the part a .rels belongs to, by the exact name of the .rels:
// xl/externalLinks/_rels/externalLinkN.xml.rels, N in decimal, or
// word/_rels/settings.xml.rels.
fn linked_file_role(rels_name: &str) -> Option<LinkedFileRole> {
    if rels_name == "word/_rels/settings.xml.rels" {
        return Some(LinkedFileRole {
            part: "word/settings.xml".to_string(),
            kinds: &ATTACHED_TEMPLATE_TYPES,
            reached_from: "word/_rels/document.xml.rels",
            reached_as: &SETTINGS_TYPES,
            may_be_unreached: false,
        });
    }
    let number = rels_name.strip_prefix("xl/externalLinks/_rels/externalLink")?.strip_suffix(".xml.rels")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(LinkedFileRole {
        part: format!("xl/externalLinks/externalLink{number}.xml"),
        kinds: &EXTERNAL_LINK_PATH_TYPES,
        reached_from: "xl/_rels/workbook.xml.rels",
        reached_as: &EXTERNAL_LINK_TYPES,
        may_be_unreached: true,
    })
}

fn is_accepted_link_to_another_file(relationship: &Relationship, rels_name: &str, links: &[Link]) -> bool {
    let Some(role) = linked_file_role(rels_name) else {
        return false;
    };
    let kinds = &relationship.kinds;
    !kinds.is_empty() && kinds.iter().all(|kind| role.kinds.contains(&kind.as_str())) && plays_its_usual_role(&role, links)
}

// The part is reached, or may go unreached, and whatever reaches it does so only
// with one of its usual Types from its usual place: not from another .rels, nor
// as the main document of _rels/.rels.
fn plays_its_usual_role(role: &LinkedFileRole, links: &[Link]) -> bool {
    let part = role.part.to_lowercase();
    let mut reaching = links.iter().filter(|link| link.to == part).peekable();
    let is_usual = |link: &Link| {
        link.from == role.reached_from
            && !link.kinds.is_empty()
            && link.kinds.iter().all(|kind| role.reached_as.contains(&kind.as_str()))
    };
    (reaching.peek().is_some() || role.may_be_unreached) && reaching.all(|link| is_usual(link))
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

fn relationship_allowed(relationship: &Relationship, part: &RelsPart, package: &Package) -> bool {
    let Relationship { kinds, targets, modes, x2t_external } = relationship;
    if kinds.iter().any(|kind| is_alt_chunk(kind)) {
        return false;
    }
    let folder = source_folder(&part.name);
    let in_package = |target: &String| target_stays_in_package(folder, target, &package.entries);
    if !x2t_external && !targets.iter().all(in_package) {
        return false;
    }
    if modes.iter().all(|mode| mode == "Internal") {
        return true;
    }
    if !kinds.is_empty() && kinds.iter().all(|kind| HYPERLINK_TYPES.contains(&kind.as_str())) {
        return true;
    }
    if is_accepted_link_to_another_file(relationship, &part.name, &package.links) {
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
pub(crate) mod tests {
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

    const CUSTOM_XML: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";

    // x2t joins the targets of _rels/.rels to the folder it extracts the package
    // into, and CPath::Normalize lets a ".." take that folder off.
    #[test]
    fn a_package_root_target_going_up_one_folder_is_refused() {
        let root = one_relationship(CUSTOM_XML, "../customXml/item1.xml", "");
        let item: (&str, String) = ("customXml/item1.xml", "<a/>".to_string());
        assert!(!allowed(package_of(&[("_rels/.rels", root), item.clone()])));
        let from_slide = internal("../media/image1.png", "");
        assert!(allowed(package(&[(SLIDE_RELS, from_slide.as_bytes()), SLIDE_PICTURE])));
        let from_folder = one_relationship(CUSTOM_XML, "../../customXml/item1.xml", "");
        assert!(!allowed(package_of(&[("xl/_rels/workbook.xml.rels", from_folder), item])));
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

    fn one_relationship(kind: &str, target: &str, mode: &str) -> String {
        rels_xml(&format!("<Relationship Id=\"rId1\" Type=\"{kind}\" Target=\"{target}\"{mode}/>"))
    }

    fn package_of(parts: &[(&str, String)]) -> Vec<u8> {
        let parts: Vec<(&str, &[u8])> = parts.iter().map(|(name, xml)| (*name, xml.as_bytes())).collect();
        package(&parts)
    }

    const EXTERNAL_LINK: &str = EXTERNAL_LINK_TYPES[0];
    const SETTINGS: &str = SETTINGS_TYPES[0];
    const EXTERNAL_LINK_PATH: &str = EXTERNAL_LINK_PATH_TYPES[0];
    const ATTACHED_TEMPLATE: &str = ATTACHED_TEMPLATE_TYPES[0];
    const WORKBOOK_RELS: &str = "xl/_rels/workbook.xml.rels";
    const EXTERNAL_LINK_RELS: &str = "xl/externalLinks/_rels/externalLink1.xml.rels";
    const SETTINGS_RELS: &str = "word/_rels/settings.xml.rels";
    const DOCUMENT_RELS: &str = "word/_rels/document.xml.rels";
    const OTHER_BOOK: &str = "../Budget/Book2.xlsx";
    const TEMPLATE: &str = "Macintosh HD:Users:me:Templates:Report.dotx";
    const WORKSHEET: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
    const OFFICE_DOCUMENT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

    // A workbook with a link to another one in externalLink1, and `more` parts.
    fn linked_workbook(kind: &str, target: &str, more: &[(&str, String)]) -> Vec<u8> {
        let mut parts = vec![
            (WORKBOOK_RELS, one_relationship(EXTERNAL_LINK, "externalLinks/externalLink1.xml", "")),
            ("xl/externalLinks/externalLink1.xml", "<externalLink/>".to_string()),
            (EXTERNAL_LINK_RELS, external(kind, target)),
        ];
        parts.extend(more.iter().cloned());
        package_of(&parts)
    }

    // A document attached to a template, and `more` parts.
    fn attached_document(kind: &str, more: &[(&str, String)]) -> Vec<u8> {
        let mut parts = vec![
            (DOCUMENT_RELS, one_relationship(SETTINGS, "settings.xml", "")),
            ("word/settings.xml", "<w:settings/>".to_string()),
            (SETTINGS_RELS, external(kind, TEMPLATE)),
        ];
        parts.extend(more.iter().cloned());
        package_of(&parts)
    }

    #[test]
    fn links_to_other_workbooks_are_accepted_with_each_type() {
        for kind in EXTERNAL_LINK_PATH_TYPES {
            for target in [OTHER_BOOK, "Book2.xlsx", "/Users/me/Book2.xlsx", "C:\\Data\\Book2.xlsx"] {
                assert!(allowed(linked_workbook(kind, target, &[])), "{kind} {target}");
            }
        }
        let strict = one_relationship(EXTERNAL_LINK_TYPES[1], "/xl/externalLinks/externalLink12.xml", "");
        let twelve = package_of(&[
            (WORKBOOK_RELS, strict),
            ("xl/externalLinks/_rels/externalLink12.xml.rels", external(EXTERNAL_LINK_PATH, OTHER_BOOK)),
        ]);
        assert!(allowed(twelve));
    }

    #[test]
    fn an_attached_template_is_accepted_with_each_type() {
        for kind in ATTACHED_TEMPLATE_TYPES {
            assert!(allowed(attached_document(kind, &[])), "{kind}");
        }
        let strict = package_of(&[
            (DOCUMENT_RELS, one_relationship(SETTINGS_TYPES[1], "settings.xml", "")),
            (SETTINGS_RELS, external(ATTACHED_TEMPLATE, "C:\\Templates\\Report.dotx")),
        ]);
        assert!(allowed(strict));
    }

    #[test]
    fn the_same_types_in_another_part_are_refused() {
        let slide = package_of(&[("ppt/slides/_rels/slide1.xml.rels", external(EXTERNAL_LINK_PATH, OTHER_BOOK))]);
        assert!(!allowed(slide));
        let document = package_of(&[(DOCUMENT_RELS, external(ATTACHED_TEMPLATE, TEMPLATE))]);
        assert!(!allowed(document));
        let pivot = package_of(&[("xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels", external(EXTERNAL_LINK_PATH, OTHER_BOOK))]);
        assert!(!allowed(pivot));
        for (rels, part) in [
            ("xl/externalLinks/_rels/ExternalLink1.xml.rels", "ExternalLink1.xml"),
            ("xl/externallinks/_rels/externalLink1.xml.rels", "externalLink1.xml"),
            ("xl/externalLinks/_rels/externalLinkA.xml.rels", "externalLinkA.xml"),
            ("xl/externalLinks/_rels/externalLink.xml.rels", "externalLink.xml"),
            ("xl/externalLinks/_rels/externalLink1.XML.rels", "externalLink1.XML"),
            ("a/xl/externalLinks/_rels/externalLink1.xml.rels", "externalLink1.xml"),
        ] {
            let bytes = package_of(&[
                (WORKBOOK_RELS, one_relationship(EXTERNAL_LINK, &format!("externalLinks/{part}"), "")),
                (rels, external(EXTERNAL_LINK_PATH, OTHER_BOOK)),
            ]);
            assert!(!allowed(bytes), "{rels}");
        }
        assert!(!allowed(linked_workbook(ATTACHED_TEMPLATE, TEMPLATE, &[])));
        assert!(!allowed(attached_document(EXTERNAL_LINK_PATH, &[])));
    }

    #[test]
    fn a_part_reached_from_another_rels_or_as_the_main_document_loses_its_exemption() {
        for target in [
            "../externalLinks/externalLink1.xml",
            "../externalLinks/./externalLink1.xml",
            "../EXTERNALLINKS/externalLink1.xml",
            "/xl/externalLinks/externalLink1.xml",
            "xl/externalLinks/externalLink1.xml",
        ] {
            let sheet = ("xl/worksheets/_rels/sheet1.xml.rels", one_relationship(WORKSHEET, target, ""));
            assert!(!allowed(linked_workbook(EXTERNAL_LINK_PATH, OTHER_BOOK, &[sheet])), "{target}");
        }
        let main = ("_rels/.rels", one_relationship(OFFICE_DOCUMENT, "xl/externalLinks/externalLink1.xml", ""));
        assert!(!allowed(linked_workbook(EXTERNAL_LINK_PATH, OTHER_BOOK, &[main])));
        let main = ("_rels/.rels", one_relationship(OFFICE_DOCUMENT, "/word/settings.xml", ""));
        assert!(!allowed(attached_document(ATTACHED_TEMPLATE, &[main])));
        let footer = ("word/_rels/footer1.xml.rels", one_relationship(SETTINGS, "settings.xml", ""));
        assert!(!allowed(attached_document(ATTACHED_TEMPLATE, &[footer])));
    }

    #[test]
    fn a_part_reached_otherwise_than_with_its_usual_type_loses_its_exemption() {
        let both = rels_xml(&format!(
            "<Relationship Id=\"rId1\" Type=\"{EXTERNAL_LINK}\" Target=\"externalLinks/externalLink1.xml\"/><Relationship Id=\"rId2\" Type=\"{WORKSHEET}\" Target=\"externalLinks/externalLink1.xml\"/>"
        ));
        let other_type = one_relationship(WORKSHEET, "externalLinks/externalLink1.xml", "");
        for workbook in [both, other_type] {
            let bytes = package_of(&[(WORKBOOK_RELS, workbook), (EXTERNAL_LINK_RELS, external(EXTERNAL_LINK_PATH, OTHER_BOOK))]);
            assert!(!allowed(bytes));
        }
    }

    #[test]
    fn an_external_link_part_no_relationship_reaches_keeps_its_exemption() {
        let unreached = package_of(&[
            ("xl/externalLinks/externalLink0.xml", "<externalLink/>".to_string()),
            ("xl/externalLinks/_rels/externalLink0.xml.rels", external(EXTERNAL_LINK_PATH, OTHER_BOOK)),
        ]);
        assert!(allowed(unreached));
    }

    #[test]
    fn an_unreached_external_link_part_is_refused_once_another_part_reaches_it() {
        let parts = [
            ("xl/externalLinks/externalLink0.xml", "<externalLink/>".to_string()),
            ("xl/externalLinks/_rels/externalLink0.xml.rels", external(EXTERNAL_LINK_PATH, OTHER_BOOK)),
        ];
        let sheet = one_relationship(EXTERNAL_LINK, "../externalLinks/externalLink0.xml", "");
        let other_type = one_relationship(WORKSHEET, "externalLinks/externalLink0.xml", "");
        for (rels, xml) in [("xl/worksheets/_rels/sheet1.xml.rels", sheet), (WORKBOOK_RELS, other_type)] {
            let mut with_link = parts.to_vec();
            with_link.push((rels, xml));
            assert!(!allowed(package_of(&with_link)), "{rels}");
        }
    }

    #[test]
    fn an_unreached_settings_part_keeps_refusing_its_attached_template() {
        let unreached = package_of(&[
            ("word/settings.xml", "<w:settings/>".to_string()),
            (SETTINGS_RELS, external(ATTACHED_TEMPLATE, TEMPLATE)),
        ]);
        assert!(!allowed(unreached));
    }

    #[test]
    fn a_linked_picture_next_to_an_accepted_link_is_refused() {
        let picture = format!(
            "<Relationship Id=\"rId2\" Type=\"{IMAGE}\" Target=\"{ABS}\" TargetMode=\"External\"/>"
        );
        let link = format!(
            "<Relationship Id=\"rId1\" Type=\"{EXTERNAL_LINK_PATH}\" Target=\"{OTHER_BOOK}\" TargetMode=\"External\"/>"
        );
        let workbook = package_of(&[
            (WORKBOOK_RELS, one_relationship(EXTERNAL_LINK, "externalLinks/externalLink1.xml", "")),
            (EXTERNAL_LINK_RELS, rels_xml(&format!("{link}{picture}"))),
        ]);
        assert!(!allowed(workbook));
        let template = format!(
            "<Relationship Id=\"rId1\" Type=\"{ATTACHED_TEMPLATE}\" Target=\"{TEMPLATE}\" TargetMode=\"External\"/>"
        );
        let document = package_of(&[
            (DOCUMENT_RELS, one_relationship(SETTINGS, "settings.xml", "")),
            (SETTINGS_RELS, rels_xml(&format!("{template}{picture}"))),
        ]);
        assert!(!allowed(document));
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

    // A workbook as Office stores one inside a document that holds a chart.
    fn workbook() -> Vec<u8> {
        package(&[("[Content_Types].xml", b"<Types/>"), ("xl/workbook.xml", b"<workbook/>")])
    }

    // A package whose `stored` parts are kept as they are, the others deflated.
    fn package_storing(parts: &[(&str, &[u8])], stored: &[&str]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in parts {
            let method = if stored.contains(name) { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
            writer.start_file(*name, SimpleFileOptions::default().compression_method(method)).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    const DOCX_WORKBOOK: &str = "word/embeddings/Microsoft_Excel_Worksheet.xlsx";
    const PPTX_WORKBOOKS: [&str; 3] = [
        "ppt/embeddings/Microsoft_Excel_Worksheet1.xlsx",
        "ppt/embeddings/Microsoft_Excel_Worksheet2.xlsx",
        "ppt/embeddings/Microsoft_Excel_Worksheet3.xlsx",
    ];

    fn docx_with_a_workbook() -> Vec<u8> {
        let book = workbook();
        package_storing(&[DOCUMENT, (DOCX_WORKBOOK, &book), ("word/styles.xml", b"<w:styles/>")], &[DOCX_WORKBOOK])
    }

    fn pptx_with_workbooks(count: usize) -> Vec<u8> {
        let book = workbook();
        let picture = internal("../media/image1.png", "");
        let mut parts: Vec<(&str, &[u8])> = vec![("ppt/presentation.xml", b"<p:presentation/>")];
        parts.extend(PPTX_WORKBOOKS[..count].iter().map(|name| (*name, book.as_slice())));
        parts.extend([(SLIDE_RELS, picture.as_bytes()), SLIDE_PICTURE]);
        package_storing(&parts, &PPTX_WORKBOOKS)
    }

    #[test]
    fn documents_holding_stored_workbooks_with_their_own_end_record_are_accepted() {
        for (case, bytes) in [
            ("a docx with one workbook", docx_with_a_workbook()),
            ("a pptx with one workbook", pptx_with_workbooks(1)),
            ("a pptx with three workbooks", pptx_with_workbooks(3)),
            ("a pptx with three workbooks and Zip64 end records", with_zip64_end(pptx_with_workbooks(3))),
            ("a docx whose workbook has a data descriptor", with_descriptor(docx_with_a_workbook(), DOCX_WORKBOOK)),
        ] {
            assert!(occurrences(&bytes, END_SIGNATURE).len() > 1, "{case}");
            assert!(package_accepted(&bytes), "{case}");
        }
    }

    // The central directory entry of `name`, whose name comes after its local
    // header's.
    fn directory_entry_of(bytes: &[u8], name: &str) -> usize {
        let found = occurrences(bytes, name.as_bytes());
        found[found.len() - 1] - ENTRY_LEN
    }

    // The package with a data descriptor after the data of `name`, the offsets
    // of what follows moved to make room for it.
    fn with_descriptor(bytes: Vec<u8>, name: &str) -> Vec<u8> {
        let entry = directory_entry_of(&bytes, name);
        let local = u32_at(&bytes, entry + 42) as usize;
        let header_len = LOCAL_LEN + u16_at(&bytes, local + 26) as usize + u16_at(&bytes, local + 28) as usize;
        let data_end = local + header_len + u32_at(&bytes, entry + 20) as usize;
        let mut descriptor = DESCRIPTOR_SIGNATURE.to_vec();
        descriptor.extend_from_slice(&bytes[entry + 16..entry + 28]);
        let shift = descriptor.len() as u32;
        let mut out = bytes.clone();
        out[local + 6] |= HAS_DESCRIPTOR as u8;
        out[entry + 8] |= HAS_DESCRIPTOR as u8;
        let directory = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap().central_directory_start() as usize;
        let entries = directory_entries(&bytes[directory..bytes.len() - END_LEN]).unwrap();
        let mut at = directory;
        for other in entries {
            if u32_at(other, 42) as usize > local {
                out = with_u32(out, at + 42, u32_at(other, 42) + shift);
            }
            at += other.len();
        }
        let end_offset = bytes.len() - END_LEN + 16;
        out = with_u32(out, end_offset, u32_at(&bytes, end_offset) + shift);
        out.splice(data_end..data_end, descriptor);
        out
    }

    fn with_u32(mut bytes: Vec<u8>, at: usize, value: u32) -> Vec<u8> {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        bytes
    }

    #[test]
    fn an_end_record_outside_every_entry_is_refused() {
        let docx = docx_with_a_workbook();
        let entry = directory_entry_of(&docx, DOCX_WORKBOOK);
        let size = u32_at(&docx, entry + 20);
        // The workbook's own end record is left between its entry and the next.
        let shortened = with_u32(docx.clone(), entry + 20, size - END_LEN as u32);
        let mut appended = docx.clone();
        appended.extend_from_slice(EMPTY_END);
        let book = workbook();
        let named = "word/PK\u{5}\u{6}.xml";
        let in_directory = package_storing(&[DOCUMENT, (DOCX_WORKBOOK, &book), (named, b"<a/>")], &[DOCX_WORKBOOK]);
        let mut in_comment = zip::ZipWriter::new(Cursor::new(Vec::new()));
        in_comment.start_file(DOCX_WORKBOOK, SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
        in_comment.write_all(&book).unwrap();
        in_comment.set_raw_comment(EMPTY_END.into());
        let in_comment = in_comment.finish().unwrap().into_inner();
        for (case, bytes) in [
            ("between two entries", shortened),
            ("in the central directory", in_directory),
            ("in the comment", in_comment),
            ("after the package", appended),
        ] {
            assert!(opens(&bytes), "{case}");
            assert!(!package_accepted(&bytes), "{case}");
        }
    }

    #[test]
    fn an_entry_that_cannot_lie_where_it_says_is_refused() {
        let docx = docx_with_a_workbook();
        let entry = directory_entry_of(&docx, DOCX_WORKBOOK);
        let size = u32_at(&docx, entry + 20);
        let len = docx.len() as u32;
        for (case, bytes) in [
            ("over the next entry", with_u32(docx.clone(), entry + 20, size + 8)),
            ("over the central directory", with_u32(docx.clone(), entry + 20, len)),
            ("past the end of the file", with_u32(docx.clone(), entry + 42, len)),
        ] {
            assert!(opens(&bytes), "{case}");
            assert!(!package_accepted(&bytes), "{case}");
        }
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

    // The CRC-32 of the zip format.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    // A package whose part `named` has an Info-ZIP Unicode Path extra field
    // (0x7075) holding `unicode`, with the CRC-32 of the stored name. The zip
    // library writes no such field, so it is written under another id first.
    pub(crate) fn package_with_unicode_path(parts: &[(&str, &[u8])], named: &str, unicode: &str) -> Vec<u8> {
        const PLACEHOLDER_ID: u16 = 0x6666;
        let mut field = vec![1];
        field.extend_from_slice(&crc32(named.as_bytes()).to_le_bytes());
        field.extend_from_slice(unicode.as_bytes());
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in parts {
            let mut options = zip::write::FullFileOptions::default();
            if *name == named {
                options.add_extra_data(PLACEHOLDER_ID, field.clone().into_boxed_slice(), false).unwrap();
            }
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        let mut bytes = writer.finish().unwrap().into_inner();
        let mut placed = PLACEHOLDER_ID.to_le_bytes().to_vec();
        placed.extend_from_slice(&(field.len() as u16).to_le_bytes());
        placed.extend_from_slice(&field);
        let found = occurrences(&bytes, &placed);
        assert_eq!(found.len(), 2, "a local header and a directory entry");
        for at in found {
            bytes[at..at + 2].copy_from_slice(&UNICODE_PATH_EXTRA_ID.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn each_part_is_read_by_the_name_the_converter_reads() {
        let rels = external(IMAGE, ABS);
        let parts: &[(&str, &[u8])] = &[DOCUMENT, (DOCUMENT_RELS, rels.as_bytes())];
        let bytes = package_with_unicode_path(parts, DOCUMENT_RELS, "word/_rels/ignored.bin");
        let archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
        assert_eq!(archive.name_for_index(1), Some("word/_rels/ignored.bin"), "the zip library reads the field");
        assert!(!package_accepted(&bytes));
    }

    #[test]
    fn a_unicode_path_with_the_stored_name_is_accepted() {
        let rels = one_relationship(IMAGE, "media/a.png", "");
        let parts: &[(&str, &[u8])] = &[DOCUMENT, (DOCUMENT_RELS, rels.as_bytes()), ("word/media/a.png", b"png")];
        assert!(package_accepted(&package_with_unicode_path(parts, DOCUMENT_RELS, DOCUMENT_RELS)));
        let picture = "word/media/im\u{e1}gen.png";
        let parts: &[(&str, &[u8])] = &[DOCUMENT, (picture, b"png")];
        assert!(package_accepted(&package_with_unicode_path(parts, picture, picture)));
    }

    #[test]
    fn names_that_are_not_ascii_are_accepted_with_or_without_the_utf8_flag() {
        let utf8 = package(&[DOCUMENT, ("word/media/im\u{e1}gen.png", b"png")]);
        let flags = directory_entry_of(&utf8, "word/media/im\u{e1}gen.png") + 8;
        assert_eq!(u16_at(&utf8, flags) & (1 << 11), 1 << 11);
        assert!(package_accepted(&utf8));
        let mut cp437 = package(&[DOCUMENT, ("word/media/imXgen.png", b"png")]);
        for at in occurrences(&cp437, b"imXgen") {
            cp437[at + 2] = 0xA0;
        }
        let archive = zip::ZipArchive::new(Cursor::new(cp437.clone())).unwrap();
        assert!(archive.file_names().any(|name| name == "word/media/im\u{e1}gen.png"), "read as CP437");
        assert!(package_accepted(&cp437));
    }

    #[test]
    fn names_the_file_system_does_not_keep_as_written_are_refused() {
        // From word/_rels the target leaves the package; from word/_rels. it is x.png.
        let rels = one_relationship(IMAGE, "../../x.png", "");
        let parts: &[(&str, &[u8])] = &[DOCUMENT, (DOCUMENT_RELS, rels.as_bytes()), ("x.png", b"png")];
        assert!(!package_accepted(&package(parts)));
        for name in [
            "word/_rels/document.xml.rels.",
            "word/_rels/document.xml.rels ",
            "word/_rels./document.xml.rels",
            "word/_rels/document.xml.rels::$DATA",
            "word/_rels/document.xml.rels\u{0}",
            "word/_rels/document.xml.rels\u{1}",
        ] {
            let parts: &[(&str, &[u8])] = &[DOCUMENT, (name, rels.as_bytes()), ("x.png", b"png")];
            assert!(!package_accepted(&package(parts)), "{name:?}");
        }
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
