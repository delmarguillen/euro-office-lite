#!/usr/bin/env bash
# Downloads pre-compiled sdkjs bundles from the 'dependencies' release for CI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET_DIR="$SCRIPT_DIR/../src-tauri/binaries"
REPO="${GITHUB_REPOSITORY:-delmarguillen/euro-office-lite}"
ZIP_NAME="sdkjs-compiled.zip"
TEMP_ZIP="${TMPDIR:-/tmp}/$ZIP_NAME"

log() {
    echo "[$(date '+%H:%M:%S')] $1"
}

log "=== get-sdkjs-ci.sh ==="

# Check if already present
if [ -f "$TARGET_DIR/sdk-all-min.js" ]; then
    SIZE=$(wc -c < "$TARGET_DIR/sdk-all-min.js")
    log "sdk-all-min.js already present ($SIZE bytes), skipping download"
    exit 0
fi

log "Downloading $ZIP_NAME from 'dependencies' release..."
gh release download dependencies --repo "$REPO" --pattern "$ZIP_NAME" --output "$TEMP_ZIP" --clobber

STAGING="${TMPDIR:-/tmp}/sdkjs-staging"
rm -rf "$STAGING"
mkdir -p "$STAGING"

log "Extracting..."
unzip -o "$TEMP_ZIP" -d "$STAGING"
rm -f "$TEMP_ZIP"

# Copy word bundle to binaries (used by DoctRenderer)
if [ -f "$STAGING/word/sdk-all-min.js" ]; then
    cp "$STAGING/word/sdk-all-min.js" "$TARGET_DIR/sdk-all-min.js"
    log "sdk-all-min.js: $(wc -c < "$TARGET_DIR/sdk-all-min.js") bytes"
fi
if [ -f "$STAGING/word/sdk-all.js" ]; then
    cp "$STAGING/word/sdk-all.js" "$TARGET_DIR/sdk-all.js"
    log "sdk-all.js: $(wc -c < "$TARGET_DIR/sdk-all.js") bytes"
fi

rm -rf "$STAGING"
log "sdkjs bundles ready in $TARGET_DIR"
