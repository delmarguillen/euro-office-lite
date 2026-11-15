#!/bin/bash
# restore.sh DIR: puts back what backup.sh DIR saved (an entry that did not
# exist then is removed) and checks the result against its snapshot.

HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/backup.sh"

restore_entries() {
  local dir=$1 entry
  for entry in "${ENTRIES[@]}"; do
    rm -rf -- "${CONFIG_DIR:?}/$entry"
  done
  [ -s "$dir/entries.txt" ] || return 0
  tar -C "$CONFIG_DIR" -xpf "$dir/state.tar"
}

restore_main() {
  local dir=${1:-}
  [ -n "$dir" ] || die "usage: restore.sh DIR"
  [ -f "$dir/snapshot.txt" ] || die "no backup in $dir"
  [ -z "$(app_pids)" ] || die "the app is running: close it first"
  restore_entries "$dir" || die "tar failed"
  snapshot "$dir/entries.txt" "$dir/snapshot-after.txt"
  if ! cmp -s "$dir/snapshot.txt" "$dir/snapshot-after.txt"; then
    log "restored state differs from the backup:"
    diff "$dir/snapshot.txt" "$dir/snapshot-after.txt" >&2
    exit 1
  fi
  echo "restored $(wc -l < "$dir/snapshot.txt") paths from $dir, sha256 match"
}

restore_main "$@"
