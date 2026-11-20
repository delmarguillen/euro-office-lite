#!/bin/bash
# mac/run.sh CASE|all [--dmg FILE] [--control] [--screenshots] [--out DIR]
#
# Runs harness cases against the app in /Applications on macOS and writes
# OUT/result.json with the schema of ../run.sh. With --dmg, installs the app
# of that disk image first unless its binary is already the installed one,
# and checks it with codesign --verify. A case whose keys have no macOS
# equivalent is skipped: "skipped": true and the reason in its detail.
# Exit status: 0 every case ok, 1 some case not ok, 2 the harness could not run.

# ../run.sh keeps a case in an associative array (bash 4); macOS has bash 3.2.
if [ "${BASH_VERSINFO[0]}" -lt 4 ]; then
  for candidate in /opt/homebrew/bin/bash /usr/local/bin/bash; do
    [ -x "$candidate" ] && exec "$candidate" "$0" "$@"
  done
  echo "[harness] mac/run.sh needs bash 4 or later (brew install bash)" >&2
  exit 2
fi

MAC_RUN_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
. "$MAC_RUN_DIR/../run.sh"
. "$MAC_RUN_DIR/lib.sh"

DMG=""

parse_args() {
  while [ $# -gt 0 ]; do
    case $1 in
      --dmg) DMG=$2; shift ;;
      --control) CONTROL=true ;;
      --screenshots) ALWAYS_SHOOT=true ;;
      --out) OUT=$2; shift ;;
      -*) die "unknown option $1" ;;
      *) SELECTION=$1 ;;
    esac
    shift
  done
  [ -n "$SELECTION" ] || die "usage: mac/run.sh CASE|all [--dmg FILE] [--control] [--screenshots] [--out DIR]"
  [ -n "$OUT" ] || OUT=${HARNESS_WORK:-$HOME/eol-harness-work}/run-$(date +%Y%m%d-%H%M%S)
}

check_app_installed() {
  [ -x "$APP_BIN" ] || die "$APP_BIN is not installed: give a disk image with --dmg"
}

install_from_dmg() {
  local dmg=$1 mnt app want
  [ -f "$dmg" ] || die "no disk image at $dmg"
  mnt=$(dmg_attach "$dmg") || die "hdiutil could not mount $dmg"
  app=$(dmg_app "$mnt")
  [ -n "$app" ] && want=$(sha256_of "$(bundle_binary "$app")")
  if [ -z "$want" ]; then
    dmg_detach "$mnt"
    die "no app binary in $dmg"
  fi
  if [ "$want" = "$(installed_sha256)" ]; then
    log "the binary of $(basename "$dmg") is already installed"
  else
    log "installing $(basename "$app") from $(basename "$dmg")"
    rm -rf "${APP_DIR:?}"
    ditto "$app" "$APP_DIR" || { dmg_detach "$mnt"; die "ditto failed"; }
  fi
  dmg_detach "$mnt"
  APP_BIN=$(bundle_binary "$APP_DIR") || die "$APP_DIR has no Info.plist"
  [ "$want" = "$(installed_sha256)" ] || die "the installed binary is not the one in $dmg"
  codesign --verify --deep --strict "$APP_DIR" >"$OUT/codesign.log" 2>&1 ||
    die "codesign --verify failed, see $OUT/codesign.log"
  log "codesign --verify: $APP_DIR is valid"
}

# skip_reason: why the case read last cannot run on macOS, or nothing.
skip_reason() {
  local key chord
  for key in keys_before keys_after; do
    for chord in $(case_value "$key"); do
      chord_statements "$chord" >/dev/null && continue
      echo "key $chord of $key has no macOS equivalent in mac/lib.sh"
      return
    done
  done
}

record_skipped() {
  local name=$1 reason=$2
  RESULTS+=("{\"case\": $(json_str "$name"), \"ok\": true, \"expected\": $(json_str "$(expected_outcome)"), \"observed\": \"skipped\", \"detail\": $(json_str "skipped on macOS: $reason"), \"screenshot\": null, \"seconds\": 0, \"skipped\": true}")
  log "$name: skipped: $reason"
}

run_case_on_mac() {
  local name=$1 reason
  if read_case "$HERE/cases/$name.toml"; then
    reason=$(skip_reason)
    if [ -n "$reason" ]; then
      record_skipped "$name" "$reason"
      return
    fi
  fi
  run_case "$name"
}

mac_main() {
  parse_args "$@"
  mkdir -p "$OUT" || die "cannot create $OUT"
  # open --stdout and the save panel need absolute paths.
  OUT=$(cd "$OUT" && pwd)
  FOLLOWERS_FILE=$OUT/.log-followers
  trap stop_log_followers EXIT
  check_machine_free
  [ -n "$DMG" ] && install_from_dmg "$DMG"
  check_app_installed
  log "app: $APP_DIR $(installed_version), binary sha256 $(installed_sha256)"
  log_pdftotext
  local name
  for name in $(selected_cases); do
    run_case_on_mac "$name"
  done
  write_result
  all_ok || exit 1
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  mac_main "$@"
fi
