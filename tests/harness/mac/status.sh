#!/bin/bash
# mac/status.sh [DMG...]: the installed app, read from its Info.plist and its
# binary. The binary carries no commit id: each DMG given is matched by the
# sha256 of the binary it holds.

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
. "$HERE/lib.sh"

show_app() {
  local version
  version=$(installed_version)
  echo "app: $APP_DIR version ${version:-not installed}"
}

show_binary() {
  local sum
  sum=$(installed_sha256)
  echo "binary: $APP_BIN sha256 ${sum:-missing}"
}

show_signature() {
  [ -d "$APP_DIR" ] || return 0
  if codesign --verify --deep --strict "$APP_DIR" 2>/dev/null; then
    echo "codesign --verify: valid"
    return
  fi
  echo "codesign --verify: INVALID"
}

show_dmg_match() {
  local dmg=$1 sum verdict="differs from"
  sum=$(dmg_binary_sha256 "$dmg")
  [ -n "$sum" ] && [ "$sum" = "$(installed_sha256)" ] && verdict="IS"
  echo "dmg: $(basename "$dmg") binary ${sum:-unreadable} $verdict the installed one"
}

show_processes() {
  local running
  running=$(app_pids | tr '\n' ' ')
  echo "running: ${running:-none}"
}

show_user_state() {
  echo "recover entries: $(recover_entries | wc -l | tr -d ' ')"
  echo "recent-files.json: $(sha256_of "$CONFIG_DIR/recent-files.json")"
}

show_app
show_binary
show_signature
for dmg in "$@"; do
  show_dmg_match "$dmg"
done
show_processes
show_user_state
