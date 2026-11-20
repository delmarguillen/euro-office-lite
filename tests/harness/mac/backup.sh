#!/bin/bash
# mac/backup.sh DIR: saves the app's data folder (recovery, recent files,
# window state) and its cache folder (fonts) into DIR, with a listing and the
# sha256 of every file for restore.sh.

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
. "$HERE/lib.sh"

# Relative to $HOME, so tar keeps the paths short.
ROOTS=("Library/Application Support/org.euro-office.lite" "Library/Caches/org.euro-office.lite")

existing_roots() {
  local root
  for root in "${ROOTS[@]}"; do
    [ -e "$HOME/$root" ] && echo "$root"
  done
}

# snapshot ROOTS_FILE OUT: every path under the roots, with a sha256 per file.
snapshot() {
  local roots=$1 out=$2 root path
  (
    cd "$HOME" || exit 1
    while IFS= read -r root; do
      find "$root"
    done < "$roots" | sort | while IFS= read -r path; do
      [ -d "$path" ] && echo "dir $path"
      [ -f "$path" ] && echo "file $(sha256_of "$path") $path"
    done
  ) > "$out"
}

backup_main() {
  local dir=${1:-}
  [ -n "$dir" ] || die "usage: mac/backup.sh DIR"
  [ -z "$(app_pids)" ] || die "the app is running: close it first"
  mkdir -p "$dir" || die "cannot create $dir"
  existing_roots > "$dir/roots.txt"
  tar -C "$HOME" -cf "$dir/state.tar" -T "$dir/roots.txt" || die "tar failed"
  snapshot "$dir/roots.txt" "$dir/snapshot.txt"
  echo "saved $(wc -l < "$dir/snapshot.txt" | tr -d ' ') paths to $dir"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  backup_main "$@"
fi
