#!/usr/bin/env bash
# Stages only the runtime-needed files from src/ into src-dist/.
# Run before "npx tauri build" so frontendDist points to a slim tree.
# macOS/Linux equivalent of prepare-dist.ps1.
set -euo pipefail

PROJECT_ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
SRC="$PROJECT_ROOT/src"
DIST="$PROJECT_ROOT/src-dist"
LOG_DIR="${TMPDIR:-/tmp}/euro-office-lite"
LOG_FILE="$LOG_DIR/prepare-dist.log"

mkdir -p "$LOG_DIR"

TOTAL_FILES=0
TOTAL_BYTES=0
SECONDS=0

log() {
    local line
    line="$(date '+%H:%M:%S') $1"
    echo "$line" | tee -a "$LOG_FILE"
}

: > "$LOG_FILE"
log "=== prepare-dist.sh ==="
log "Source:      $SRC"
log "Destination: $DIST"
log "System:      $(uname -ms)"

# Clean previous dist (symlink-safe)
if [ -L "$DIST" ]; then
    log "Removing previous src-dist/ (symlink)"
    rm "$DIST"
elif [ -d "$DIST" ]; then
    log "Removing previous src-dist/ (directory)"
    rm -rf "$DIST"
fi

copy_tree() {
    local src_path="$1"
    local dest_path="$2"
    local label="$3"
    shift 3
    local excludes=()
    if [ $# -gt 0 ]; then excludes=("$@"); fi

    if [ ! -d "$src_path" ]; then
        log "SKIP (not found): $label -> $src_path"
        return
    fi

    local rsync_args=(-a --quiet)
    rsync_args+=(--exclude '.git')
    if [ ${#excludes[@]} -gt 0 ]; then
        for excl in "${excludes[@]}"; do
            rsync_args+=(--exclude "$excl")
        done
    fi

    mkdir -p "$dest_path"
    rsync "${rsync_args[@]}" "$src_path/" "$dest_path/"

    local count size
    count=$(find "$dest_path" -type f | wc -l | tr -d ' ')
    size=$(du -sb "$dest_path" 2>/dev/null | cut -f1 || echo 0)
    TOTAL_FILES=$((TOTAL_FILES + count))
    TOTAL_BYTES=$((TOTAL_BYTES + size))

    local size_mb
    size_mb=$(echo "scale=1; $size / 1048576" | bc 2>/dev/null || echo "?")
    log "$(printf 'COPY: %-55s %6s files  %8s MB' "$label" "$count" "$size_mb")"
}

copy_single_file() {
    local src_file="$1"
    local dest_file="$2"
    local label="$3"

    if [ ! -f "$src_file" ]; then
        log "SKIP (not found): $label"
        return
    fi

    mkdir -p "$(dirname "$dest_file")"
    cp "$src_file" "$dest_file"

    local size
    size=$(stat -f%z "$dest_file" 2>/dev/null || stat -c%s "$dest_file" 2>/dev/null || echo 0)
    TOTAL_FILES=$((TOTAL_FILES + 1))
    TOTAL_BYTES=$((TOTAL_BYTES + size))

    local size_mb
    size_mb=$(echo "scale=1; $size / 1048576" | bc 2>/dev/null || echo "?")
    log "$(printf 'COPY: %-55s %6s files  %8s MB' "$label" "1" "$size_mb")"
}

# --- Root files ---
copy_single_file "$SRC/index.html" "$DIST/index.html" "index.html"
copy_single_file "$SRC/bridge.js"  "$DIST/bridge.js"  "bridge.js"

# --- Fonts ---
copy_tree "$SRC/fonts" "$DIST/fonts" "src/fonts"

# --- web-apps editors (main/ without help/) ---
for ed in documenteditor spreadsheeteditor presentationeditor; do
    copy_tree "$SRC/web-apps/apps/$ed/main" "$DIST/web-apps/apps/$ed/main" "web-apps/apps/$ed/main" "help"
done

# --- web-apps shared ---
copy_tree "$SRC/web-apps/apps/api"    "$DIST/web-apps/apps/api"    "web-apps/apps/api"
copy_tree "$SRC/web-apps/apps/common" "$DIST/web-apps/apps/common" "web-apps/apps/common"

# --- web-apps vendor (only needed libraries) ---
for lib in backbone underscore xregexp jquery jquery.browser \
           requirejs requirejs-text es6-promise fetch \
           perfect-scrollbar svg-injector socketio less; do
    copy_tree "$SRC/web-apps/vendor/$lib" "$DIST/web-apps/vendor/$lib" "web-apps/vendor/$lib"
done

for lib in ace framework7-react monaco; do
    log "$(printf 'EXCL: %-55s (not needed at runtime)' "web-apps/vendor/$lib")"
done

# --- sdkjs modules ---
for mod in word cell slide common; do
    copy_tree "$SRC/sdkjs/$mod" "$DIST/sdkjs/$mod" "sdkjs/$mod"
done

# --- sdkjs/pdf (referenced by word editor's scripts.js) ---
copy_tree "$SRC/sdkjs/pdf" "$DIST/sdkjs/pdf" "sdkjs/pdf" "build" "test" ".git"

# --- sdkjs/vendor ---
copy_tree "$SRC/sdkjs/vendor" "$DIST/sdkjs/vendor" "sdkjs/vendor"

# --- sdkjs/develop (scripts.js per editor) ---
for mod in word cell slide; do
    copy_tree "$SRC/sdkjs/develop/sdkjs/$mod" "$DIST/sdkjs/develop/sdkjs/$mod" "sdkjs/develop/sdkjs/$mod"
done

for d in tests build visio .docker .github; do
    log "$(printf 'EXCL: %-55s (not needed at runtime)' "sdkjs/$d")"
done

for d in pdfeditor visioeditor build .docker .github test; do
    log "$(printf 'EXCL: %-55s (not needed at runtime)' "web-apps/$d")"
done

# --- Summary ---
TOTAL_MB=$(echo "scale=1; $TOTAL_BYTES / 1048576" | bc 2>/dev/null || echo "?")
log ""
log "========================================"
log "Total files:  $TOTAL_FILES"
log "Total size:   $TOTAL_MB MB"
log "Elapsed:      ${SECONDS}s"
log "========================================"
log "Done. frontendDist should point to ../src-dist"

exit 0
