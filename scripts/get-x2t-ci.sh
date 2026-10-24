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

# sha256 of each zip in the 'dependencies' release. A replaced asset must
# fail the build until its new digest is pinned here. A case statement keeps
# this working on the bash 3.2 that macOS ships.
expected_sha256() {
    case "$1" in
        x2t-binaries-linux-x64-9.4.0.129.zip) echo "6b7de53b626b61f5af5137fb9fa6d7b6c314075c7d4530fd8367f3db336a79d0" ;;
        x2t-binaries-linux-arm64.zip) echo "4abf69447ba6be774881aaf85e04205b1e99801cef231b3c14aea0c3f24efef7" ;;
        x2t-binaries-macos-arm64-9.4.0.129.zip) echo "ed9a6e1ee9f5155bfde2238f93eeba12d477005c7f7e61f9a858f667a11f3db5" ;;
        x2t-binaries-macos-x64-9.4.0.129.zip) echo "911bc13abec66cf9aec7e996fec14d289b53009ecaa8261020f5f50450001c6f" ;;
        *) return 1 ;;
    esac
}

sha256_of() {
    if [ "$OS" = "Darwin" ]; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        sha256sum "$1" | cut -d' ' -f1
    fi
}

log "=== get-x2t-ci.sh ==="
log "System: $(uname -ms)"
log "Repo: $REPO"
log "Target dir: $TARGET_DIR"
[ -n "$TARGET_TRIPLE" ] && log "Target triple (override): $TARGET_TRIPLE"

if [ "$OS" = "Darwin" ]; then
    case "$TARGET_TRIPLE" in
        x86_64-apple-darwin)
            ZIP_NAME="x2t-binaries-macos-x64-9.4.0.129.zip"
            TRIPLE="x86_64-apple-darwin"
            ;;
        aarch64-apple-darwin|"")
            ZIP_NAME="x2t-binaries-macos-arm64-9.4.0.129.zip"
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
    if [ "$ARCH" = "aarch64" ]; then
        ZIP_NAME="x2t-binaries-linux-arm64.zip"
        TRIPLE="aarch64-unknown-linux-gnu"
    else
        ZIP_NAME="x2t-binaries-linux-x64-9.4.0.129.zip"
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

if ! EXPECTED_SHA256="$(expected_sha256 "$ZIP_NAME")"; then
    log "ERROR: no pinned sha256 for $ZIP_NAME; add it to expected_sha256()"
    exit 1
fi

# Retry only the download (transient API/network errors); a checksum
# mismatch below is never retried.
log "Downloading $ZIP_NAME from 'dependencies' release..."
attempt=1
delay=10
until gh release download dependencies --repo "$REPO" --pattern "$ZIP_NAME" --output "$TEMP_ZIP" --clobber; do
    if [ "$attempt" -ge 3 ]; then
        log "ERROR: download of $ZIP_NAME failed after $attempt attempts"
        exit 1
    fi
    log "Download attempt $attempt failed; retrying in ${delay}s"
    sleep "$delay"
    attempt=$((attempt + 1))
    delay=$((delay * 2))
done

ACTUAL_SHA256="$(sha256_of "$TEMP_ZIP")"
if [ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]; then
    log "ERROR: sha256 mismatch for $ZIP_NAME"
    log "  expected: $EXPECTED_SHA256"
    log "  actual:   $ACTUAL_SHA256"
    rm -f "$TEMP_ZIP"
    exit 1
fi
log "sha256 verified for $ZIP_NAME: $ACTUAL_SHA256"

mkdir -p "$TARGET_DIR"

log "Extracting to $TARGET_DIR..."
unzip -o "$TEMP_ZIP" -d "$TARGET_DIR"
rm -f "$TEMP_ZIP"

# x2t 9.x on macOS loads versioned frameworks; codesign rejects a framework
# whose Versions/Current came out of the zip as a copy instead of a link.
if [ "$OS" = "Darwin" ]; then
    FW_COUNT=0
    for fw in "$TARGET_DIR"/*.framework; do
        [ -d "$fw" ] || continue
        if [ ! -L "$fw/Versions/Current" ]; then
            log "ERROR: $(basename "$fw")/Versions/Current is not a symbolic link"
            exit 1
        fi
        FW_COUNT=$((FW_COUNT + 1))
    done
    if [ "$FW_COUNT" -eq 0 ]; then
        log "ERROR: no *.framework in $ZIP_NAME"
        exit 1
    fi
    log "Frameworks with a linked Versions/Current: $FW_COUNT"
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
$VERIFY_CMD "$SIDECAR" 2>/dev/null || log "($VERIFY_CMD not available or failed)"

# On Linux, every library must resolve from the extracted ones: a missing
# one would otherwise only surface later, when x2t first runs.
if [ "$OS" = "Linux" ]; then
    log "Library resolution with LD_LIBRARY_PATH:"
    LDD_OUT="$(LD_LIBRARY_PATH="$TARGET_DIR" ldd "$SIDECAR" 2>/dev/null)" || log "(ldd with LD_LIBRARY_PATH failed)"
    echo "$LDD_OUT"
    if echo "$LDD_OUT" | grep -q "not found"; then
        log "ERROR: unresolved shared libraries for x2t-$TRIPLE:"
        echo "$LDD_OUT" | grep "not found"
        exit 1
    fi
fi

COUNT=$(find "$TARGET_DIR" -type f | wc -l | tr -d ' ')
SIZE=$(du -sh "$TARGET_DIR" | cut -f1)
log "x2t ready: $COUNT files, $SIZE total in $TARGET_DIR (target: $TRIPLE)"
