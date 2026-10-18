#!/usr/bin/env bash
# Euro-Office Lite - Font preparation script (macOS/Linux)
# Downloads Liberation Sans/Serif/Mono + Carlito, copies TTFs, generates AllFonts.js
# Equivalent of prepare-fonts.ps1 for non-Windows platforms.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FONTS_DIR="$PROJECT_ROOT/src/fonts"
ALLFONTS_PATH="$PROJECT_ROOT/src/sdkjs/common/AllFonts.js"
TEMP_DIR="${TMPDIR:-/tmp}/euro-office-fonts"

LIBERATION_URL="https://github.com/liberationfonts/liberation-fonts/files/7261482/liberation-fonts-ttf-2.1.5.tar.gz"
# Carlito 1.104; a commit, not a branch, so the fonts cannot change under us.
CARLITO_URL_BASE="https://github.com/googlefonts/carlito/raw/3a810cab78ebd6e2e4eed42af9e8453c4f9b850a/fonts/ttf"

# sha256 of every downloaded file; must match the table in prepare-fonts.ps1.
LIBERATION_TAR_SHA256="7191c669bf38899f73a2094ed00f7b800553364f90e2637010a69c0e268f25d0"
FONT_SHA256S="76d04c18ea243f426b7de1f3ad208e927008f961dc5945e5aad352d0dfde8ee8  LiberationSans-Regular.ttf
e5bae5c4cde31f22142753855f4f8fb86da6ff39955ed3c0a11248b0d16948b0  LiberationSans-Italic.ttf
788abee4c806d660e8aee46689dd8540cd4bb98da03dcc9d171ce3efd99a9173  LiberationSans-Bold.ttf
698da70fc191cc5f33ad4d6d3fe830fe4624b898ea2e3169955928b7c491f1ee  LiberationSans-BoldItalic.ttf
058ea80864aef09a23f45cbec2bb5400bc3dfbdea01c3f10538a21fcb497fb74  LiberationSerif-Regular.ttf
0e3dea9f8d613e006ccfa62201f33e265d19167bd0907725c3e145368b04fc2e  LiberationSerif-Italic.ttf
d754ba427cfe0bca54ae052384baa8f842da5bd6550ad4da024ac441e7a7d5ce  LiberationSerif-Bold.ttf
f17db8af71e24d2066b587546021d4f0b296be389512b658dec3c09affeb11a7  LiberationSerif-BoldItalic.ttf
f2b83c763e8afd21709333370bed4774337fae82267937e2b5aea7e2fbd922c1  LiberationMono-Regular.ttf
605c01c711b44480a7508d349dfbf3264e81fa43d69e61cfa7d10b86e764c4d1  LiberationMono-Italic.ttf
bd62a0672d0b9b6710b01df434c80ad54fa5f0835207eb7b17b7a761463067bb  LiberationMono-Bold.ttf
79451f3c09fe25116098853b7a2ca6e2436220ccc11af022979adbcf195be130  LiberationMono-BoldItalic.ttf
f6418f708baede9789daef5d458c0f53d2a888af9820e8062934e504fedc6595  Carlito-Regular.ttf
0b019225e58d702bfedcbd35c21696769f8ee115cb6343f84c2f240312450d1c  Carlito-Italic.ttf
bb5d20f79b82599ec72983597437373a80f2d2085fa91fc144fd74e876a594db  Carlito-Bold.ttf
b32928186c119599e03ca6a1ffc680fdcb7fac95772f4b95d989cf6cd3861517  Carlito-BoldItalic.ttf"

# Reads "<sha256>  <file>" lines on stdin and checks them inside directory $1.
sha256_check() {
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$1" && sha256sum -c -)
    else
        (cd "$1" && shasum -a 256 -c -)
    fi
}

echo ""
echo "=== Euro-Office Lite - Font Preparation ==="

# 1. Download Liberation fonts
mkdir -p "$TEMP_DIR"
TAR_GZ="$TEMP_DIR/liberation-fonts.tar.gz"

if [ ! -f "$TAR_GZ" ]; then
    echo "[1/6] Downloading Liberation fonts..."
    curl -fL --retry 3 -o "$TAR_GZ" "$LIBERATION_URL"
else
    echo "[1/6] Using cached Liberation download..."
fi
if ! echo "$LIBERATION_TAR_SHA256  $(basename "$TAR_GZ")" | sha256_check "$TEMP_DIR"; then
    rm -f "$TAR_GZ"
    echo "ERROR: sha256 mismatch for the Liberation archive"
    exit 1
fi

# 2. Extract
echo "[2/6] Extracting Liberation fonts..."
EXTRACT_DIR="$TEMP_DIR/extracted"
rm -rf "$EXTRACT_DIR"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$TAR_GZ" -C "$EXTRACT_DIR"

TTF_DIR=$(find "$EXTRACT_DIR" -type d -name "liberation-fonts-ttf-*" | head -1)
if [ -z "$TTF_DIR" ]; then
    echo "ERROR: Could not find extracted font directory"
    exit 1
fi

# 3. Download Carlito fonts
echo "[3/6] Downloading Carlito fonts..."
CARLITO_DIR="$TEMP_DIR/carlito"
mkdir -p "$CARLITO_DIR"

CARLITO_FILES=("Carlito-Regular.ttf" "Carlito-Italic.ttf" "Carlito-Bold.ttf" "Carlito-BoldItalic.ttf")
for cf in "${CARLITO_FILES[@]}"; do
    dest="$CARLITO_DIR/$cf"
    if [ ! -f "$dest" ]; then
        echo "  Downloading: $cf"
        curl -fL --retry 3 -o "$dest" "$CARLITO_URL_BASE/$cf"
    else
        echo "  Cached: $cf"
    fi
done

# 4. Copy all TTF files to src/fonts/
echo "[4/6] Copying fonts..."
mkdir -p "$FONTS_DIR"

LIBERATION_FILES=(
    "LiberationSans-Regular.ttf" "LiberationSans-Italic.ttf"
    "LiberationSans-Bold.ttf" "LiberationSans-BoldItalic.ttf"
    "LiberationSerif-Regular.ttf" "LiberationSerif-Italic.ttf"
    "LiberationSerif-Bold.ttf" "LiberationSerif-BoldItalic.ttf"
    "LiberationMono-Regular.ttf" "LiberationMono-Italic.ttf"
    "LiberationMono-Bold.ttf" "LiberationMono-BoldItalic.ttf"
)

for f in "${LIBERATION_FILES[@]}"; do
    cp "$TTF_DIR/$f" "$FONTS_DIR/$f"
    echo "  Copied: $f"
done

for f in "${CARLITO_FILES[@]}"; do
    cp "$CARLITO_DIR/$f" "$FONTS_DIR/$f"
    echo "  Copied: $f"
done

if ! echo "$FONT_SHA256S" | sha256_check "$FONTS_DIR"; then
    echo "ERROR: sha256 mismatch in $FONTS_DIR (a stale download in $TEMP_DIR can cause this)"
    exit 1
fi

# 5. Generate AllFonts.js
echo "[5/6] Generating AllFonts.js..."

ALL_FILES=(
    "${LIBERATION_FILES[@]}"
    "${CARLITO_FILES[@]}"
)

# Build __fonts_files array
FILES_JSON=""
for i in "${!ALL_FILES[@]}"; do
    if [ "$i" -gt 0 ]; then FILES_JSON+=", "; fi
    FILES_JSON+="\"${ALL_FILES[$i]}\""
done

# Font family index mapping (0-based indices into ALL_FILES)
# Liberation Sans: 0-3, Liberation Serif: 4-7, Liberation Mono: 8-11, Carlito: 12-15
cat > "$ALLFONTS_PATH" << 'HEREDOC_EOF'
// Generated by scripts/prepare-fonts.sh -- do not edit manually.
HEREDOC_EOF

cat >> "$ALLFONTS_PATH" << EOF
window["__fonts_files"] = [$FILES_JSON];
window["__fonts_infos"] = [
    ["Liberation Sans", 0, 0, 1, 0, 2, 0, 3, 0],
    ["Liberation Serif", 4, 0, 5, 0, 6, 0, 7, 0],
    ["Liberation Mono", 8, 0, 9, 0, 10, 0, 11, 0],
    ["Carlito", 12, 0, 13, 0, 14, 0, 15, 0],
    ["Arial", 0, 0, 1, 0, 2, 0, 3, 0],
    ["Helvetica", 0, 0, 1, 0, 2, 0, 3, 0],
    ["Times New Roman", 4, 0, 5, 0, 6, 0, 7, 0],
    ["Courier New", 8, 0, 9, 0, 10, 0, 11, 0],
    ["Calibri", 12, 0, 13, 0, 14, 0, 15, 0],
    ["Cambria", 4, 0, 5, 0, 6, 0, 7, 0]
];
window["g_fonts_selection_bin"] = "";
EOF

# 6. Generate font thumbnail sprites
echo "[6/6] Generating font thumbnail sprites..."
SPRITE_SCRIPT="$PROJECT_ROOT/scripts/generate-font-sprite.py"
if [ -f "$SPRITE_SCRIPT" ]; then
    python3 "$SPRITE_SCRIPT"
else
    echo "WARNING: generate-font-sprite.py not found, skipping sprite generation"
fi

echo ""
echo "=== Font preparation complete ==="
echo "Fonts in: $FONTS_DIR"
echo "AllFonts.js updated: $ALLFONTS_PATH"
echo ""
