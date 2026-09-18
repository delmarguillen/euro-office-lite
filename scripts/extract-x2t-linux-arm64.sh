#!/usr/bin/env bash
# Extracts the x2t sidecar and its runtime libraries for aarch64 Linux.
#
# The 'dependencies' release only carries x2t-binaries-linux-x64.zip, so for
# arm64 we take the same route the one-shot extract-x2t-linux.yml workflow
# takes for x64, but from the ONLYOFFICE aarch64 package (ONLYOFFICE ships
# Linux arm64 builds; Euro-Office's CI does not).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET_DIR="${X2T_TARGET_DIR:-$SCRIPT_DIR/../src-tauri/binaries}"
ONLYOFFICE_VERSION="${ONLYOFFICE_VERSION:-v9.4.0}"
TRIPLE="aarch64-unknown-linux-gnu"
WORK="${TMPDIR:-/tmp}/x2t-arm64-$$"

log() { echo "[$(date '+%H:%M:%S')] $1"; }

if [ "$(uname -m)" != "aarch64" ]; then
    log "WARNING: host is $(uname -m); extracting arm64 binaries anyway"
fi

if [ -f "$TARGET_DIR/x2t-$TRIPLE" ]; then
    log "x2t-$TRIPLE already present, skipping"
    exit 0
fi

mkdir -p "$WORK" "$TARGET_DIR"
trap 'rm -rf "$WORK"' EXIT

DEB="$WORK/onlyoffice-desktopeditors_arm64.deb"
URL="https://github.com/ONLYOFFICE/DesktopEditors/releases/download/${ONLYOFFICE_VERSION}/onlyoffice-desktopeditors_arm64.deb"
log "Downloading $URL"
curl -fL -o "$DEB" "$URL"

log "Unpacking package"
cd "$WORK"
if command -v bsdtar >/dev/null 2>&1; then
    bsdtar -xf "$DEB" data.tar.xz
else
    ar x "$DEB" data.tar.xz
fi
bsdtar -xf data.tar.xz 2>/dev/null || tar xf data.tar.xz

CONVERTER="$(find "$WORK" -type d -name converter | head -1)"
[ -n "$CONVERTER" ] || { log "ERROR: converter directory not found"; exit 1; }
[ -f "$CONVERTER/x2t" ] || { log "ERROR: x2t not found in $CONVERTER"; exit 1; }

log "Staging into $TARGET_DIR"
cp "$CONVERTER/x2t" "$TARGET_DIR/x2t-$TRIPLE"
chmod +x "$TARGET_DIR/x2t-$TRIPLE"
find "$CONVERTER" -maxdepth 1 -name "*.so" -exec cp -P {} "$TARGET_DIR/" \;
find "$CONVERTER" -maxdepth 1 -name "*.so.*" -exec cp -P {} "$TARGET_DIR/" \;
find "$CONVERTER" -maxdepth 1 \( -name "*.config" -o -name "*.xml" -o -name "*.dat" \) -exec cp {} "$TARGET_DIR/" \;
[ -d "$(dirname "$CONVERTER")/core-fonts" ] && cp -r "$(dirname "$CONVERTER")/core-fonts" "$TARGET_DIR/core-fonts" || true

log "Verification:"
file "$TARGET_DIR/x2t-$TRIPLE"
if LD_LIBRARY_PATH="$TARGET_DIR" ldd "$TARGET_DIR/x2t-$TRIPLE" | grep -q "not found"; then
    log "ERROR: unresolved shared libraries:"
    LD_LIBRARY_PATH="$TARGET_DIR" ldd "$TARGET_DIR/x2t-$TRIPLE" | grep "not found"
    exit 1
fi
log "x2t ready: $(find "$TARGET_DIR" -type f | wc -l | tr -d ' ') files, $(du -sh "$TARGET_DIR" | cut -f1) in $TARGET_DIR"
