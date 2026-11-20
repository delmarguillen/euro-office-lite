#!/bin/bash
# mac/restore.sh DIR: puts back what mac/backup.sh DIR saved (a folder that
# did not exist then is removed) and checks the result against its snapshot.

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
. "$HERE/backup.sh"

restore_roots() {
  local dir=$1 root
  for root in "${ROOTS[@]}"; do
    rm -rf -- "${HOME:?}/$root"
  done
  [ -s "$dir/roots.txt" ] || return 0
  tar -C "$HOME" -xpf "$dir/state.tar"
}

restore_main() {
  local dir=${1:-}
  [ -n "$dir" ] || die "usage: mac/restore.sh DIR"
  [ -f "$dir/snapshot.txt" ] || die "no backup in $dir"
  [ -z "$(app_pids)" ] || die "the app is running: close it first"
  restore_roots "$dir" || die "tar failed"
  snapshot "$dir/roots.txt" "$dir/snapshot-after.txt"
  if ! cmp -s "$dir/snapshot.txt" "$dir/snapshot-after.txt"; then
    log "restored state differs from the backup:"
    diff "$dir/snapshot.txt" "$dir/snapshot-after.txt" >&2
    exit 1
  fi
  echo "restored $(wc -l < "$dir/snapshot.txt" | tr -d ' ') paths from $dir, sha256 match"
}

restore_main "$@"
