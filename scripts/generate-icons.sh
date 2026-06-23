#!/usr/bin/env bash
# Generates macOS .icns and PNG icons from the existing icon.ico.
# Requires: Python 3 with Pillow (already available in CI for prepare-fonts).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ICONS_DIR="$SCRIPT_DIR/../src-tauri/icons"
ICO_FILE="$ICONS_DIR/icon.ico"
ICONSET_DIR="${TMPDIR:-/tmp}/euro-office-lite.iconset"

echo "[icons] Generating macOS icons from $ICO_FILE"

if [ ! -f "$ICO_FILE" ]; then
    echo "[icons] ERROR: $ICO_FILE not found"
    exit 1
fi

# Create iconset directory before Python writes to it
mkdir -p "$ICONSET_DIR"

# Extract largest PNG from .ico using Python/Pillow
python3 -c "
from PIL import Image
import sys

ico = Image.open('$ICO_FILE')
# Get the largest frame
sizes = []
for i in range(getattr(ico, 'n_frames', 1)):
    ico.seek(i)
    sizes.append((ico.size[0] * ico.size[1], i, ico.size))

sizes.sort(reverse=True)
ico.seek(sizes[0][1])
img = ico.copy().convert('RGBA')

# Save as 1024x1024 PNG (source for iconutil)
img = img.resize((1024, 1024), Image.LANCZOS)
img.save('$ICONS_DIR/icon.png')
print(f'[icons] Generated icon.png (1024x1024)')

# Generate sizes needed for .iconset
for size in [16, 32, 64, 128, 256, 512, 1024]:
    resized = img.resize((size, size), Image.LANCZOS)
    resized.save(f'$ICONSET_DIR/icon_{size}x{size}.png')
    if size <= 512:
        double = img.resize((size * 2, size * 2), Image.LANCZOS)
        double.save(f'$ICONSET_DIR/icon_{size}x{size}@2x.png')

# Also save standard Tauri icon sizes
for size in [32, 128, 256]:
    resized = img.resize((size, size), Image.LANCZOS)
    resized.save(f'$ICONS_DIR/{size}x{size}.png')
    if size == 128:
        double = img.resize((256, 256), Image.LANCZOS)
        double.save(f'$ICONS_DIR/128x128@2x.png')

print('[icons] Generated all PNG sizes')
"

# Generate .icns using macOS iconutil
if command -v iconutil &>/dev/null; then
    iconutil -c icns "$ICONSET_DIR" -o "$ICONS_DIR/icon.icns"
    echo "[icons] Generated icon.icns via iconutil"
else
    echo "[icons] WARNING: iconutil not available, skipping .icns generation"
fi

# Cleanup
rm -rf "$ICONSET_DIR"

echo "[icons] Done. Icons in $ICONS_DIR:"
ls -la "$ICONS_DIR/"
