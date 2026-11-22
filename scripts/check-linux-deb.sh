#!/usr/bin/env bash
# Fails when the packaged .deb does not have the layout the app expects.
set -euo pipefail

DEB="${1:?Usage: check-linux-deb.sh <deb>}"
APP_DIR="usr/lib/Euro-Office-Lite"

# One path per line, without the leading ./ that dpkg-deb --build writes.
deb_paths() {
    dpkg-deb --fsys-tarfile "$DEB" | tar -t | sed 's|^\./||'
}

# The permissions of one path, as tar lists them (-rwxr-xr-x).
deb_mode() {
    dpkg-deb --fsys-tarfile "$DEB" | tar -tv | awk -v path="$1" '
        { name = $6; sub(/^\.\//, "", name) }
        name == path { print $1 }'
}

fail() {
    echo "ERROR: $1"
    exit 1
}

# x2t reads DoctRenderer.config from the folder its own file is in, so the
# only x2t has to be the one in binaries/, next to that file.
check_single_x2t() {
    local found
    found=$(grep -E '(^|/)x2t(-[^/]*)?$' <<< "$PATHS" || true)
    if [ "$found" != "$APP_DIR/binaries/x2t" ]; then
        fail "expected x2t only at $APP_DIR/binaries/x2t, found: ${found:-none}"
    fi
    echo "x2t: $found"
}

# Every conversion starts this file.
check_x2t_is_executable() {
    local mode
    mode=$(deb_mode "$APP_DIR/binaries/x2t")
    case "$mode" in
        -rwx*) echo "x2t mode: $mode" ;;
        *) fail "$APP_DIR/binaries/x2t is not executable: ${mode:-not found}" ;;
    esac
}

PATHS=$(deb_paths)
check_single_x2t
check_x2t_is_executable
