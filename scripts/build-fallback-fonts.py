#!/usr/bin/env python3
"""Builds the two fallback fonts in src-tauri/fallback-fonts/.

  EOLiteSigns.ttf    DejaVu Sans 2.37, U+2190-2BFF
  EOLiteEmoji.ttf    Noto Emoji 3.002 at weight 400, U+1F000-1FAFF

Both leave out every code point a font in src/fonts already has. Needs
fontTools 4.69.1; another version may write different bytes.

  python3 scripts/build-fallback-fonts.py <download folder outside the repo>
"""

import hashlib
import sys
import tarfile
import urllib.request
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

FONTTOOLS_VERSION = "4.69.1"

DEJAVU_URL = "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.tar.bz2"
DEJAVU_SHA256 = "fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7"
DEJAVU_MEMBER = "dejavu-fonts-ttf-2.37/ttf/DejaVuSans.ttf"

# A commit of google/fonts, not a branch, so the font cannot change under us.
NOTO_EMOJI_URL = "https://raw.githubusercontent.com/google/fonts/5026800714d07231937d92eb4cceabf06e05d74e/ofl/notoemoji/NotoEmoji%5Bwght%5D.ttf"
NOTO_EMOJI_SHA256 = "de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551"

SYMBOLS_RANGE = range(0x2190, 0x2C00)
EMOJI_RANGE = range(0x1F000, 0x1FB00)

REPO = Path(__file__).resolve().parent.parent
BUNDLED_DIR = REPO / "src" / "fonts"
OUT_DIR = REPO / "src-tauri" / "fallback-fonts"


def sha256_of(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def download(url, sha256, target):
    if not target.exists():
        print(f"Downloading {url}")
        urllib.request.urlretrieve(url, target)
    actual = sha256_of(target)
    if actual != sha256:
        sys.exit(f"sha256 mismatch for {target}: {actual}")
    return target


def bundled_code_points():
    covered = set()
    for path in sorted(BUNDLED_DIR.glob("*.ttf")):
        covered.update(TTFont(path, lazy=True).getBestCmap())
    return covered


def wanted_code_points(font, code_range, bundled):
    cmap = font.getBestCmap()
    return sorted(cp for cp in cmap if cp in code_range and cp not in bundled)


def keep_only(font, code_points):
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.notdef_outline = True
    options.glyph_names = False
    options.recalc_timestamp = False
    # FontForge's own table; the subsetter cannot rewrite it.
    options.drop_tables += ["FFTM"]
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=code_points)
    subsetter.subset(font)


# The licenses of both sources ask a changed font to carry another name. The
# copyright (0) and license (13, 14) records stay as they are. No family is
# "... Symbols": for a missing font the editor prefers a family whose name
# holds the missing one's (GetFaceNamePenalty_private in sdkjs), and "Symbol"
# is a font.
def rename(font, family, postscript, source):
    names = {
        1: family,
        2: "Regular",
        3: f"{postscript};{source}",
        4: family,
        6: postscript,
        16: None,
        17: None,
        18: None,
    }
    table = font["name"]
    for record in list(table.names):
        if record.nameID not in names:
            continue
        text = names[record.nameID]
        if text is None:
            table.removeNames(nameID=record.nameID)
        else:
            record.string = text
    table.setName(f"Subset of {source}", 10, 3, 1, 0x409)


# x2t picks the font for a missing family by its panose first (GetPanosePenalty
# in ONLYOFFICE core). With none, a text font always scores better than these.
def clear_panose(font):
    panose = font["OS/2"].panose
    for field in vars(panose):
        setattr(panose, field, 0)


def save(font, name):
    clear_panose(font)
    font.recalcTimestamp = False
    target = OUT_DIR / name
    font.save(target)
    print(f"{sha256_of(target)}  {name}  {target.stat().st_size} bytes  {len(font.getBestCmap())} code points")


def build_signs(downloads, bundled):
    archive = download(DEJAVU_URL, DEJAVU_SHA256, downloads / "dejavu-fonts-ttf-2.37.tar.bz2")
    with tarfile.open(archive) as tar:
        source = tar.extractfile(DEJAVU_MEMBER)
        font = TTFont(source)
        keep_only(font, wanted_code_points(font, SYMBOLS_RANGE, bundled))
    rename(font, "EOLite Signs", "EOLiteSigns", "DejaVu Sans 2.37")
    save(font, "EOLiteSigns.ttf")


def build_emoji(downloads, bundled):
    source = download(NOTO_EMOJI_URL, NOTO_EMOJI_SHA256, downloads / "NotoEmoji-wght.ttf")
    font = instancer.instantiateVariableFont(TTFont(source), {"wght": 400})
    keep_only(font, wanted_code_points(font, EMOJI_RANGE, bundled))
    rename(font, "EOLite Emoji", "EOLiteEmoji", "Noto Emoji 3.002")
    save(font, "EOLiteEmoji.ttf")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    if fontTools.version != FONTTOOLS_VERSION:
        sys.exit(f"fontTools {FONTTOOLS_VERSION} is required, found {fontTools.version}")
    downloads = Path(sys.argv[1]).resolve()
    if REPO in downloads.parents or downloads == REPO:
        sys.exit("the download folder must be outside the repository")
    downloads.mkdir(parents=True, exist_ok=True)
    OUT_DIR.mkdir(exist_ok=True)
    bundled = bundled_code_points()
    build_signs(downloads, bundled)
    build_emoji(downloads, bundled)


if __name__ == "__main__":
    main()
