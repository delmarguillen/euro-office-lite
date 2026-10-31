#!/bin/sh
# Rebuilds the corpus fixtures from the sources in this folder. Needs
# LibreOffice 7.3.7.2 (soffice) and python3: another LibreOffice writes other
# bytes than the committed files, so the script refuses to run with it.
# Usage: make-fixtures.sh [output folder, default: tests/corpus]
set -eu

LIBREOFFICE_VERSION=7.3.7.2

SRC=$(cd "$(dirname "$0")" && pwd)
OUT=$(cd "${1:-$SRC/..}" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# A throwaway profile, so no user setting changes the output.
soffice_here() {
  soffice "-env:UserInstallation=file://$TMP/profile" "$@"
}
convert() {
  soffice_here --headless --convert-to "$1" --outdir "$OUT" "$TMP/$2"
}

# It prints "LibreOffice 7.3.7.2 30(Build:2)": the second word is the version.
found=$(soffice_here --version | cut -d' ' -f2)
if [ "$found" != "$LIBREOFFICE_VERSION" ]; then
  echo "make-fixtures.sh: needs LibreOffice $LIBREOFFICE_VERSION, found '$found'" >&2
  exit 1
fi

# The image the rich document and the slides embed, generated, not drawn.
python3 "$SRC/shapes_png.py" > "$TMP/shapes.png"
B64=$(base64 -w0 "$TMP/shapes.png")
for f in "$SRC"/*.fodt "$SRC"/*.fods "$SRC"/*.fodp; do
  sed "s|@SHAPES_PNG_BASE64@|$B64|" "$f" > "$TMP/$(basename "$f")"
done

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
