#!/usr/bin/env bash
# Downloads macOS x2t binaries from the 'dependencies' release for CI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET_DIR="$SCRIPT_DIR/../src-tauri/binaries"
REPO="${GITHUB_REPOSITORY:-delmarguillen/euro-office-lite}"
ZIP_NAME="x2t-binaries-macos-arm64.zip"
TEMP_ZIP="${TMPDIR:-/tmp}/$ZIP_NAME"

log() {
    echo "[$(date '+%H:%M:%S')] $1"
}

log "=== get-x2t-ci.sh ==="
log "System: $(uname -ms)"
log "Repo: $REPO"
log "Target dir: $TARGET_DIR"

# Check if already present
if ls "$TARGET_DIR"/x2t-*-apple-darwin 1>/dev/null 2>&1; then
    log "x2t macOS binary already present, skipping download"
    ls -la "$TARGET_DIR"/x2t-*
    exit 0
fi

log "Downloading $ZIP_NAME from 'dependencies' release..."
gh release download dependencies --repo "$REPO" --pattern "$ZIP_NAME" --output "$TEMP_ZIP" --clobber

mkdir -p "$TARGET_DIR"

log "Extracting to $TARGET_DIR..."
unzip -o "$TEMP_ZIP" -d "$TARGET_DIR"
rm -f "$TEMP_ZIP"

# Determine target triple
ARCH=$(uname -m)
if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
    TRIPLE="aarch64-apple-darwin"
else
    TRIPLE="x86_64-apple-darwin"
fi

SIDECAR="$TARGET_DIR/x2t-$TRIPLE"

# Rename if needed
if [ ! -f "$SIDECAR" ]; then
    if [ -f "$TARGET_DIR/x2t" ]; then
        mv "$TARGET_DIR/x2t" "$SIDECAR"
        log "Renamed x2t -> x2t-$TRIPLE"
    else
        log "ERROR: x2t binary not found after extraction"
        log "Contents of $TARGET_DIR:"
        ls -la "$TARGET_DIR/"
        exit 1
    fi
fi

# Make executable
chmod +x "$SIDECAR"

# Verify binary
log "Binary verification:"
file "$SIDECAR"
log "Dependencies:"
otool -L "$SIDECAR" 2>/dev/null || log "(otool not available)"

COUNT=$(find "$TARGET_DIR" -type f | wc -l | tr -d ' ')
SIZE=$(du -sh "$TARGET_DIR" | cut -f1)
log "x2t ready: $COUNT files, $SIZE total in $TARGET_DIR (target: $TRIPLE)"
