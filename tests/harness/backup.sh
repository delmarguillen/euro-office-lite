#!/bin/bash
# backup.sh DIR: saves the app's recovery folder, recent files and window
# state into DIR, with a listing and the sha256 of every file for restore.sh.

HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/lib.sh"

ENTRIES=(recover recent-files.json .window-state.json)

existing_entries() {
  local entry
  for entry in "${ENTRIES[@]}"; do
    [ -e "$CONFIG_DIR/$entry" ] && echo "$entry"
  done
}

# snapshot ENTRIES_FILE OUT: every path under the entries, with a sha256 per file.
snapshot() {
  local entries=$1 out=$2 path
  (
    cd "$CONFIG_DIR" || exit 1
    while IFS= read -r path; do
      [ -d "$path" ] && echo "dir $path"
      [ -f "$path" ] && echo "file $(sha256_of "$path") $path"
    done < <(xargs -r -a "$entries" -d '\n' find | sort)
  ) > "$out"
}

main() {
  local dir=${1:-}
  [ -n "$dir" ] || die "usage: backup.sh DIR"
  [ -d "$CONFIG_DIR" ] || die "no $CONFIG_DIR"
  [ -z "$(app_pids)" ] || die "the app is running: close it first"
  mkdir -p "$dir" || die "cannot create $dir"
  existing_entries > "$dir/entries.txt"
  tar -C "$CONFIG_DIR" -cf "$dir/state.tar" -T "$dir/entries.txt" || die "tar failed"
  snapshot "$dir/entries.txt" "$dir/snapshot.txt"
  echo "saved $(wc -l < "$dir/snapshot.txt") paths to $dir"
}

[ "${BASH_SOURCE[0]}" = "$0" ] && main "$@"
