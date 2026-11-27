# Fallback fonts

Two fonts built into the app (`src/fallback_fonts.rs`). The font scan gets
them so a symbol or an emoji that no other font draws still has a glyph.

| File | Family | Glyphs from | Code points | License |
|---|---|---|---|---|
| `EuroOfficeSigns.ttf` | Euro-Office Signs | DejaVu Sans 2.37 | 1325 in U+2190-2BFF | `LICENSE-DejaVu.txt` |
| `EuroOfficeEmoji.ttf` | Euro-Office Emoji | Noto Emoji 3.002, weight 400 | 1242 in U+1F000-1FAFF | `LICENSE-NotoEmoji.txt` (SIL OFL 1.1) |

Both are subsets with black and white outlines, renamed as their licenses ask
of a changed font. Neither has a letter, nor a code point that a font in
`src/fonts` has, and their panose is empty.

They are rebuilt, byte for byte, with fontTools 4.69.1:

```
python3 scripts/build-fallback-fonts.py <download folder outside the repo>
```
