// Text-level work on the XML x2t writes: drop what changes on every save
// (dates, edit session ids, the writer's name), and pull out the document text
// for the round-trip check. x2t writes well-formed XML, so a small tokenizer
// is enough; nothing here is a general XML parser.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Start,
    End,
    Empty,
    // Declarations, comments, CDATA and processing instructions.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token<'a> {
    Text(&'a str),
    Tag { raw: &'a str, name: &'a str, kind: Kind },
}

fn tokens(xml: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < xml.len() {
        let Some(offset) = xml[pos..].find('<') else {
            out.push(Token::Text(&xml[pos..]));
            break;
        };
        if offset > 0 {
            out.push(Token::Text(&xml[pos..pos + offset]));
        }
        let start = pos + offset;
        let rest = &xml[start..];
        let end = if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if rest.starts_with("<![CDATA[") {
            rest.find("]]>").map(|i| i + 3)
        } else {
            // Up to the first > outside a quoted attribute value.
            let mut quote = None;
            let mut found = None;
            for (i, &c) in rest.as_bytes().iter().enumerate().skip(1) {
                match (quote, c) {
                    (None, b'"') | (None, b'\'') => quote = Some(c),
                    (Some(q), _) if c == q => quote = None,
                    (None, b'>') => {
                        found = Some(i + 1);
                        break;
                    }
                    _ => {}
                }
            }
            found
        };
        let end = start + end.unwrap_or(rest.len());
        let raw = &xml[start..end];
        let kind = if raw.starts_with("<!") || raw.starts_with("<?") {
            Kind::Other
        } else if raw.starts_with("</") {
            Kind::End
        } else if raw.ends_with("/>") {
            Kind::Empty
        } else {
            Kind::Start
        };
        let name_start = if kind == Kind::End { 2 } else { 1 };
        let name_len = raw[name_start..]
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(raw.len() - name_start);
        let name = if kind == Kind::Other { "" } else { &raw[name_start..name_start + name_len] };
        out.push(Token::Tag { raw, name, kind });
        pos = end;
    }
    out
}

// (name, value) pairs of a start or empty tag, values still escaped.
fn attributes(raw: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let inner = raw.trim_start_matches('<').trim_end_matches('>').trim_end_matches('/');
    let mut rest = match inner.find(char::is_whitespace) {
        Some(i) => &inner[i..],
        None => return out,
    };
    loop {
        rest = rest.trim_start();
        let Some(eq) = rest.find('=') else { break };
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let Some(close) = after[1..].find(quote) else { break };
        out.push((name, &after[1..1 + close]));
        rest = &after[close + 2..];
    }
    out
}

pub fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';').filter(|s| *s <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// The XML without the named elements, content included.
pub fn remove_elements(xml: &str, names: &[&str]) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut skipping: Option<(&str, usize)> = None;
    for token in tokens(xml) {
        if let Some((name, depth)) = skipping {
            if let Token::Tag { name: n, kind, .. } = &token {
                if *n == name && *kind == Kind::Start {
                    skipping = Some((name, depth + 1));
                } else if *n == name && *kind == Kind::End {
                    skipping = if depth == 0 { None } else { Some((name, depth - 1)) };
                }
            }
            continue;
        }
        match token {
            Token::Tag { name, kind: Kind::Empty, .. } if names.contains(&name) => {}
            Token::Tag { name, kind: Kind::Start, .. } if names.contains(&name) => skipping = Some((name, 0)),
            Token::Text(text) => out.push_str(text),
            Token::Tag { raw, .. } => out.push_str(raw),
        }
    }
    out
}

// The XML without the attributes `drop` picks. Tags it does not touch are
// kept byte for byte.
pub fn remove_attributes(xml: &str, drop: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(xml.len());
    for token in tokens(xml) {
        match token {
            Token::Text(text) => out.push_str(text),
            Token::Tag { raw, name, kind } if kind == Kind::Start || kind == Kind::Empty => {
                let attrs = attributes(raw);
                if !attrs.iter().any(|(n, _)| drop(n)) {
                    out.push_str(raw);
                    continue;
                }
                out.push('<');
                out.push_str(name);
                for (n, v) in attrs.into_iter().filter(|(n, _)| !drop(n)) {
                    out.push_str(&format!(" {}=\"{}\"", n, v));
                }
                out.push_str(if kind == Kind::Empty { "/>" } else { ">" });
            }
            Token::Tag { raw, .. } => out.push_str(raw),
        }
    }
    out
}

// The order of the Default and Override entries carries no meaning in a
// package, and x2t writes them in a different order on each OS: they are
// put back in sorted order before the closing tag.
fn sort_content_types(xml: &str) -> String {
    let mut entries = Vec::new();
    let mut out = String::with_capacity(xml.len());
    for token in tokens(xml) {
        match token {
            Token::Tag { raw, name, kind: Kind::Empty } if name == "Default" || name == "Override" => entries.push(raw),
            Token::Tag { raw, name: "Types", kind: Kind::End } => {
                entries.sort_unstable();
                entries.iter().for_each(|entry| out.push_str(entry));
                entries.clear();
                out.push_str(raw);
            }
            Token::Text(text) => out.push_str(text),
            Token::Tag { raw, .. } => out.push_str(raw),
        }
    }
    // Entries without a closing tag stay where the tokens left them.
    entries.iter().for_each(|entry| out.push_str(entry));
    out
}

// A package part with what changes on every save removed. Parts not listed
// here are compared as they are.
pub fn normalize_part(part: &str, xml: &str) -> String {
    match part {
        "docProps/core.xml" => remove_elements(
            xml,
            &["dcterms:created", "dcterms:modified", "cp:lastModifiedBy", "cp:revision", "cp:lastPrinted"],
        ),
        // The writer's name and version are kept apart (app.application).
        "docProps/app.xml" => remove_elements(xml, &["Application", "AppVersion", "TotalTime", "DocSecurity"]),
        "meta.xml" => remove_elements(
            xml,
            &["meta:creation-date", "dc:date", "meta:editing-duration", "meta:editing-cycles", "meta:generator"],
        ),
        "[Content_Types].xml" => sort_content_types(xml),
        _ if part.starts_with("word/") && part.ends_with(".xml") => {
            let xml = if part == "word/settings.xml" {
                remove_elements(xml, &["w:rsids"])
            } else {
                xml.to_string()
            };
            remove_attributes(&xml, |name| {
                name.starts_with("w:rsid") || name == "w14:paraId" || name == "w14:textId"
            })
        }
        _ => xml.to_string(),
    }
}

// The decoded text of the first element with this name.
pub fn element_text(xml: &str, element: &str) -> Option<String> {
    let mut inside = false;
    let mut text = String::new();
    for token in tokens(xml) {
        match token {
            Token::Tag { name, kind: Kind::Start, .. } if name == element => inside = true,
            Token::Tag { name, kind: Kind::End, .. } if name == element && inside => return Some(decode(&text)),
            Token::Text(t) if inside => text.push_str(t),
            _ => {}
        }
    }
    None
}

// Trailing spaces on each line and empty lines at the end go: they carry no
// text and differ between extractors.
pub fn tidy(text: &str) -> String {
    let mut lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches([' ', '\t', '\r'])).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

// Paragraph text of WordprocessingML or DrawingML: one line per paragraph,
// with tabs and breaks inside it. Property blocks hold tab stops, not tabs.
fn paragraphs(xml: &str, paragraph: &str, text_el: &str, tab: &str, breaks: &[&str], props: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut in_text = false;
    let mut in_props = 0usize;
    for token in tokens(xml) {
        match token {
            Token::Tag { name, kind, .. } => {
                if props.contains(&name) {
                    match kind {
                        Kind::Start => in_props += 1,
                        Kind::End => in_props = in_props.saturating_sub(1),
                        _ => {}
                    }
                    continue;
                }
                if in_props > 0 {
                    continue;
                }
                match kind {
                    Kind::Start if name == paragraph => current = Some(String::new()),
                    Kind::Empty if name == paragraph => out.push(String::new()),
                    Kind::End if name == paragraph => out.extend(current.take()),
                    Kind::Start if name == text_el => in_text = true,
                    Kind::End if name == text_el => in_text = false,
                    Kind::Empty if name == tab => current.iter_mut().for_each(|p| p.push('\t')),
                    Kind::Empty if breaks.contains(&name) => current.iter_mut().for_each(|p| p.push('\n')),
                    _ => {}
                }
            }
            Token::Text(text) if in_text && in_props == 0 => {
                if let Some(p) = current.as_mut() {
                    p.push_str(&decode(text));
                }
            }
            _ => {}
        }
    }
    out
}

pub fn docx_text(document_xml: &str) -> String {
    tidy(&paragraphs(document_xml, "w:p", "w:t", "w:tab", &["w:br", "w:cr"], &["w:pPr", "w:rPr"]).join("\n"))
}

// Slides in their numeric order (slide2 before slide10).
pub fn pptx_text(slides: &[(u32, String)]) -> String {
    let mut sorted: Vec<&(u32, String)> = slides.iter().collect();
    sorted.sort_by_key(|(n, _)| *n);
    let mut out = Vec::new();
    for (n, xml) in sorted {
        out.push(format!("# slide {}", n));
        out.extend(paragraphs(xml, "a:p", "a:t", "a:tab", &["a:br"], &["a:pPr", "a:rPr", "a:endParaRPr"]));
    }
    tidy(&out.join("\n"))
}

// One ref=value line per filled cell, shared strings resolved.
pub fn xlsx_text(shared_strings: Option<&str>, sheets: &[(u32, String)]) -> String {
    let mut shared = Vec::new();
    if let Some(xml) = shared_strings {
        let mut current: Option<String> = None;
        let mut in_t = false;
        let mut in_phonetic = false;
        for token in tokens(xml) {
            match token {
                Token::Tag { name: "si", kind: Kind::Start, .. } => current = Some(String::new()),
                Token::Tag { name: "si", kind: Kind::End, .. } => shared.extend(current.take()),
                Token::Tag { name: "si", kind: Kind::Empty, .. } => shared.push(String::new()),
                Token::Tag { name: "rPh", kind: Kind::Start, .. } => in_phonetic = true,
                Token::Tag { name: "rPh", kind: Kind::End, .. } => in_phonetic = false,
                Token::Tag { name: "t", kind: Kind::Start, .. } => in_t = true,
                Token::Tag { name: "t", kind: Kind::End, .. } => in_t = false,
                Token::Text(text) if in_t && !in_phonetic => {
                    if let Some(s) = current.as_mut() {
                        s.push_str(&decode(text));
                    }
                }
                _ => {}
            }
        }
    }
    let mut sorted: Vec<&(u32, String)> = sheets.iter().collect();
    sorted.sort_by_key(|(n, _)| *n);
    let mut out = Vec::new();
    for (n, xml) in sorted {
        out.push(format!("# sheet {}", n));
        let mut cell: Option<(String, String)> = None;
        let mut value = String::new();
        let mut capture = false;
        for token in tokens(xml) {
            match token {
                Token::Tag { raw, name: "c", kind, .. } if kind == Kind::Start || kind == Kind::Empty => {
                    let attrs = attributes(raw);
                    let get = |k: &str| attrs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()).unwrap_or_default();
                    if kind == Kind::Start {
                        cell = Some((get("r"), get("t")));
                        value.clear();
                    }
                }
                Token::Tag { name: "c", kind: Kind::End, .. } => {
                    if let Some((reference, kind)) = cell.take() {
                        let shown = if kind == "s" {
                            value.trim().parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default()
                        } else {
                            value.clone()
                        };
                        if !shown.is_empty() {
                            out.push(format!("{}={}", reference, shown));
                        }
                    }
                }
                Token::Tag { name: "v", kind: Kind::Start, .. } | Token::Tag { name: "t", kind: Kind::Start, .. } => {
                    capture = cell.is_some()
                }
                Token::Tag { name: "v", kind: Kind::End, .. } | Token::Tag { name: "t", kind: Kind::End, .. } => capture = false,
                Token::Text(text) if capture => value.push_str(&decode(text)),
                _ => {}
            }
        }
    }
    tidy(&out.join("\n"))
}

// One line per text:p or text:h of content.xml; a paragraph nested in another
// (a frame or a note) comes out on its own line.
pub fn odf_text(content_xml: &str) -> String {
    let mut out = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    for token in tokens(content_xml) {
        match token {
            Token::Tag { name, kind, raw } => match (name, kind) {
                ("text:p" | "text:h", Kind::Start) => stack.push(String::new()),
                ("text:p" | "text:h", Kind::Empty) => out.push(String::new()),
                ("text:p" | "text:h", Kind::End) => out.extend(stack.pop()),
                ("text:tab", Kind::Empty) => stack.iter_mut().last().into_iter().for_each(|p| p.push('\t')),
                ("text:line-break", Kind::Empty) => stack.iter_mut().last().into_iter().for_each(|p| p.push('\n')),
                ("text:s", Kind::Empty) => {
                    let count = attributes(raw)
                        .iter()
                        .find(|(n, _)| *n == "text:c")
                        .and_then(|(_, v)| v.parse::<usize>().ok())
                        .unwrap_or(1);
                    if let Some(p) = stack.last_mut() {
                        p.push_str(&" ".repeat(count));
                    }
                }
                _ => {}
            },
            Token::Text(text) => {
                if let Some(p) = stack.last_mut() {
                    p.push_str(&decode(text));
                }
            }
        }
    }
    tidy(&out.join("\n"))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontNames {
    pub names: Vec<String>,
    // How many <a:font script=...> entries were skipped.
    pub scripts_ignored: usize,
}

// Every font an OOXML part names: typeface attributes (theme latin, ea and cs,
// slides, charts), spreadsheet <name>/<rFont>, and header or footer font
// codes. An empty typeface names no font; a +mj/+mn one points at the theme,
// which is checked itself.
pub fn ooxml_font_names(xml: &str) -> FontNames {
    let mut names = Vec::new();
    let mut scripts_ignored = 0;
    let mut in_font = false;
    let mut in_header = 0usize;
    for token in tokens(xml) {
        match token {
            Token::Tag { raw, name, kind } => {
                match (name, kind) {
                    ("font", Kind::Start) => in_font = true,
                    ("font", Kind::End) => in_font = false,
                    ("oddHeader" | "oddFooter" | "evenHeader" | "evenFooter" | "firstHeader" | "firstFooter", Kind::Start) => {
                        in_header += 1
                    }
                    ("oddHeader" | "oddFooter" | "evenHeader" | "evenFooter" | "firstHeader" | "firstFooter", Kind::End) => {
                        in_header = in_header.saturating_sub(1)
                    }
                    _ => {}
                }
                // Only the theme's per-script fallback list is skipped.
                if name == "a:font" && attributes(raw).iter().any(|(attr, _)| *attr == "script") {
                    scripts_ignored += 1;
                    continue;
                }
                for (attr, value) in attributes(raw) {
                    let value = decode(value);
                    let wanted = attr == "typeface"
                        || (attr == "val" && (name == "rFont" || (name == "name" && in_font)));
                    if wanted && !value.is_empty() && !value.starts_with('+') {
                        names.push(value);
                    }
                }
            }
            Token::Text(text) if in_header > 0 => {
                // &"Font Name,Style" sets the font of what follows.
                let text = decode(text);
                let mut rest = text.as_str();
                while let Some(i) = rest.find("&\"") {
                    rest = &rest[i + 2..];
                    let Some(end) = rest.find('"') else { break };
                    let spec = &rest[..end];
                    let font = spec.split(',').next().unwrap_or("").trim();
                    if !font.is_empty() && font != "-" {
                        names.push(font.to_string());
                    }
                    rest = &rest[end + 1..];
                }
            }
            _ => {}
        }
    }
    names.sort();
    names.dedup();
    FontNames { names, scripts_ignored }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_properties_lose_dates_and_counters_but_keep_the_user_metadata() {
        let xml = r#"<cp:coreProperties><dc:title>My &amp; title</dc:title><dc:creator>Ana</dc:creator><dc:subject>S</dc:subject><cp:keywords>k1 k2</cp:keywords><cp:lastModifiedBy>Bob</cp:lastModifiedBy><cp:revision>3</cp:revision><dcterms:created xsi:type="dcterms:W3CDTF">2026-10-18T10:00:00Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">2026-10-18T10:01:00Z</dcterms:modified><cp:lastPrinted/></cp:coreProperties>"#;
        assert_eq!(
            normalize_part("docProps/core.xml", xml),
            "<cp:coreProperties><dc:title>My &amp; title</dc:title><dc:creator>Ana</dc:creator><dc:subject>S</dc:subject><cp:keywords>k1 k2</cp:keywords></cp:coreProperties>"
        );
    }

    #[test]
    fn app_properties_lose_the_writer_and_the_timers() {
        let xml = "<Properties><Template>Normal</Template><TotalTime>2</TotalTime><Application>X/8.2</Application><DocSecurity>0</DocSecurity><AppVersion>8.2</AppVersion><Pages>1</Pages></Properties>";
        assert_eq!(normalize_part("docProps/app.xml", xml), "<Properties><Template>Normal</Template><Pages>1</Pages></Properties>");
        assert_eq!(element_text(xml, "Application"), Some("X/8.2".into()));
    }

    #[test]
    fn word_parts_lose_edit_session_ids() {
        let doc = r#"<w:p w:rsidR="00A1" w:rsidRDefault="00B2" w14:paraId="1A2B" w14:textId="77"><w:pPr w:rsidP="1"/><w:r w:rsidRPr="9"><w:t xml:space="preserve">a "b"</w:t></w:r></w:p><w:tbl w:rsidTr="x"/>"#;
        assert_eq!(
            normalize_part("word/document.xml", doc),
            r#"<w:p><w:pPr/><w:r><w:t xml:space="preserve">a "b"</w:t></w:r></w:p><w:tbl/>"#
        );
        let settings = "<w:settings><w:zoom w:percent=\"100\"/><w:rsids><w:rsidRoot w:val=\"1\"/><w:rsid w:val=\"2\"/></w:rsids><w:x/></w:settings>";
        assert_eq!(normalize_part("word/settings.xml", settings), "<w:settings><w:zoom w:percent=\"100\"/><w:x/></w:settings>");
        // A part with nothing to drop stays byte for byte, odd spacing included.
        let plain = "<w:p  w:x='1' ><w:t>t</w:t></w:p>";
        assert_eq!(normalize_part("word/document.xml", plain), plain);
    }

    #[test]
    fn odf_meta_loses_dates_and_the_generator_but_keeps_the_title() {
        let xml = "<office:meta><meta:generator>X</meta:generator><dc:title>T</dc:title><meta:creation-date>2026</meta:creation-date><dc:date>2026</dc:date><meta:editing-duration>PT1S</meta:editing-duration><meta:editing-cycles>2</meta:editing-cycles><meta:keyword>k</meta:keyword></office:meta>";
        assert_eq!(normalize_part("meta.xml", xml), "<office:meta><dc:title>T</dc:title><meta:keyword>k</meta:keyword></office:meta>");
        assert_eq!(element_text(xml, "meta:generator"), Some("X".into()));
    }

    // The [Content_Types].xml of the same saved docx from four platforms of
    // one run: the same entries in the order each build of x2t writes them.
    #[test]
    fn content_types_compare_their_entries_in_any_order() {
        let entry = |name: &str| -> String {
            let (kind, attr, value, ty) = match name {
                "bin" => ("Default", "Extension", "bin", "application/vnd.openxmlformats-officedocument.oleObject"),
                "jpeg" => ("Default", "Extension", "jpeg", "image/jpeg"),
                "rels" => ("Default", "Extension", "rels", "application/vnd.openxmlformats-package.relationships+xml"),
                "xml" => ("Default", "Extension", "xml", "application/xml"),
                "png" => ("Default", "Extension", "png", "image/png"),
                "wmf" => ("Default", "Extension", "wmf", "image/x-wmf"),
                "document" => ("Override", "PartName", "/word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"),
                "styles" => ("Override", "PartName", "/word/styles.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"),
                "theme" => ("Override", "PartName", "/word/theme/theme1.xml", "application/vnd.openxmlformats-officedocument.theme+xml"),
                "fontTable" => ("Override", "PartName", "/word/fontTable.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"),
                "webSettings" => ("Override", "PartName", "/word/webSettings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml"),
                "settings" => ("Override", "PartName", "/word/settings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"),
                "app" => ("Override", "PartName", "/docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml"),
                "core" => ("Override", "PartName", "/docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml"),
                other => panic!("{}", other),
            };
            format!("<{}  {}=\"{}\" ContentType=\"{}\"/>", kind, attr, value, ty)
        };
        let part = |order: &str| -> String {
            let entries: String = order.split(' ').map(entry).collect();
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">{}</Types>",
                entries
            )
        };
        let orders = [
            "bin jpeg rels xml png wmf document styles theme fontTable webSettings settings app core",
            "xml wmf bin rels jpeg png app theme fontTable document styles core settings webSettings",
            "wmf png xml jpeg rels bin app theme core webSettings settings styles fontTable document",
            "wmf png jpeg xml rels bin core app theme fontTable webSettings styles settings document",
        ];
        let normalized: Vec<String> = orders.iter().map(|o| normalize_part("[Content_Types].xml", &part(o))).collect();
        assert!(normalized.iter().all(|n| *n == normalized[0]), "{:#?}", normalized);
        // Another entry, or another content type, is still a difference.
        let fewer = normalize_part("[Content_Types].xml", &part("bin jpeg rels xml png wmf document styles theme fontTable webSettings settings app"));
        assert_ne!(fewer, normalized[0]);
        let retyped = part(orders[0]).replace("image/png", "image/x-png");
        assert_ne!(normalize_part("[Content_Types].xml", &retyped), normalized[0]);
    }

    #[test]
    fn other_parts_are_untouched() {
        assert_eq!(normalize_part("xl/styles.xml", "<a x=\"1\"/>"), "<a x=\"1\"/>");
    }

    #[test]
    fn docx_text_is_one_line_per_paragraph() {
        let xml = r#"<w:document><w:body><w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr><w:r><w:t>One</w:t></w:r><w:r><w:tab/><w:t xml:space="preserve"> two &amp; </w:t><w:br/><w:t>three</w:t></w:r></w:p><w:p/><w:p><w:r><w:t>Last</w:t></w:r></w:p></w:body></w:document>"#;
        // tidy also drops the space left before the break.
        assert_eq!(docx_text(xml), "One\t two &\nthree\n\nLast\n");
    }

    #[test]
    fn pptx_text_follows_the_slide_numbers() {
        let slide = |t: &str| format!("<p:sld><a:p><a:r><a:rPr lang=\"en\"/><a:t>{}</a:t></a:r><a:br/><a:r><a:t>x</a:t></a:r></a:p></p:sld>", t);
        let text = pptx_text(&[(10, slide("ten")), (2, slide("two"))]);
        assert_eq!(text, "# slide 2\ntwo\nx\n# slide 10\nten\nx\n");
    }

    #[test]
    fn xlsx_text_resolves_shared_strings() {
        let shared = "<sst><si><t>Item</t></si><si><r><t>Ri</t></r><r><t>ch</t></r><rPh><t>no</t></rPh></si></sst>";
        let sheet = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" s="2"/></row><row r="2"><c r="A2"><f>SUM(B2:B3)</f><v>3.5</v></c><c r="B2" t="inlineStr"><is><t>in &lt;line&gt;</t></is></c></row></sheetData></worksheet>"#;
        assert_eq!(xlsx_text(Some(shared), &[(1, sheet.to_string())]), "# sheet 1\nA1=Item\nB1=Rich\nA2=3.5\nB2=in <line>\n");
    }

    #[test]
    fn odf_text_expands_spaces_tabs_and_nested_paragraphs() {
        let xml = r#"<office:text><text:h text:outline-level="1">Head</text:h><text:p>a<text:s/>b<text:s text:c="3"/>c<text:tab/>d<text:span text:style-name="B">e</text:span><text:line-break/>f<text:note><text:note-citation>1</text:note-citation><text:note-body><text:p>Note</text:p></text:note-body></text:note></text:p><text:p/></office:text>"#;
        assert_eq!(odf_text(xml), "Head\nNote\na b   c\tde\nf1\n");
    }

    #[test]
    fn tidy_drops_trailing_blanks_and_keeps_page_breaks() {
        assert_eq!(tidy("a  \nb\t\n\x0c\n\n"), "a\nb\n\x0c\n");
        assert_eq!(tidy("\n\n"), "");
    }

    #[test]
    fn font_names_of_an_ooxml_part() {
        let theme = r#"<a:theme><a:fontScheme><a:majorFont><a:latin typeface="Liberation Sans"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Jpan" typeface="MS Gothic"/></a:majorFont><a:minorFont><a:latin typeface="Arial"/></a:minorFont></a:fontScheme></a:theme>"#;
        assert_eq!(
            ooxml_font_names(theme),
            FontNames { names: vec!["Arial".into(), "Liberation Sans".into()], scripts_ignored: 1 }
        );
        // ea and cs count like latin.
        let east = r#"<a:theme><a:latin typeface="Liberation Sans"/><a:ea typeface="SimSun"/><a:cs typeface="Mangal"/><a:font script="Arab" typeface="Times New Roman"/><a:font script="Hebr" typeface="Times New Roman"/></a:theme>"#;
        assert_eq!(
            ooxml_font_names(east),
            FontNames { names: vec!["Liberation Sans".into(), "Mangal".into(), "SimSun".into()], scripts_ignored: 2 }
        );
        let styles = r#"<styleSheet><fonts><font><sz val="10"/><name val="Carlito"/></font></fonts><cellStyles><cellStyle name="Normal"/></cellStyles></styleSheet>"#;
        assert_eq!(ooxml_font_names(styles).names, vec!["Carlito"]);
        let shared = r#"<sst><si><r><rPr><rFont val="Calibri"/></rPr><t>x</t></r></si></sst>"#;
        assert_eq!(ooxml_font_names(shared).names, vec!["Calibri"]);
        let sheet = r#"<worksheet><headerFooter><oddHeader>&amp;C&amp;"Times New Roman,Regular"&amp;12&amp;A</oddHeader></headerFooter></worksheet>"#;
        assert_eq!(ooxml_font_names(sheet).names, vec!["Times New Roman"]);
        let slide = r#"<p:sld><a:rPr><a:latin typeface="+mn-lt"/><a:sym typeface="Wingdings"/></a:rPr><a:rFont/><a:font typeface="Arial"/></p:sld>"#;
        assert_eq!(ooxml_font_names(slide), FontNames { names: vec!["Arial".into(), "Wingdings".into()], scripts_ignored: 0 });
    }

    #[test]
    fn entities_decode() {
        assert_eq!(decode("a &amp; b &lt;&gt; &#233;&#xE9; &bogus; & x"), "a & b <> éé &bogus; & x");
    }
}
