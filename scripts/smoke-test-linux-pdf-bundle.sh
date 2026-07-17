#!/usr/bin/env bash
# Verifies that a packaged Linux resource tree can execute the DoctRenderer
# path with the exact x2t libraries and sdkjs bundles that will be released.
set -euo pipefail

APP_ROOT="${1:?Usage: smoke-test-linux-pdf-bundle.sh <packaged-app-root>}"
BINARIES="$APP_ROOT/binaries"
EDITORS="$APP_ROOT/editors"
TEMPLATES=(
    "$APP_ROOT/templates/blank.docx"
    "$APP_ROOT/templates/blank.xlsx"
    "$APP_ROOT/templates/blank.pptx"
)

required=(
    "$BINARIES/x2t"
    "$BINARIES/AllFonts.js"
    "$EDITORS/sdkjs/common/Native/native.js"
    "$EDITORS/sdkjs/common/Native/jquery_native.js"
    "$EDITORS/sdkjs/common/libfont/engine/fonts_native.js"
    "$EDITORS/sdkjs/word/sdk-all-min.js"
    "$EDITORS/sdkjs/word/sdk-all.js"
    "$EDITORS/sdkjs/cell/sdk-all-min.js"
    "$EDITORS/sdkjs/cell/sdk-all.js"
    "$EDITORS/sdkjs/slide/sdk-all-min.js"
    "$EDITORS/sdkjs/slide/sdk-all.js"
    "$EDITORS/web-apps/vendor/xregexp/xregexp-all-min.js"
    "${TEMPLATES[@]}"
)

for path in "${required[@]}"; do
    if [ ! -s "$path" ]; then
        echo "ERROR: Missing packaged PDF resource: $path"
        exit 1
    fi
done

WORK_DIR=$(mktemp -d "${RUNNER_TEMP:-/tmp}/eol-pdf-smoke.XXXXXX")
trap 'rm -rf "$WORK_DIR"' EXIT

WORK_BINARIES="$WORK_DIR/binaries"
WORK_EDITORS="$WORK_DIR/editors"
mkdir -p \
    "$WORK_BINARIES" \
    "$WORK_EDITORS/sdkjs/common/Native" \
    "$WORK_EDITORS/sdkjs/common/libfont/engine" \
    "$WORK_EDITORS/sdkjs/word" \
    "$WORK_EDITORS/sdkjs/cell" \
    "$WORK_EDITORS/sdkjs/slide" \
    "$WORK_EDITORS/web-apps/vendor/xregexp" \
    "$WORK_DIR/dictionaries" \
    "$WORK_DIR/tmp"

ln -s "$BINARIES/x2t" "$WORK_BINARIES/x2t"
shopt -s nullglob
for file in "$BINARIES"/*.so "$BINARIES"/*.so.* "$BINARIES"/*.dat; do
    ln -s "$file" "$WORK_BINARIES/$(basename "$file")"
done
shopt -u nullglob

for optional in package.config fonts; do
    if [ -e "$BINARIES/$optional" ]; then
        ln -s "$BINARIES/$optional" "$WORK_BINARIES/$optional"
    fi
done
if [ -s "$BINARIES/font_selection.bin" ]; then
    cp "$BINARIES/font_selection.bin" "$WORK_BINARIES/font_selection.bin"
fi

cp "$BINARIES/AllFonts.js" "$WORK_BINARIES/AllFonts.js"
cp "$BINARIES/AllFonts.js" "$WORK_EDITORS/sdkjs/common/AllFonts.js"

cat > "$WORK_BINARIES/DoctRenderer.config" <<'CONFIG'
<Settings>
<file>../editors/sdkjs/common/Native/native.js</file>
<file>../editors/sdkjs/common/Native/jquery_native.js</file>
<allfonts>../editors/sdkjs/common/AllFonts.js</allfonts>
<file>../editors/web-apps/vendor/xregexp/xregexp-all-min.js</file>
<sdkjs>../editors/sdkjs</sdkjs>
<dictionaries>../dictionaries</dictionaries>
<DoctSdk>
<file>../editors/sdkjs/word/sdk-all-min.js</file>
<file>../editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>../editors/sdkjs/word/sdk-all.js</file>
</DoctSdk>
<PpttSdk>
<file>../editors/sdkjs/slide/sdk-all-min.js</file>
<file>../editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>../editors/sdkjs/slide/sdk-all.js</file>
</PpttSdk>
<XlstSdk>
<file>../editors/sdkjs/cell/sdk-all-min.js</file>
<file>../editors/sdkjs/common/libfont/engine/fonts_native.js</file>
<file>../editors/sdkjs/cell/sdk-all.js</file>
</XlstSdk>
</Settings>
CONFIG

for relative in \
    sdkjs/common/Native/native.js \
    sdkjs/common/Native/jquery_native.js \
    sdkjs/common/libfont/engine/fonts_native.js \
    sdkjs/word/sdk-all-min.js \
    sdkjs/word/sdk-all.js \
    sdkjs/cell/sdk-all-min.js \
    sdkjs/cell/sdk-all.js \
    sdkjs/slide/sdk-all-min.js \
    sdkjs/slide/sdk-all.js \
    web-apps/vendor/xregexp/xregexp-all-min.js; do
    ln -s "$EDITORS/$relative" "$WORK_EDITORS/$relative"
done

for template in "${TEMPLATES[@]}"; do
    extension="${template##*.}"
    output="$WORK_DIR/output-$extension.pdf"
    params="$WORK_DIR/pdf-$extension.xml"

    cat > "$params" <<PARAMS
<?xml version="1.0" encoding="utf-8"?>
<TaskQueueDataConvert xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<m_sFileFrom>$template</m_sFileFrom>
<m_sFileTo>$output</m_sFileTo>
<m_nFormatTo>513</m_nFormatTo>
<m_bEmbeddedFonts>true</m_bEmbeddedFonts>
<m_sFontDir>$BINARIES/fonts</m_sFontDir>
<m_sAllFontsPath>$WORK_BINARIES/AllFonts.js</m_sAllFontsPath>
<m_sTempDir>$WORK_DIR/tmp</m_sTempDir>
</TaskQueueDataConvert>
PARAMS

    echo "Running packaged DoctRenderer PDF smoke test for $extension..."
    (
        cd "$WORK_BINARIES"
        LD_LIBRARY_PATH="$WORK_BINARIES:$BINARIES" ./x2t "$params"
    )

    if [ ! -s "$output" ]; then
        echo "ERROR: x2t exited successfully but did not produce a PDF for $extension"
        exit 1
    fi

    echo "PDF smoke test passed for $extension ($(wc -c < "$output") bytes)"
done
