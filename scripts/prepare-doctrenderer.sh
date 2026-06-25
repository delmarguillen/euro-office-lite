#!/usr/bin/env bash
# Creates the directory structure that libdoctrenderer.so expects.
# DoctRenderer resolves JS files relative to the x2t binary:
#   ../editors/sdkjs/common/Native/native.js
#   ../editors/sdkjs/common/Native/jquery_native.js
#   ../editors/web-apps/vendor/xregexp/xregexp-all-min.js
#   ../editors/sdkjs/word/sdk-all-min.js  (or sdk-all.js)
#   ../editors/sdkjs/common/libfont/engine/fonts_native.js
#   ../editors/sdkjs/common/AllFonts.js
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SDKJS="$PROJECT_ROOT/src/sdkjs"
WEBAPPS="$PROJECT_ROOT/src/web-apps"
BINARIES="$PROJECT_ROOT/src-tauri/binaries"
EDITORS="$PROJECT_ROOT/src-tauri/editors"

echo "=== prepare-doctrenderer.sh ==="

rm -rf "$EDITORS"
mkdir -p "$EDITORS/sdkjs/common/Native"
mkdir -p "$EDITORS/sdkjs/common/libfont/engine"
mkdir -p "$EDITORS/sdkjs/word"
mkdir -p "$EDITORS/web-apps/vendor/xregexp"

cp "$SDKJS/common/Native/native.js" "$EDITORS/sdkjs/common/Native/"
cp "$SDKJS/common/Native/jquery_native.js" "$EDITORS/sdkjs/common/Native/"
cp "$SDKJS/common/AllFonts.js" "$EDITORS/sdkjs/common/AllFonts.js"

if [ -f "$SDKJS/common/libfont/engine/fonts_native.js" ]; then
    cp "$SDKJS/common/libfont/engine/fonts_native.js" "$EDITORS/sdkjs/common/libfont/engine/"
else
    echo "WARNING: fonts_native.js not found, creating stub"
    echo "// stub" > "$EDITORS/sdkjs/common/libfont/engine/fonts_native.js"
fi

cp "$WEBAPPS/vendor/xregexp/xregexp-all-min.js" "$EDITORS/web-apps/vendor/xregexp/"

if [ -f "$BINARIES/sdk-word-bundle.js" ]; then
    cp "$BINARIES/sdk-word-bundle.js" "$EDITORS/sdkjs/word/sdk-all-min.js"
else
    echo "ERROR: sdk-word-bundle.js not found, run generate-sdk-bundle.sh first"
    exit 1
fi

echo "DoctRenderer structure created:"
find "$EDITORS" -type f | while read -r f; do
    size=$(wc -c < "$f")
    echo "  ${f#$BINARIES/} ($size bytes)"
done

echo ""
echo "=== Verification ==="
echo "DoctRenderer.config contents:"
cat "$BINARIES/DoctRenderer.config" 2>/dev/null || echo "NOT FOUND"
echo ""
echo "All files in editors/ with sizes:"
find "$EDITORS" -type f -exec ls -la {} \;
echo ""
echo "JS and config files in binaries/:"
ls -la "$BINARIES"/*.js "$BINARIES"/*.config "$BINARIES"/*.bin 2>/dev/null || true
echo ""
echo "dictionaries/ directory:"
DICT_DIR="$(dirname "$BINARIES")/dictionaries"
if [ -d "$DICT_DIR" ]; then
    echo "  exists at $DICT_DIR"
else
    echo "  NOT FOUND at $DICT_DIR"
fi
