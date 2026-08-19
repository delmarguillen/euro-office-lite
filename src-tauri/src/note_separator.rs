// Removing the footnote separator line (#42).
//
// The editor has no setting for it: the horizontal rule above the footnotes is
// drawn from the special footnote the engine keeps in word/footnotes.xml, and
// nothing in the UI reaches it. What the engine writes is
//
//   <w:footnote w:type="separator" w:id="-1"><w:p><w:pPr>...</w:pPr>
//     <w:r></w:r><w:r><w:separator/></w:r><w:r></w:r></w:p></w:footnote>
//
// followed by the continuationSeparator footnote (w:id="0"). Dropping the run
// that holds <w:separator/> leaves an empty separator, which is exactly what a
// document without the line looks like. Validated experimentally: a document
// that HAS real footnotes keeps that empty separator across a save, while one
// with no notes at all has the whole footnotes part regenerated, so the edit
// only means anything once the document has notes.
//
// Only that one run goes. The continuationSeparator is left alone on purpose:
// it draws the rule of a footnote continued on the next page, which is a
// different line and a different decision.
//
// A zip cannot be edited in place, so the whole package is rewritten: every
// entry copied across and word/footnotes.xml replaced. The new package is built
// beside the document and moved onto it with a rename, so a failure halfway
// through leaves the user's file untouched rather than truncated.

use crate::file_ops::AppState;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::State;

// Result codes, shared verbatim with the bridge: the frontend branches on these
// strings to pick between reopening the document and showing a message.
pub const REMOVED: &str = "removed";
pub const NO_NOTES: &str = "no_notes";
pub const ALREADY_REMOVED: &str = "already_removed";
pub const NOT_DOCX: &str = "not_docx";
pub const NO_FILE: &str = "no_file";

const FOOTNOTES_PART: &str = "word/footnotes.xml";

#[tauri::command]
pub async fn remove_note_separator(state: State<'_, AppState>) -> Result<String, String> {
    // Cloned out of the mutex here: the blocking closure below outlives the lock,
    // and the path is all it needs.
    let current = state.current_file.lock().unwrap().clone();
    let Some(path) = current else {
        // A new document that was never saved has nothing on disk to operate on.
        return Ok(NO_FILE.to_string());
    };
    if !is_docx(&path) {
        return Ok(NOT_DOCX.to_string());
    }

    // Reading, rewriting and renaming a whole package is blocking I/O, and a sync
    // command runs inline on the IPC handler, which on Linux is the GTK main loop:
    // the UI would stop repainting for as long as the surgery takes (journal 046,
    // the same discipline the clipboard reads follow).
    tauri::async_runtime::spawn_blocking(move || remove_note_separator_at(&path))
        .await
        .map_err(|e| format!("note separator task failed: {}", e))?
}

fn is_docx(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("docx"))
        .unwrap_or(false)
}

fn remove_note_separator_at(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    let (code, rewritten) = remove_separator_from_docx(&bytes)?;
    let Some(rewritten) = rewritten else {
        return Ok(code.to_string());
    };

    // Same directory as the destination: a rename is only atomic within one
    // filesystem, and the system temp dir is routinely on another one.
    let temp_path = temp_sibling(path);
    if let Err(e) = std::fs::write(&temp_path, &rewritten) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Cannot write {}: {}", temp_path.display(), e));
    }
    if let Err(e) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Cannot replace {}: {}", path.display(), e));
    }
    Ok(code.to_string())
}

fn temp_sibling(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "document.docx".to_string());
    let temp_name = format!(".{}.eo-{}-{}.tmp", name, std::process::id(), stamp);
    match path.parent() {
        Some(dir) => dir.join(temp_name),
        None => PathBuf::from(temp_name),
    }
}

// The whole surgery, on bytes, so it can be exercised without a Tauri app or a
// file on disk. Returns the result code and, only for REMOVED, the bytes of the
// rewritten package.
pub fn remove_separator_from_docx(bytes: &[u8]) -> Result<(&'static str, Option<Vec<u8>>), String> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("Not a readable docx: {}", e))?;

    let footnotes = match read_entry(&mut archive, FOOTNOTES_PART)? {
        Some(data) => data,
        // No footnotes part at all: the document has never had a note.
        None => return Ok((NO_NOTES, None)),
    };
    let xml = String::from_utf8(footnotes)
        .map_err(|_| format!("{} is not valid UTF-8", FOOTNOTES_PART))?;

    // The engine regenerates the whole part for a document with no real notes, so
    // the edit would be thrown away. Saying so is more useful than silently
    // writing something that does not survive the next save.
    if !has_real_notes(&xml) {
        return Ok((NO_NOTES, None));
    }

    let Some(edited) = strip_separator_run(&xml) else {
        return Ok((ALREADY_REMOVED, None));
    };

    let rewritten = rewrite_package(&mut archive, &edited)?;
    Ok((REMOVED, Some(rewritten)))
}

fn read_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let mut file = match archive.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(format!("Cannot read {}: {}", name, e)),
    };
    let mut data = Vec::new();
    file.read_to_end(&mut data)
        .map_err(|e| format!("Cannot read {}: {}", name, e))?;
    Ok(Some(data))
}

// Every entry is copied across byte for byte except the footnotes part. Stored
// entries stay stored and everything else is deflated: a docx uses no other
// method, and re-deflating changes only the packaging, not the content.
fn rewrite_package<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    footnotes_xml: &str,
) -> Result<Vec<u8>, String> {
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Cannot read entry {}: {}", i, e))?;
        let name = entry.name().to_string();

        if entry.is_dir() {
            out.add_directory(name, zip::write::SimpleFileOptions::default())
                .map_err(|e| format!("Cannot copy directory {}: {}", i, e))?;
            continue;
        }

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(match entry.compression() {
                zip::CompressionMethod::Stored => zip::CompressionMethod::Stored,
                _ => zip::CompressionMethod::Deflated,
            });

        let payload = if name == FOOTNOTES_PART {
            footnotes_xml.as_bytes().to_vec()
        } else {
            let mut data = Vec::new();
            entry
                .read_to_end(&mut data)
                .map_err(|e| format!("Cannot read {}: {}", name, e))?;
            data
        };

        out.start_file(&name, options)
            .map_err(|e| format!("Cannot write {}: {}", name, e))?;
        out.write_all(&payload)
            .map_err(|e| format!("Cannot write {}: {}", name, e))?;
    }

    let cursor = out.finish().map_err(|e| format!("Cannot finish package: {}", e))?;
    Ok(cursor.into_inner())
}

// A real note carries a positive id: the separator is -1 and the
// continuationSeparator is 0, and both exist in a document nobody has ever added
// a footnote to.
fn has_real_notes(xml: &str) -> bool {
    element_starts(xml, "w:footnote")
        .into_iter()
        .filter_map(|pos| open_tag_at(xml, pos))
        .filter_map(|tag| attr_value(tag, "w:id"))
        .any(|id| id.trim().parse::<i64>().map(|n| n > 0).unwrap_or(false))
}

// Returns the edited XML, or None when there is nothing to remove: either the
// separator footnote is not there or its <w:separator/> run is already gone.
fn strip_separator_run(xml: &str) -> Option<String> {
    let (block_start, block_end) = separator_footnote_span(xml)?;
    let block = &xml[block_start..block_end];

    // Bounded to the separator footnote on purpose. The same element name shows
    // up in the continuationSeparator footnote and, through <w:separator/> in a
    // real note's own content, anywhere else in the part.
    let sep_pos = *element_starts(block, "w:separator").first()?;
    let run_start = run_start_before(block, sep_pos)?;
    let run_end = block[sep_pos..].find("</w:r>").map(|p| sep_pos + p + "</w:r>".len())?;

    let mut edited = String::with_capacity(xml.len());
    edited.push_str(&xml[..block_start + run_start]);
    edited.push_str(&block[run_end..]);
    edited.push_str(&xml[block_end..]);
    Some(edited)
}

fn separator_footnote_span(xml: &str) -> Option<(usize, usize)> {
    for pos in element_starts(xml, "w:footnote") {
        let tag = match open_tag_at(xml, pos) {
            Some(tag) => tag,
            None => continue,
        };
        if attr_value(tag, "w:type").as_deref() != Some("separator") {
            continue;
        }
        let close = xml[pos..].find("</w:footnote>")? + pos + "</w:footnote>".len();
        return Some((pos, close));
    }
    None
}

// The nearest enclosing <w:r>. Walking backwards has to skip <w:rPr>, <w:rFonts>
// and every other element whose name merely starts with w:r.
fn run_start_before(block: &str, pos: usize) -> Option<usize> {
    element_starts(&block[..pos], "w:r").into_iter().next_back()
}

// Positions where element `name` opens. The character after the name must end it,
// so "w:r" does not match <w:rPr> and "w:footnote" does not match <w:footnotePr>,
// <w:footnoteRef> or the <w:footnotes> root.
fn element_starts(xml: &str, name: &str) -> Vec<usize> {
    let needle = format!("<{}", name);
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(rel) = xml[from..].find(&needle) {
        let pos = from + rel;
        let after = pos + needle.len();
        match xml[after..].chars().next() {
            Some(c) if c == '>' || c == '/' || c.is_whitespace() => found.push(pos),
            _ => {}
        }
        from = after;
    }
    found
}

// The text of the opening tag that starts at `pos`, angle brackets excluded.
fn open_tag_at(xml: &str, pos: usize) -> Option<&str> {
    let end = xml[pos..].find('>')? + pos;
    Some(&xml[pos + 1..end])
}

// Attribute lookup inside one opening tag. Names are matched whole, so w:id does
// not answer for w:idOther, and both quote styles are accepted.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(rel) = tag[from..].find(name) {
        let pos = from + rel;
        from = pos + name.len();
        let preceded_ok = pos == 0
            || tag[..pos]
                .chars()
                .next_back()
                .map(|c| c.is_whitespace())
                .unwrap_or(false);
        if !preceded_ok {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let value = &rest[1..];
        let end = value.find(quote)?;
        return Some(value[..end].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // The separator footnote exactly as the engine writes it (from the autopsy of
    // a document saved by the editor), followed by the continuationSeparator.
    const SEPARATOR_FOOTNOTE: &str = "<w:footnote w:type=\"separator\" w:id=\"-1\">\
<w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>\
<w:r></w:r><w:r><w:separator/></w:r><w:r></w:r></w:p></w:footnote>";
    const CONTINUATION_FOOTNOTE: &str = "<w:footnote w:type=\"continuationSeparator\" w:id=\"0\">\
<w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>\
<w:r></w:r><w:r><w:continuationSeparator/></w:r><w:r></w:r></w:p></w:footnote>";
    const REAL_NOTE: &str = "<w:footnote w:id=\"1\"><w:p><w:pPr><w:pStyle w:val=\"FootnoteText\"/></w:pPr>\
<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteRef/></w:r>\
<w:r><w:t xml:space=\"preserve\"> a real note</w:t></w:r></w:p></w:footnote>";

    fn footnotes_part(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:footnotes xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</w:footnotes>",
            body
        )
    }

    // Everything a docx carries besides the footnotes part, so the copy-across can
    // be checked for real rather than on a one-entry package.
    fn other_entries() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("[Content_Types].xml", b"<Types/>".to_vec()),
            ("_rels/.rels", b"<Relationships/>".to_vec()),
            ("word/document.xml", b"<w:document><w:body/></w:document>".to_vec()),
            // Binary, and incompressible, so a re-deflate that mangled it would show.
            ("word/media/image1.png", (0u8..=255).cycle().take(4096).collect()),
        ]
    }

    fn docx_with(footnotes: Option<&str>) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in other_entries() {
            writer.start_file(name, options).unwrap();
            writer.write_all(&data).unwrap();
        }
        if let Some(xml) = footnotes {
            writer.start_file(FOOTNOTES_PART, options).unwrap();
            writer.write_all(xml.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn entry_of(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        read_entry(&mut archive, name).unwrap()
    }

    fn footnotes_of(bytes: &[u8]) -> String {
        String::from_utf8(entry_of(bytes, FOOTNOTES_PART).expect("footnotes part")).unwrap()
    }

    // The case the feature exists for.
    #[test]
    fn a_document_with_notes_loses_the_separator_run() {
        let body = format!("{}{}{}", SEPARATOR_FOOTNOTE, CONTINUATION_FOOTNOTE, REAL_NOTE);
        let docx = docx_with(Some(&footnotes_part(&body)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();
        assert_eq!(code, REMOVED);
        let xml = footnotes_of(&rewritten.expect("the package must be rewritten"));

        assert!(
            !xml.contains("<w:separator/>"),
            "the run holding the separator is what draws the line: {}",
            xml
        );
        assert!(
            xml.contains("<w:continuationSeparator/>"),
            "the continuation separator is a different line and stays: {}",
            xml
        );
        assert!(
            xml.contains("w:type=\"separator\""),
            "the separator footnote itself must survive, emptied, or the engine \
             regenerates it with the line back: {}",
            xml
        );
        // Only that one run of the three goes.
        assert_eq!(
            xml.matches("<w:r>").count() + xml.matches("<w:r ").count(),
            body.matches("<w:r>").count() + body.matches("<w:r ").count() - 1,
            "exactly one run may be removed: {}",
            xml
        );
        assert!(xml.contains("a real note"), "the notes themselves are untouched");
    }

    // No footnotes part: the document has never had a note.
    #[test]
    fn a_document_without_the_footnotes_part_reports_no_notes() {
        let docx = docx_with(None);

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();

        assert_eq!(code, NO_NOTES);
        assert!(rewritten.is_none(), "nothing may be written for a no-op");
    }

    // The part exists but holds only the two special footnotes. The engine
    // regenerates it wholesale for such a document, so the edit would not survive
    // the next save and the user has to be told instead.
    #[test]
    fn a_document_with_only_the_special_footnotes_reports_no_notes() {
        let body = format!("{}{}", SEPARATOR_FOOTNOTE, CONTINUATION_FOOTNOTE);
        let docx = docx_with(Some(&footnotes_part(&body)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();

        assert_eq!(code, NO_NOTES);
        assert!(rewritten.is_none());
    }

    // Running it twice must not keep eating runs.
    #[test]
    fn a_document_already_operated_on_reports_already_removed() {
        let body = format!("{}{}{}", SEPARATOR_FOOTNOTE, CONTINUATION_FOOTNOTE, REAL_NOTE);
        let docx = docx_with(Some(&footnotes_part(&body)));
        let once = remove_separator_from_docx(&docx).unwrap().1.unwrap();

        let (code, rewritten) = remove_separator_from_docx(&once).unwrap();

        assert_eq!(code, ALREADY_REMOVED);
        assert!(rewritten.is_none());
    }

    // The search has to stay inside the separator footnote. A real note can hold
    // a <w:separator/> of its own (a rule drawn inside the note), and an
    // unbounded search would eat that run instead once the separator footnote is
    // already empty.
    #[test]
    fn a_separator_inside_a_real_note_is_never_touched() {
        let note_with_rule = "<w:footnote w:id=\"1\"><w:p><w:r><w:separator/></w:r>\
<w:r><w:t>note body</w:t></w:r></w:p></w:footnote>";
        let emptied = SEPARATOR_FOOTNOTE.replace("<w:r><w:separator/></w:r>", "");
        let body = format!("{}{}{}", emptied, CONTINUATION_FOOTNOTE, note_with_rule);
        let docx = docx_with(Some(&footnotes_part(&body)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();

        assert_eq!(code, ALREADY_REMOVED);
        assert!(
            rewritten.is_none(),
            "the note's own separator is the user's content, not the footnote rule"
        );
    }

    // The surgery rewrites the whole package, so everything it is not aiming at
    // has to come out the other side unchanged.
    #[test]
    fn every_other_entry_survives_byte_for_byte() {
        let body = format!("{}{}{}", SEPARATOR_FOOTNOTE, CONTINUATION_FOOTNOTE, REAL_NOTE);
        let docx = docx_with(Some(&footnotes_part(&body)));

        let rewritten = remove_separator_from_docx(&docx).unwrap().1.unwrap();

        for (name, data) in other_entries() {
            assert_eq!(
                entry_of(&rewritten, name).unwrap_or_default(),
                data,
                "{} must be copied across untouched",
                name
            );
        }
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(rewritten)).unwrap();
        assert_eq!(
            archive.len(),
            other_entries().len() + 1,
            "no entry may be dropped or invented"
        );
        assert!(archive.by_name("word/document.xml").is_ok());
    }

    // Spacing and attributes on the run are the engine's business and vary
    // between versions, so the search cannot depend on the exact bytes.
    #[test]
    fn the_run_is_found_through_whitespace_and_attributes() {
        let separator = "<w:footnote w:type='separator' w:id='-1'>\n  <w:p>\n    <w:pPr/>\n\
    <w:r w:rsidR=\"00AB12CD\">\n      <w:separator />\n    </w:r>\n  </w:p>\n</w:footnote>";
        let body = format!("{}{}{}", separator, CONTINUATION_FOOTNOTE, REAL_NOTE);
        let docx = docx_with(Some(&footnotes_part(&body)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();
        let xml = footnotes_of(&rewritten.expect("the package must be rewritten"));

        assert_eq!(code, REMOVED);
        assert!(!xml.contains("<w:separator"), "{}", xml);
        assert!(!xml.contains("00AB12CD"), "the whole run goes, not just the tag: {}", xml);
        assert!(xml.contains("<w:continuationSeparator/>"));
    }

    // A separator footnote whose run is nested one level deeper than the autopsy
    // showed must still lose the run, not the paragraph around it.
    #[test]
    fn only_the_enclosing_run_is_removed() {
        let body = format!("{}{}{}", SEPARATOR_FOOTNOTE, CONTINUATION_FOOTNOTE, REAL_NOTE);
        let xml = footnotes_part(&body);

        let edited = strip_separator_run(&xml).unwrap();

        assert!(edited.contains("<w:pPr>"), "the paragraph properties stay: {}", edited);
        assert!(edited.contains("</w:p></w:footnote>"), "the paragraph stays closed: {}", edited);
        assert_eq!(
            edited.matches("</w:footnote>").count(),
            3,
            "no footnote may be swallowed: {}",
            edited
        );
    }

    // The element scan must not answer for elements that merely share a prefix.
    #[test]
    fn element_matching_stops_at_the_name_boundary() {
        assert_eq!(element_starts("<w:rPr><w:r><w:rFonts/>", "w:r"), vec!["<w:rPr>".len()]);
        assert_eq!(
            element_starts("<w:footnotes><w:footnotePr/><w:footnote w:id=\"1\"/>", "w:footnote"),
            vec!["<w:footnotes><w:footnotePr/>".len()]
        );
        assert_eq!(
            element_starts("<w:separatorX/><w:separator/>", "w:separator"),
            vec!["<w:separatorX/>".len()]
        );
    }

    #[test]
    fn attributes_are_read_whole_and_in_either_quote_style() {
        assert_eq!(attr_value("w:footnote w:id=\"-1\"", "w:id").as_deref(), Some("-1"));
        assert_eq!(attr_value("w:footnote w:id='7'", "w:id").as_deref(), Some("7"));
        assert_eq!(attr_value("w:footnote w:idOther=\"3\"", "w:id"), None);
        assert_eq!(attr_value("w:footnote w:type = \"separator\"", "w:type").as_deref(), Some("separator"));
    }

    #[test]
    fn only_positive_ids_count_as_real_notes() {
        assert!(!has_real_notes(&footnotes_part(SEPARATOR_FOOTNOTE)));
        assert!(!has_real_notes(&footnotes_part(CONTINUATION_FOOTNOTE)));
        assert!(has_real_notes(&footnotes_part(REAL_NOTE)));
    }

    #[test]
    fn something_that_is_not_a_zip_is_an_error_not_a_code() {
        assert!(remove_separator_from_docx(b"not a docx at all").is_err());
    }
}
