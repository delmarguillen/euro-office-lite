#!/bin/sh
# Rebuilds the corpus fixtures from the sources in this folder. Needs
# LibreOffice (soffice) and python3; tests/corpus/README.md records the
# LibreOffice version the committed files were made with.
# Usage: make-fixtures.sh [output folder, default: tests/corpus]
set -eu

SRC=$(cd "$(dirname "$0")" && pwd)
OUT=$(cd "${1:-$SRC/..}" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# The image the rich document and the slides embed, generated, not drawn.
python3 "$SRC/shapes_png.py" > "$TMP/shapes.png"
B64=$(base64 -w0 "$TMP/shapes.png")
for f in "$SRC"/*.fodt "$SRC"/*.fods "$SRC"/*.fodp; do
  sed "s|@SHAPES_PNG_BASE64@|$B64|" "$f" > "$TMP/$(basename "$f")"
done

# A throwaway profile, so no user setting changes the output.
convert() {
  soffice "-env:UserInstallation=file://$TMP/profile" --headless \
    --convert-to "$1" --outdir "$OUT" "$TMP/$2"
}
for doc in liberation-carlito justified amp-name metadata rich cyrillic-greek cjk arabic-rtl hebrew; do
  convert docx "$doc.fodt"
done
convert odt basic.fodt
# Calc and Impress name Arial and Times New Roman in the OOXML they export,
# whatever the source says, so the xlsx and pptx are system-fonts fixtures and
# the same sources also go out as ODF, which keeps the Liberation names.
convert xlsx sheet.fods
convert ods sheet.fods
convert ods basic.fods
convert pptx slides.fodp
convert odp slides.fodp
convert odp basic.fodp

python3 "$SRC/text_fixtures.py" "$OUT"
