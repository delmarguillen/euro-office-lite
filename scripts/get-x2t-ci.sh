#!/usr/bin/env bash
# Downloads x2t binaries from the 'dependencies' release for CI.
# Supports macOS and Linux.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET_DIR="$SCRIPT_DIR/../src-tauri/binaries"
REPO="${GITHUB_REPOSITORY:-delmarguillen/euro-office-lite}"
OS="$(uname -s)"
ARCH="$(uname -m)"
TARGET_TRIPLE="${1:-}"

log() {
    echo "[$(date '+%H:%M:%S')] $1"
}

log "=== get-x2t-ci.sh ==="
log "System: $(uname -ms)"
log "Repo: $REPO"
log "Target dir: $TARGET_DIR"
[ -n "$TARGET_TRIPLE" ] && log "Target triple (override): $TARGET_TRIPLE"

if [ "$OS" = "Darwin" ]; then
    case "$TARGET_TRIPLE" in
        x86_64-apple-darwin)
            ZIP_NAME="x2t-binaries-macos-x64.zip"
            TRIPLE="x86_64-apple-darwin"
            ;;
        aarch64-apple-darwin|"")
            ZIP_NAME="x2t-binaries-macos-arm64.zip"
            TRIPLE="aarch64-apple-darwin"
            ;;
        *)
            log "ERROR: Unsupported macOS target: $TARGET_TRIPLE"
            exit 1
            ;;
    esac
    CHECK_PATTERN="x2t-*-apple-darwin"
    VERIFY_CMD="otool -L"
elif [ "$OS" = "Linux" ]; then
    ZIP_NAME="x2t-binaries-linux-x64.zip"
    if [ "$ARCH" = "aarch64" ]; then
        TRIPLE="aarch64-unknown-linux-gnu"
    else
        TRIPLE="x86_64-unknown-linux-gnu"
    fi
    CHECK_PATTERN="x2t-*-linux-gnu"
    VERIFY_CMD="ldd"
else
    log "ERROR: Unsupported OS: $OS"
    exit 1
fi

TEMP_ZIP="${TMPDIR:-/tmp}/$ZIP_NAME"

# Check if already present
if ls "$TARGET_DIR"/$CHECK_PATTERN 1>/dev/null 2>&1; then
    log "x2t binary already present, skipping download"
    ls -la "$TARGET_DIR"/x2t-*
    exit 0
fi

log "Downloading $ZIP_NAME from 'dependencies' release..."
gh release download dependencies --repo "$REPO" --pattern "$ZIP_NAME" --output "$TEMP_ZIP" --clobber

mkdir -p "$TARGET_DIR"

log "Extracting to $TARGET_DIR..."
unzip -o "$TEMP_ZIP" -d "$TARGET_DIR"
rm -f "$TEMP_ZIP"

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
$VERIFY_CMD "$SIDECAR" 2>/dev/null || log "($VERIFY_CMD not available or failed)"

# On Linux, verify with LD_LIBRARY_PATH pointing to extracted libs
if [ "$OS" = "Linux" ]; then
    log "Library resolution with LD_LIBRARY_PATH:"
    LD_LIBRARY_PATH="$TARGET_DIR" ldd "$SIDECAR" 2>/dev/null || log "(ldd with LD_LIBRARY_PATH failed)"
fi

COUNT=$(find "$TARGET_DIR" -type f | wc -l | tr -d ' ')
SIZE=$(du -sh "$TARGET_DIR" | cut -f1)
log "x2t ready: $COUNT files, $SIZE total in $TARGET_DIR (target: $TRIPLE)"
