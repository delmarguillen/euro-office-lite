// Fonts the app adds to x2t's font scan so a symbol or an emoji no other font
// draws still has a glyph. Neither has a letter, nor a code point a font of
// binaries/fonts has (scripts/build-fallback-fonts.py). Never names crate::.

use std::path::{Path, PathBuf};

// The folder of a font cache generation the scan reads them from.
pub const DIR_NAME: &str = "fallback-fonts";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackFont {
    // Glyphs of DejaVu Sans in U+2190-2BFF.
    Signs,
    // Glyphs of Noto Emoji in U+1F000-1FAFF.
    Emoji,
}

pub const ALL: [FallbackFont; 2] = [FallbackFont::Signs, FallbackFont::Emoji];

impl FallbackFont {
    pub fn file_name(self) -> &'static str {
        match self {
            FallbackFont::Signs => "EuroOfficeSigns.ttf",
            FallbackFont::Emoji => "EuroOfficeEmoji.ttf",
        }
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            FallbackFont::Signs => include_bytes!("../fallback-fonts/EuroOfficeSigns.ttf"),
            FallbackFont::Emoji => include_bytes!("../fallback-fonts/EuroOfficeEmoji.ttf"),
        }
    }

    // Code points of the font; a scan that leaves one without a font needs it.
    fn probes(self) -> &'static [u32] {
        match self {
            // ✓ ★ ✔ ☎ ⚠
            FallbackFont::Signs => &[0x2713, 0x2605, 0x2714, 0x260E, 0x26A0],
            // 😀 👍
            FallbackFont::Emoji => &[0x1F600, 0x1F44D],
        }
    }
}

// The fonts the first scan gets on `os` (std::env::consts::OS). x2t leaves out
// a font FreeType calls not scalable (CFontList::Add in ONLYOFFICE core), as
// it does one with an sbix or CBDT table: the emoji fonts of macOS and Linux.
pub fn scanned_first(os: &str) -> &'static [FallbackFont] {
    match os {
        "linux" | "macos" => &[FallbackFont::Emoji],
        _ => &[],
    }
}

// The fonts not scanned yet that the scan behind `all_fonts_js` needs: those
// with a probe outside every range of its __fonts_ranges. None when the list
// cannot be read, so the scan stays as it was.
pub fn still_needed(scanned: &[FallbackFont], all_fonts_js: &str) -> Vec<FallbackFont> {
    let Some(ranges) = covered_ranges(all_fonts_js) else {
        return Vec::new();
    };
    let mut needed = Vec::new();
    for font in ALL {
        if scanned.contains(&font) {
            continue;
        }
        if all_covered(&ranges, font.probes()) {
            continue;
        }
        needed.push(font);
    }
    needed
}

// The (first, last) of each triple in
// window["__fonts_ranges"] = [first,last,font, ...]: the code points for which
// x2t found a font.
fn covered_ranges(all_fonts_js: &str) -> Option<Vec<(u32, u32)>> {
    let marker = "window[\"__fonts_ranges\"]";
    let start = all_fonts_js.find(marker)?;
    let rest = &all_fonts_js[start + marker.len()..];
    let open = rest.find('[')?;
    let list = &rest[open + 1..];
    let close = list.find(']')?;
    let mut numbers = Vec::new();
    for field in list[..close].split(',') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        numbers.push(field.parse::<u32>().ok()?);
    }
    if numbers.len() % 3 != 0 {
        return None;
    }
    let ranges = numbers.chunks_exact(3).map(|triple| (triple[0], triple[1]));
    Some(ranges.collect())
}

fn all_covered(ranges: &[(u32, u32)], code_points: &[u32]) -> bool {
    code_points.iter().all(|code_point| is_covered(ranges, *code_point))
}

fn is_covered(ranges: &[(u32, u32)], code_point: u32) -> bool {
    ranges
        .iter()
        .any(|(first, last)| (*first..=*last).contains(&code_point))
}

// Writes `fonts` into the generation's fallback folder and returns the
// folder. x2t records the path of each font it lists, so they stay there for
// as long as the generation does.
pub fn write_into(generation_dir: &Path, fonts: &[FallbackFont]) -> std::io::Result<PathBuf> {
    let dir = generation_dir.join(DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    for font in fonts {
        std::fs::write(dir.join(font.file_name()), font.bytes())?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn scratch_dir(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "eo-fallback-fonts-{}-{}-{}",
            name,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // An AllFonts.js whose scan found a font for the given ranges.
    fn all_fonts_js(ranges: &[(u32, u32)]) -> String {
        let triples: Vec<String> = ranges
            .iter()
            .map(|(first, last)| format!("{},{},0", first, last))
            .collect();
        format!(
            "window[\"__fonts_files\"] = [\n\"/f/A.ttf\"\n];\n\nwindow[\"__fonts_infos\"] = [\n[\"A\",0,0,-1,0,-1,0,-1,0]\n];\n\nwindow[\"__fonts_ranges\"] = [\n{}];\n\nwindow[\"g_fonts_selection_bin\"] = \"\";\n",
            triples.join(",")
        )
    }

    // The code points a TrueType font maps to a glyph, from its format 12
    // cmap subtable or, without one, its format 4 one.
    fn code_points(font: &[u8]) -> BTreeSet<u32> {
        let cmap = table(font, b"cmap").expect("cmap table");
        let count = u16_at(cmap, 2) as usize;
        let mut format4 = None;
        for index in 0..count {
            let offset = u32_at(cmap, 4 + index * 8 + 4) as usize;
            let subtable = &cmap[offset..];
            match u16_at(subtable, 0) {
                12 => return format12_code_points(subtable),
                4 => format4 = Some(subtable),
                _ => {}
            }
        }
        format4_code_points(format4.expect("a format 4 or 12 cmap subtable"))
    }

    fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
        let count = u16_at(font, 4) as usize;
        for index in 0..count {
            let record = &font[12 + index * 16..];
            if &record[..4] != tag {
                continue;
            }
            let offset = u32_at(record, 8) as usize;
            let length = u32_at(record, 12) as usize;
            return Some(&font[offset..offset + length]);
        }
        None
    }

    fn format12_code_points(subtable: &[u8]) -> BTreeSet<u32> {
        let groups = u32_at(subtable, 12) as usize;
        let mut found = BTreeSet::new();
        for group in 0..groups {
            let first = u32_at(subtable, 16 + group * 12);
            let last = u32_at(subtable, 16 + group * 12 + 4);
            let glyph = u32_at(subtable, 16 + group * 12 + 8);
            found.extend((first..=last).filter(|code| glyph + (code - first) != 0));
        }
        found
    }

    fn format4_code_points(subtable: &[u8]) -> BTreeSet<u32> {
        let segments = u16_at(subtable, 6) as usize / 2;
        let ends = 14;
        let starts = ends + segments * 2 + 2;
        let deltas = starts + segments * 2;
        let offsets = deltas + segments * 2;
        let mut found = BTreeSet::new();
        for segment in 0..segments {
            let first = u16_at(subtable, starts + segment * 2);
            let last = u16_at(subtable, ends + segment * 2);
            let delta = u16_at(subtable, deltas + segment * 2);
            let offset_at = offsets + segment * 2;
            let offset = u16_at(subtable, offset_at) as usize;
            for code in first..=last {
                if code == 0xFFFF {
                    continue;
                }
                let glyph = if offset == 0 {
                    code.wrapping_add(delta)
                } else {
                    let stored = u16_at(subtable, offset_at + offset + (code - first) as usize * 2);
                    if stored == 0 {
                        0
                    } else {
                        stored.wrapping_add(delta)
                    }
                };
                if glyph != 0 {
                    found.insert(u32::from(code));
                }
            }
        }
        found
    }

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([bytes[at], bytes[at + 1]])
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    // The regular face of each family in src/fonts.
    fn bundled_code_points() -> BTreeSet<u32> {
        let faces: [&[u8]; 4] = [
            include_bytes!("../../src/fonts/LiberationSans-Regular.ttf"),
            include_bytes!("../../src/fonts/LiberationSerif-Regular.ttf"),
            include_bytes!("../../src/fonts/LiberationMono-Regular.ttf"),
            include_bytes!("../../src/fonts/Carlito-Regular.ttf"),
        ];
        let mut found = BTreeSet::new();
        for face in faces {
            found.extend(code_points(face));
        }
        found
    }

    // ✓ ☺ 😀 ★ → € ♥ ✔ ☎ ⚠
    const REPORTED: [u32; 10] = [
        0x2713, 0x263A, 0x1F600, 0x2605, 0x2192, 0x20AC, 0x2665, 0x2714, 0x260E, 0x26A0,
    ];

    #[test]
    fn every_reported_symbol_has_a_bundled_or_a_fallback_font() {
        let mut drawn = bundled_code_points();
        for font in ALL {
            drawn.extend(code_points(font.bytes()));
        }
        for code_point in REPORTED {
            assert!(drawn.contains(&code_point), "U+{:04X}", code_point);
        }
    }

    #[test]
    fn the_bundled_fonts_alone_lack_six_of_the_reported_symbols() {
        let bundled = bundled_code_points();
        let missing: Vec<u32> = REPORTED
            .into_iter()
            .filter(|code_point| !bundled.contains(code_point))
            .collect();
        assert_eq!(missing, vec![0x2713, 0x1F600, 0x2605, 0x2714, 0x260E, 0x26A0]);
    }

    #[test]
    fn a_fallback_font_has_no_code_point_a_bundled_font_has() {
        let bundled = bundled_code_points();
        for font in ALL {
            let shared: Vec<u32> = code_points(font.bytes())
                .intersection(&bundled)
                .copied()
                .collect();
            assert!(shared.is_empty(), "{:?} shares {:x?}", font, shared);
        }
    }

    #[test]
    fn each_fallback_font_stays_in_its_block() {
        let signs = code_points(FallbackFont::Signs.bytes());
        let emoji = code_points(FallbackFont::Emoji.bytes());
        assert!(signs.iter().all(|code| (0x2190..=0x2BFF).contains(code)));
        assert!(emoji.iter().all(|code| (0x1F000..=0x1FAFF).contains(code)));
        assert!(signs.len() > 1000, "{}", signs.len());
        assert!(emoji.len() > 1000, "{}", emoji.len());
    }

    #[test]
    fn each_font_has_its_own_probes() {
        for font in ALL {
            let drawn = code_points(font.bytes());
            for probe in font.probes() {
                assert!(drawn.contains(probe), "{:?} lacks U+{:04X}", font, probe);
            }
        }
    }

    #[test]
    fn only_linux_and_macos_scan_the_emoji_font_first() {
        assert_eq!(scanned_first("linux"), [FallbackFont::Emoji]);
        assert_eq!(scanned_first("macos"), [FallbackFont::Emoji]);
        assert!(scanned_first("windows").is_empty());
        assert!(scanned_first("freebsd").is_empty());
    }

    #[test]
    fn a_scan_that_found_a_font_for_every_probe_needs_no_fallback() {
        let found = all_fonts_js(&[(0x20, 0x7E), (0x2190, 0x2BFF), (0x1F300, 0x1F6FF)]);
        assert!(still_needed(&[], &found).is_empty());
    }

    #[test]
    fn a_scan_without_a_font_for_one_symbol_needs_the_signs_font() {
        // Everything but U+26A0.
        let found = all_fonts_js(&[(0x2190, 0x269F), (0x26A1, 0x2BFF), (0x1F300, 0x1F6FF)]);
        assert_eq!(still_needed(&[], &found), [FallbackFont::Signs]);
    }

    #[test]
    fn a_scan_without_a_font_for_one_emoji_needs_the_emoji_font() {
        // U+1F600 is there, as DejaVu Sans has it; U+1F44D is not.
        let found = all_fonts_js(&[(0x2190, 0x2BFF), (0x1F600, 0x1F64F)]);
        assert_eq!(still_needed(&[], &found), [FallbackFont::Emoji]);
    }

    #[test]
    fn a_scan_with_only_text_fonts_needs_both() {
        let found = all_fonts_js(&[(0x20, 0x7E), (0xA0, 0x17F)]);
        assert_eq!(still_needed(&[], &found), ALL);
    }

    #[test]
    fn a_font_already_scanned_is_not_asked_for_again() {
        let found = all_fonts_js(&[(0x20, 0x7E), (0x1F000, 0x1FAFF)]);
        assert_eq!(
            still_needed(&[FallbackFont::Emoji], &found),
            [FallbackFont::Signs]
        );
        assert!(still_needed(&ALL, &all_fonts_js(&[(0x20, 0x7E)])).is_empty());
    }

    #[test]
    fn a_list_without_ranges_asks_for_nothing() {
        assert!(still_needed(&[], "window[\"__fonts_files\"] = [];").is_empty());
        assert!(still_needed(&[], "window[\"__fonts_ranges\"] = [1,2").is_empty());
        assert!(still_needed(&[], "window[\"__fonts_ranges\"] = [1,2,0,3];").is_empty());
        assert!(still_needed(&[], "window[\"__fonts_ranges\"] = [1,x,0];").is_empty());
    }

    #[test]
    fn the_ranges_are_read_as_x2t_writes_them() {
        let text = "\u{feff}window[\"__fonts_infos\"] = [\n[\"A\",0,0,-1,0,-1,0,-1,0]\n];\n\nwindow[\"__fonts_ranges\"] = [\n0,0,4,32,126,4,9985,10175,1];\n\nwindow[\"g_fonts_selection_bin\"] = \"EgAA\";\n";
        assert_eq!(
            covered_ranges(text),
            Some(vec![(0, 0), (32, 126), (9985, 10175)])
        );
    }

    #[test]
    fn the_fonts_are_written_into_the_fallback_folder_of_the_generation() {
        let generation = scratch_dir("write");

        let dir = write_into(&generation, &[FallbackFont::Emoji]).unwrap();

        assert_eq!(dir, generation.join("fallback-fonts"));
        let emoji = std::fs::read(dir.join("EuroOfficeEmoji.ttf")).unwrap();
        assert_eq!(emoji, FallbackFont::Emoji.bytes());
        assert!(!dir.join("EuroOfficeSigns.ttf").exists());

        write_into(&generation, &ALL).unwrap();
        let signs = std::fs::read(dir.join("EuroOfficeSigns.ttf")).unwrap();
        assert_eq!(signs, FallbackFont::Signs.bytes());
        let _ = std::fs::remove_dir_all(&generation);
    }
}
