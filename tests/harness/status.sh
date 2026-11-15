#!/bin/bash
# status.sh [DEB...]: what is installed, read from dpkg and from the binary.
# The binary carries no commit id: each DEB given is matched by the sha256 of
# the binary it holds.

HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/lib.sh"

show_package() {
  local status
  status=$(dpkg-query -W -f='${Version} (${Status})' "$APP_PACKAGE" 2>/dev/null)
  echo "package: $APP_PACKAGE ${status:-not installed}"
}

show_binary() {
  local sum
  sum=$(installed_sha256)
  echo "binary: $APP_BIN sha256 ${sum:-missing}"
}

show_deb_match() {
  local deb=$1 sum verdict="differs from"
  sum=$(deb_binary_sha256 "$deb")
  [ "$sum" = "$(installed_sha256)" ] && verdict="IS"
  echo "deb: $(basename "$deb") binary $sum $verdict the installed one"
}

show_processes() {
  local running
  running=$(app_pids | tr '\n' ' ')
  echo "running: ${running:-none}"
}

show_user_state() {
  local recent=$CONFIG_DIR/recent-files.json
  echo "recover entries: $(recover_entries | wc -l)"
  echo "recent-files.json: $(sha256_of "$recent")"
}

show_package
show_binary
for deb in "$@"; do
  show_deb_match "$deb"
done
show_processes
show_user_state
