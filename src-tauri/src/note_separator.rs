// Removing the note separator lines (#42).
//
// The editor has no setting for them: the horizontal rules above the notes are
// drawn from the special notes the engine keeps in word/footnotes.xml and
// word/endnotes.xml, and nothing in the UI reaches them. What the engine writes
// is
//
//   <w:footnote w:type="separator" w:id="-1"><w:p><w:pPr>...</w:pPr>
//     <w:r></w:r><w:r><w:separator/></w:r><w:r></w:r></w:p></w:footnote>
//
// followed by the continuationSeparator note (w:id="0"), which holds
// <w:continuationSeparator/> in the same shape. Endnotes mirror all of it with
// <w:endnote> blocks in word/endnotes.xml. Dropping the run that holds the
// element leaves an empty special note, which is exactly what a document
// without the line looks like. Validated experimentally: a document that HAS
// real notes keeps that empty separator across a save, while one with no notes
// at all has the whole part regenerated, so the edit only means anything once
// the document has notes of that kind.
//
// Four targets, one action: separator and continuationSeparator, in the
// footnotes part and in the endnotes part. The scope grew from the footnote
// separator alone after the screenshot on the issue was read properly: the
// reporter's document uses ENDNOTES, and the full-width rule he wants gone is
// the continuationSeparator drawn where the note block continues from the
// previous page. A part with no real notes of its own is left alone, since the
// engine regenerates it wholesale and the edit would be a lie.
//
// A zip cannot be edited in place, so the whole package is rewritten: every
// entry copied across and the edited parts replaced in a single pass. The new
// package is built beside the document and moved onto it with a rename, so a
// failure halfway through leaves the user's file untouched rather than
// truncated.

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
const ENDNOTES_PART: &str = "word/endnotes.xml";

// The two parts are the same document twice over, differing only in the name of
// the block element, so everything below is driven off this table instead of
// being written twice.
const NOTE_PARTS: [(&str, &str); 2] = [
    (FOOTNOTES_PART, "w:footnote"),
    (ENDNOTES_PART, "w:endnote"),
];

// The special notes to empty, each paired with the element its run holds.
const SPECIAL_NOTES: [(&str, &str); 2] = [
    ("separator", "w:separator"),
    ("continuationSeparator", "w:continuationSeparator"),
];

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

    // Every part present and carrying notes of its own is operated on. Both may
    // be there, either alone, or neither.
    let mut edits: Vec<(String, String)> = Vec::new();
    let mut any_real_notes = false;

    for (part, note_element) in NOTE_PARTS {
        let Some(data) = read_entry(&mut archive, part)? else {
            continue;
        };
        let xml = String::from_utf8(data).map_err(|_| format!("{} is not valid UTF-8", part))?;

        // The engine regenerates a part whose only notes are the special ones, so
        // the edit would be thrown away. Such a part is left alone, and if no part
        // has real notes the user is told rather than sold a change that does not
        // survive the next save.
        if !has_real_notes(&xml, note_element) {
            continue;
        }
        any_real_notes = true;

        if let Some(edited) = strip_separator_runs(&xml, note_element) {
            edits.push((part.to_string(), edited));
        }
    }

    if !any_real_notes {
        return Ok((NO_NOTES, None));
    }
    if edits.is_empty() {
        return Ok((ALREADY_REMOVED, None));
    }

    // One rewrite for however many parts changed: the package is copied once.
    let rewritten = rewrite_package(&mut archive, &edits)?;
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

// Every entry is copied across byte for byte except the ones in `edits`. Stored
// entries stay stored and everything else is deflated: a docx uses no other
// method, and re-deflating changes only the packaging, not the content.
fn rewrite_package<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    edits: &[(String, String)],
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

        let replacement = edits.iter().find(|(part, _)| *part == name);
        let payload = if let Some((_, xml)) = replacement {
            xml.as_bytes().to_vec()
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
// continuationSeparator is 0, and both exist in a part nobody has ever added a
// note to. `note_element` is w:footnote or w:endnote depending on the part.
fn has_real_notes(xml: &str, note_element: &str) -> bool {
    element_starts(xml, note_element)
        .into_iter()
        .filter_map(|pos| open_tag_at(xml, pos))
        .filter_map(|tag| attr_value(tag, "w:id"))
        .any(|id| id.trim().parse::<i64>().map(|n| n > 0).unwrap_or(false))
}

// Both special notes of one part, emptied in turn. Returns the edited XML, or
// None when there was nothing to remove in either of them.
fn strip_separator_runs(xml: &str, note_element: &str) -> Option<String> {
    let mut edited: Option<String> = None;
    for (note_type, run_element) in SPECIAL_NOTES {
        let source = edited.as_deref().unwrap_or(xml);
        if let Some(next) = strip_run_from_special_note(source, note_element, note_type, run_element)
        {
            edited = Some(next);
        }
    }
    edited
}

// Returns the edited XML, or None when there is nothing to remove: either the
// special note is not there or its run is already gone.
fn strip_run_from_special_note(
    xml: &str,
    note_element: &str,
    note_type: &str,
    run_element: &str,
) -> Option<String> {
    let (block_start, block_end) = special_note_span(xml, note_element, note_type)?;
    let block = &xml[block_start..block_end];

    // Bounded to the special note on purpose. The same element name shows up
    // through <w:separator/> or <w:continuationSeparator/> in a real note's own
    // content, anywhere else in the part.
    let sep_pos = *element_starts(block, run_element).first()?;
    let run_start = run_start_before(block, sep_pos)?;
    let run_end = block[sep_pos..].find("</w:r>").map(|p| sep_pos + p + "</w:r>".len())?;

    let mut edited = String::with_capacity(xml.len());
    edited.push_str(&xml[..block_start + run_start]);
    edited.push_str(&block[run_end..]);
    edited.push_str(&xml[block_end..]);
    Some(edited)
}

fn special_note_span(xml: &str, note_element: &str, note_type: &str) -> Option<(usize, usize)> {
    let closing = format!("</{}>", note_element);
    for pos in element_starts(xml, note_element) {
        let tag = match open_tag_at(xml, pos) {
            Some(tag) => tag,
            None => continue,
        };
        if attr_value(tag, "w:type").as_deref() != Some(note_type) {
            continue;
        }
        let close = xml[pos..].find(&closing)? + pos + closing.len();
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
// so "w:r" does not match <w:rPr>, "w:footnote" does not match <w:footnotePr>,
// <w:footnoteRef> or the <w:footnotes> root, and "w:separator" does not match
// <w:continuationSeparator/>.
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

    const FOOTNOTE: &str = "w:footnote";
    const ENDNOTE: &str = "w:endnote";

    // The special notes exactly as the engine writes them (from the autopsy of a
    // document saved by the editor), built for either note element since the two
    // parts are the same document twice over.
    fn special_note(note_element: &str, note_type: &str, id: &str, run_element: &str) -> String {
        format!(
            "<{n} w:type=\"{t}\" w:id=\"{i}\">\
<w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>\
<w:r></w:r><w:r><{r}/></w:r><w:r></w:r></w:p></{n}>",
            n = note_element,
            t = note_type,
            i = id,
            r = run_element
        )
    }

    fn separator_note(note_element: &str) -> String {
        special_note(note_element, "separator", "-1", "w:separator")
    }

    fn continuation_note(note_element: &str) -> String {
        special_note(note_element, "continuationSeparator", "0", "w:continuationSeparator")
    }

    // Both special notes, in the order the engine emits them.
    fn specials(note_element: &str) -> String {
        format!("{}{}", separator_note(note_element), continuation_note(note_element))
    }

    fn real_note(note_element: &str) -> String {
        let style = if note_element == FOOTNOTE { "Footnote" } else { "Endnote" };
        format!(
            "<{n} w:id=\"1\"><w:p><w:pPr><w:pStyle w:val=\"{s}Text\"/></w:pPr>\
<w:r><w:rPr><w:rStyle w:val=\"{s}Reference\"/></w:rPr><w:{l}Ref/></w:r>\
<w:r><w:t xml:space=\"preserve\"> a real note</w:t></w:r></w:p></{n}>",
            n = note_element,
            s = style,
            l = note_element
        )
    }

    // The part as a whole: <w:footnotes> or <w:endnotes> around the blocks.
    fn notes_part(note_element: &str, body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<{n}s xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{b}</{n}s>",
            n = note_element,
            b = body
        )
    }

    // A part holding the two special notes and one real note: the ordinary case.
    fn populated_part(note_element: &str) -> String {
        notes_part(note_element, &format!("{}{}", specials(note_element), real_note(note_element)))
    }

    fn footnotes_part(body: &str) -> String {
        notes_part(FOOTNOTE, body)
    }

    // Everything a docx carries besides the note parts, so the copy-across can be
    // checked for real rather than on a one-entry package.
    fn other_entries() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("[Content_Types].xml", b"<Types/>".to_vec()),
            ("_rels/.rels", b"<Relationships/>".to_vec()),
            ("word/document.xml", b"<w:document><w:body/></w:document>".to_vec()),
            // Binary, and incompressible, so a re-deflate that mangled it would show.
            ("word/media/image1.png", (0u8..=255).cycle().take(4096).collect()),
        ]
    }

    fn docx_with_parts(footnotes: Option<&str>, endnotes: Option<&str>) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in other_entries() {
            writer.start_file(name, options).unwrap();
            writer.write_all(&data).unwrap();
        }
        for (part, xml) in [(FOOTNOTES_PART, footnotes), (ENDNOTES_PART, endnotes)] {
            if let Some(xml) = xml {
                writer.start_file(part, options).unwrap();
                writer.write_all(xml.as_bytes()).unwrap();
            }
        }
        writer.finish().unwrap().into_inner()
    }

    fn docx_with(footnotes: Option<&str>) -> Vec<u8> {
        docx_with_parts(footnotes, None)
    }

    fn entry_of(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        read_entry(&mut archive, name).unwrap()
    }

    fn part_of(bytes: &[u8], name: &str) -> String {
        String::from_utf8(entry_of(bytes, name).expect("the part must be there")).unwrap()
    }

    fn footnotes_of(bytes: &[u8]) -> String {
        part_of(bytes, FOOTNOTES_PART)
    }

    fn endnotes_of(bytes: &[u8]) -> String {
        part_of(bytes, ENDNOTES_PART)
    }

    // Neither rule may survive in a part that was operated on, and the special
    // notes themselves have to stay, emptied.
    fn assert_both_rules_gone(xml: &str, note_element: &str) {
        assert!(
            !xml.contains("<w:separator/>"),
            "the run holding the separator is what draws the short line: {}",
            xml
        );
        assert!(
            !xml.contains("<w:continuationSeparator/>"),
            "the continuation separator draws the full-width line the reporter \
             wants gone as well: {}",
            xml
        );
        assert!(
            xml.contains("w:type=\"separator\"") && xml.contains("w:type=\"continuationSeparator\""),
            "both special {} blocks must survive, emptied, or the engine \
             regenerates them with the lines back: {}",
            note_element,
            xml
        );
    }

    // The case the feature exists for. Both rules of the footnotes part go: the
    // decision to keep the continuationSeparator was reversed once the reporter's
    // screenshot was read properly, because the full-width line drawn where the
    // note block continues from the previous page is part of what he is asking to
    // remove, not a separate feature.
    #[test]
    fn a_document_with_notes_loses_both_separator_runs() {
        let body = format!("{}{}", specials(FOOTNOTE), real_note(FOOTNOTE));
        let docx = docx_with(Some(&footnotes_part(&body)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();
        assert_eq!(code, REMOVED);
        let xml = footnotes_of(&rewritten.expect("the package must be rewritten"));

        assert_both_rules_gone(&xml, FOOTNOTE);
        // Only those two runs of the six go.
        assert_eq!(
            xml.matches("<w:r>").count() + xml.matches("<w:r ").count(),
            body.matches("<w:r>").count() + body.matches("<w:r ").count() - 2,
            "exactly two runs may be removed: {}",
            xml
        );
        assert!(xml.contains("a real note"), "the notes themselves are untouched");
    }

    // The reporter's own document: endnotes, no footnotes part at all.
    #[test]
    fn a_document_with_only_endnotes_loses_both_endnote_rules() {
        let docx = docx_with_parts(None, Some(&populated_part(ENDNOTE)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();
        assert_eq!(code, REMOVED);
        let rewritten = rewritten.expect("the package must be rewritten");

        assert_both_rules_gone(&endnotes_of(&rewritten), ENDNOTE);
        assert!(
            entry_of(&rewritten, FOOTNOTES_PART).is_none(),
            "no part may be invented for a document that has none"
        );
        assert!(endnotes_of(&rewritten).contains("a real note"));
    }

    // Both kinds of note in one document: one action, four runs.
    #[test]
    fn a_document_with_both_kinds_loses_all_four_runs() {
        let docx = docx_with_parts(Some(&populated_part(FOOTNOTE)), Some(&populated_part(ENDNOTE)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();
        assert_eq!(code, REMOVED);
        let rewritten = rewritten.expect("the package must be rewritten");

        assert_both_rules_gone(&footnotes_of(&rewritten), FOOTNOTE);
        assert_both_rules_gone(&endnotes_of(&rewritten), ENDNOTE);
    }

    // A part with nothing but the special notes is regenerated wholesale by the
    // engine, so it stays exactly as it was even when the other part is operated
    // on: editing it would be telling the user something that does not survive
    // the next save.
    #[test]
    fn a_part_without_real_notes_is_left_alone_while_the_other_is_operated_on() {
        let empty_endnotes = notes_part(ENDNOTE, &specials(ENDNOTE));
        let docx = docx_with_parts(Some(&populated_part(FOOTNOTE)), Some(&empty_endnotes));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();
        assert_eq!(code, REMOVED);
        let rewritten = rewritten.expect("the package must be rewritten");

        assert_both_rules_gone(&footnotes_of(&rewritten), FOOTNOTE);
        assert_eq!(
            endnotes_of(&rewritten),
            empty_endnotes,
            "a part the engine regenerates must come out byte for byte as it went in"
        );
    }

    // No note part at all: the document has never had a note.
    #[test]
    fn a_document_without_any_note_part_reports_no_notes() {
        let docx = docx_with_parts(None, None);

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();

        assert_eq!(code, NO_NOTES);
        assert!(rewritten.is_none(), "nothing may be written for a no-op");
    }

    // Both parts present but holding only the special notes: same answer.
    #[test]
    fn a_document_with_only_the_special_notes_reports_no_notes() {
        let docx = docx_with_parts(
            Some(&notes_part(FOOTNOTE, &specials(FOOTNOTE))),
            Some(&notes_part(ENDNOTE, &specials(ENDNOTE))),
        );

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();

        assert_eq!(code, NO_NOTES);
        assert!(rewritten.is_none());
    }

    // Running it twice must not keep eating runs, and that holds with both parts
    // in play.
    #[test]
    fn a_document_already_operated_on_reports_already_removed() {
        let docx = docx_with_parts(Some(&populated_part(FOOTNOTE)), Some(&populated_part(ENDNOTE)));
        let once = remove_separator_from_docx(&docx).unwrap().1.unwrap();

        let (code, rewritten) = remove_separator_from_docx(&once).unwrap();

        assert_eq!(code, ALREADY_REMOVED);
        assert!(rewritten.is_none());
    }

    // One part still has a rule to lose: that is a removal, not an already_removed,
    // even though the other part is done.
    #[test]
    fn one_part_still_holding_a_rule_is_enough_for_a_removal() {
        let done = notes_part(
            FOOTNOTE,
            &format!(
                "{}{}",
                specials(FOOTNOTE)
                    .replace("<w:r><w:separator/></w:r>", "")
                    .replace("<w:r><w:continuationSeparator/></w:r>", ""),
                real_note(FOOTNOTE)
            ),
        );
        let docx = docx_with_parts(Some(&done), Some(&populated_part(ENDNOTE)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();
        assert_eq!(code, REMOVED);
        let rewritten = rewritten.expect("the package must be rewritten");

        assert_eq!(footnotes_of(&rewritten), done, "the finished part is not touched again");
        assert_both_rules_gone(&endnotes_of(&rewritten), ENDNOTE);
    }

    // The search has to stay inside the special notes. A real note can hold a
    // <w:separator/> or a <w:continuationSeparator/> of its own (a rule drawn
    // inside the note), and an unbounded search would eat that run instead once
    // the special notes are already empty.
    #[test]
    fn a_separator_inside_a_real_note_is_never_touched() {
        for (note_element, run_element) in [
            (FOOTNOTE, "w:separator"),
            (FOOTNOTE, "w:continuationSeparator"),
            (ENDNOTE, "w:separator"),
            (ENDNOTE, "w:continuationSeparator"),
        ] {
            let note_with_rule = format!(
                "<{n} w:id=\"1\"><w:p><w:r><{r}/></w:r><w:r><w:t>note body</w:t></w:r></w:p></{n}>",
                n = note_element,
                r = run_element
            );
            let emptied = specials(note_element)
                .replace("<w:r><w:separator/></w:r>", "")
                .replace("<w:r><w:continuationSeparator/></w:r>", "");
            let part = notes_part(note_element, &format!("{}{}", emptied, note_with_rule));
            let docx = if note_element == FOOTNOTE {
                docx_with_parts(Some(&part), None)
            } else {
                docx_with_parts(None, Some(&part))
            };

            let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();

            assert_eq!(code, ALREADY_REMOVED, "{} / {}", note_element, run_element);
            assert!(
                rewritten.is_none(),
                "the note's own {} is the user's content, not the note rule",
                run_element
            );
        }
    }

    // The surgery rewrites the whole package, so everything it is not aiming at
    // has to come out the other side unchanged.
    #[test]
    fn every_other_entry_survives_byte_for_byte() {
        let docx = docx_with_parts(Some(&populated_part(FOOTNOTE)), Some(&populated_part(ENDNOTE)));

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
            other_entries().len() + 2,
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
        let continuation = "<w:footnote w:type = 'continuationSeparator' w:id='0'>\n  <w:p>\n\
    <w:r w:rsidR=\"00EF34AB\">\n      <w:continuationSeparator />\n    </w:r>\n  </w:p>\n</w:footnote>";
        let body = format!("{}{}{}", separator, continuation, real_note(FOOTNOTE));
        let docx = docx_with(Some(&footnotes_part(&body)));

        let (code, rewritten) = remove_separator_from_docx(&docx).unwrap();
        let xml = footnotes_of(&rewritten.expect("the package must be rewritten"));

        assert_eq!(code, REMOVED);
        assert!(!xml.contains("<w:separator"), "{}", xml);
        assert!(!xml.contains("<w:continuationSeparator"), "{}", xml);
        assert!(!xml.contains("00AB12CD"), "the whole run goes, not just the tag: {}", xml);
        assert!(!xml.contains("00EF34AB"), "the whole run goes, not just the tag: {}", xml);
    }

    // A special note whose run is nested one level deeper than the autopsy showed
    // must still lose the run, not the paragraph around it.
    #[test]
    fn only_the_enclosing_runs_are_removed() {
        let xml = populated_part(FOOTNOTE);

        let edited = strip_separator_runs(&xml, FOOTNOTE).unwrap();

        assert!(edited.contains("<w:pPr>"), "the paragraph properties stay: {}", edited);
        assert!(edited.contains("</w:p></w:footnote>"), "the paragraph stays closed: {}", edited);
        assert_eq!(
            edited.matches("</w:footnote>").count(),
            3,
            "no footnote may be swallowed: {}",
            edited
        );
    }

    // The element scan must not answer for elements that merely share a prefix,
    // and that is what keeps <w:separator/> from matching inside
    // <w:continuationSeparator/> and w:endnote from matching the root.
    #[test]
    fn element_matching_stops_at_the_name_boundary() {
        assert_eq!(element_starts("<w:rPr><w:r><w:rFonts/>", "w:r"), vec!["<w:rPr>".len()]);
        assert_eq!(
            element_starts("<w:footnotes><w:footnotePr/><w:footnote w:id=\"1\"/>", "w:footnote"),
            vec!["<w:footnotes><w:footnotePr/>".len()]
        );
        assert_eq!(
            element_starts("<w:endnotes><w:endnotePr/><w:endnote w:id=\"1\"/>", "w:endnote"),
            vec!["<w:endnotes><w:endnotePr/>".len()]
        );
        assert_eq!(
            element_starts("<w:separatorX/><w:separator/>", "w:separator"),
            vec!["<w:separatorX/>".len()]
        );
        assert!(element_starts("<w:continuationSeparator/>", "w:separator").is_empty());
    }

    #[test]
    fn attributes_are_read_whole_and_in_either_quote_style() {
        assert_eq!(attr_value("w:footnote w:id=\"-1\"", "w:id").as_deref(), Some("-1"));
        assert_eq!(attr_value("w:endnote w:id='7'", "w:id").as_deref(), Some("7"));
        assert_eq!(attr_value("w:footnote w:idOther=\"3\"", "w:id"), None);
        assert_eq!(attr_value("w:footnote w:type = \"separator\"", "w:type").as_deref(), Some("separator"));
    }

    #[test]
    fn only_positive_ids_of_the_matching_element_count_as_real_notes() {
        for note_element in [FOOTNOTE, ENDNOTE] {
            let part = notes_part(note_element, &specials(note_element));
            assert!(!has_real_notes(&part, note_element));
            assert!(has_real_notes(&populated_part(note_element), note_element));
        }
        // A footnotes part never answers for endnotes, and the other way round.
        assert!(!has_real_notes(&populated_part(FOOTNOTE), ENDNOTE));
        assert!(!has_real_notes(&populated_part(ENDNOTE), FOOTNOTE));
    }

    #[test]
    fn something_that_is_not_a_zip_is_an_error_not_a_code() {
        assert!(remove_separator_from_docx(b"not a docx at all").is_err());
    }
}
