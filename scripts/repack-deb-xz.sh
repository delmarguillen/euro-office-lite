#!/usr/bin/env bash
# Rewrites a .deb in place with xz members. Tauri writes control.tar.gz and
# data.tar.gz and its deb settings have no compression option.
set -euo pipefail

DEB="${1:?Usage: repack-deb-xz.sh <deb>}"

# Mode, owner, size and path of every packaged entry, sorted by path. Without
# the ./ entries dpkg-deb --build adds.
packaged_entries() {
    dpkg-deb --fsys-tarfile "$1" | tar -tv --numeric-owner | awk '
        { path = $6; sub(/^\.\//, "", path); sub(/\/$/, "", path) }
        path != "" && path != "." { print $1, $2, $3, path }' | sort -k4
}

# --root-owner-group: the tree is extracted as the current user, and the
# package must keep its entries owned by root.
repack_with_xz() {
    local tree="$1" output="$2"
    dpkg-deb --raw-extract "$DEB" "$tree"
    dpkg-deb --root-owner-group -Zxz --build "$tree" "$output"
}

check_same_package() {
    local output="$1"
    if ! diff <(packaged_entries "$DEB") <(packaged_entries "$output"); then
        echo "ERROR: the repacked .deb does not hold the same entries"
        exit 1
    fi
    if ! diff <(dpkg-deb --field "$DEB") <(dpkg-deb --field "$output"); then
        echo "ERROR: the repacked .deb does not hold the same control fields"
        exit 1
    fi
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

echo "Before: $(wc -c < "$DEB") bytes, members: $(ar t "$DEB" | tr '\n' ' ')"
repack_with_xz "$WORK/tree" "$WORK/repacked.deb"
check_same_package "$WORK/repacked.deb"
mv "$WORK/repacked.deb" "$DEB"
echo "After: $(wc -c < "$DEB") bytes, members: $(ar t "$DEB" | tr '\n' ' ')"
