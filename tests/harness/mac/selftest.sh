#!/bin/bash
# mac/selftest.sh: checks mac/run.sh and its helpers on any system with
# bash 4, python3 and pgrep, Linux included. open, osascript, hdiutil,
# codesign, ditto, screencapture, shasum and PlistBuddy are fakes that log
# what they get; the "app" is a script that writes the log lines the real
# one writes. Nothing here shows that keys reach a real window.
# Exit status: 0 every check passed, 1 otherwise.

MAC=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
HARNESS=$(dirname "$MAC")
T=$(mktemp -d "${TMPDIR:-/tmp}/eol-mac-selftest.XXXXXX")
FAKES=$T/bin
DMG=$T/Euro-Office-Lite.dmg
export HARNESS_MAC_APP=$T/Applications/Euro-Office-Lite.app
export HARNESS_MAC_LOGDIR=$T/applog
export PLISTBUDDY=$FAKES/PlistBuddy
export FAKE_LOG=$T/fake.log
export HOME=$T/home
export PATH=$FAKES:$PATH

PASSED=0
FAILED=0

cleanup() {
  pkill -f "$HARNESS_MAC_APP/Contents/MacOS/" 2>/dev/null
  rm -rf "$T"
}
trap cleanup EXIT

check() {
  local name=$1
  shift
  if "$@"; then
    PASSED=$(( PASSED + 1 ))
    echo "ok   $name"
    return
  fi
  FAILED=$(( FAILED + 1 ))
  echo "FAIL $name"
}

# --- fakes -----------------------------------------------------------------------

fake() {
  cat > "$FAKES/$1"
  chmod +x "$FAKES/$1"
}

make_fakes() {
  mkdir -p "$FAKES" "$HOME" "$HARNESS_MAC_LOGDIR"
  fake PlistBuddy <<'EOF'
#!/bin/bash
[ -f "$3" ] || exit 1
case $2 in
  "Print :CFBundleExecutable") echo Euro-Office-Lite ;;
  "Print :CFBundleShortVersionString") echo 9.9.9-selftest ;;
  *) exit 1 ;;
esac
EOF
  fake shasum <<'EOF'
#!/bin/bash
shift 2
sha256sum "$@"
EOF
  fake codesign <<'EOF'
#!/bin/bash
echo "codesign $*" >> "$FAKE_LOG"
exit ${FAKE_CODESIGN_STATUS:-0}
EOF
  fake ditto <<'EOF'
#!/bin/bash
mkdir -p "$(dirname "$2")"
cp -R "$1" "$2"
EOF
  fake screencapture <<'EOF'
#!/bin/bash
printf 'PNG' > "${!#}"
EOF
  fake hdiutil <<'EOF'
#!/bin/bash
echo "hdiutil $*" >> "$FAKE_LOG"
if [ "$1" = detach ]; then
  rm -rf "${2:?}"/*
  exit 0
fi
dmg=${!#}
while [ $# -gt 0 ]; do
  [ "$1" = -mountpoint ] && mnt=$2
  shift
done
cp -R "$dmg.d/." "$mnt/"
EOF
  fake open <<'EOF'
#!/bin/bash
out=/dev/null
err=/dev/null
files=()
while [ $# -gt 0 ]; do
  case $1 in
    -a) app=$2; shift ;;
    --stdout) out=$2; shift ;;
    --stderr) err=$2; shift ;;
    *) files+=("$1") ;;
  esac
  shift
done
echo "open $app --stdout $out ${files[*]}" >> "$FAKE_LOG"
nohup "$app/Contents/MacOS/Euro-Office-Lite" "${files[@]}" >"$out" 2>"$err" </dev/null &
EOF
  fake osascript <<'EOF'
#!/bin/bash
echo "osascript $*" >> "$FAKE_LOG"
case "$*" in
  *windows.applescript*)
    kill -0 "$2" 2>/dev/null || exit 0
    printf 'AXStandardWindow\tdocument\n'
    [ -n "$FAKE_DIALOG" ] && printf 'AXDialog\t%s\n' "$FAKE_DIALOG"
    ;;
  *" to quit"*)
    [ -n "$FAKE_IGNORE_QUIT" ] || pkill -f "$HARNESS_MAC_APP/Contents/MacOS/"
    ;;
esac
exit 0
EOF
  # The disk image: hdiutil above mounts the folder next to it.
  local app=$DMG.d/Euro-Office-Lite.app
  mkdir -p "$app/Contents/MacOS"
  : > "$app/Contents/Info.plist"
  : > "$DMG"
  cat > "$app/Contents/MacOS/Euro-Office-Lite" <<'EOF'
#!/bin/bash
# FAKE_APP_MODE: ready writes "Document ready" to its js-debug log; fail
# writes "[OPEN] failed" to standard output, as the backend does.
log=$HARNESS_MAC_LOGDIR/js-debug-$$.log
echo "start $*" >> "$log"
sleep 1
case ${FAKE_APP_MODE:-ready} in
  ready) echo "[editor] Document ready" >> "$log" ;;
  fail) echo "[OPEN] failed: not a document" ;;
esac
while :; do
  sleep 0.2
done
EOF
  chmod +x "$app/Contents/MacOS/Euro-Office-Lite"
}

# --- helpers ---------------------------------------------------------------------

run_mac() { /bin/bash "$MAC/run.sh" "$@" >"$T/run.log" 2>&1; }

# result_value DIR CASE FIELD: one field of one case of DIR/result.json.
result_value() {
  python3 - "$1/result.json" "$2" "$3" <<'EOF'
import json, sys
result = json.load(open(sys.argv[1]))
case = [c for c in result["cases"] if c["case"] == sys.argv[2]][0]
print(case.get(sys.argv[3]))
EOF
}

result_package() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["package"])' "$1/result.json"; }

app_gone() { [ -z "$(pgrep -f "$HARNESS_MAC_APP/Contents/MacOS/")" ]; }

# --- checks without the app ------------------------------------------------------

test_syntax() {
  local f
  for f in "$HARNESS"/*.sh "$MAC"/*.sh; do
    bash -n "$f" || return 1
  done
  python3 -c 'import ast, sys; ast.parse(open(sys.argv[1]).read())' "$MAC/follow_log.py"
}

# ../run.sh still runs on its own: without a case it prints its usage and exits 2.
test_linux_entry() {
  bash "$HARNESS/run.sh" >/dev/null 2>&1
  [ $? -eq 2 ]
}

test_cases_parse_and_map() {
  (
    . "$MAC/run.sh"
    n=0
    for f in "$HARNESS"/cases/*.toml; do
      read_case "$f" || exit 1
      reason=$(skip_reason)
      [ -z "$reason" ] || { echo "$f: $reason"; exit 1; }
      n=$(( n + 1 ))
    done
    echo "     $n cases read, none skipped"
    [ "$n" -gt 0 ]
  )
}

test_chords() {
  (
    . "$MAC/lib.sh"
    [ "$(chord_statements ctrl+End)" = "key code 119 using {command down}" ] || exit 1
    [ "$(chord_statements ctrl+shift+s)" = 'keystroke "s" using {command down, shift down}' ] || exit 1
    [ "$(chord_statements Return)" = "key code 36" ] || exit 1
    [ "$(chord_statements F2)" = "key code 120" ] || exit 1
    [ "$(chord_statements shift+Tab)" = "key code 48 using {shift down}" ] || exit 1
    [ "$(chord_statements shift)" = $'key down shift\nkey up shift' ] || exit 1
    [ "$(HARNESS_MAC_CTRL=control; MAC_CTRL=control; chord_statements ctrl+End)" = "key code 119 using {control down}" ] || exit 1
    ! chord_statements KP_Enter >/dev/null || exit 1
    ! chord_statements 'ctrl+"' >/dev/null
  )
}

test_text_is_an_argument() {
  (
    . "$MAC/lib.sh"
    : > "$FAKE_LOG"
    send_text 'a "quoted" text'
    grep -qF 'keystroke (item 1 of argv) -e end run a "quoted" text' "$FAKE_LOG"
  )
}

test_skip_unknown_key() {
  local file=$T/skip.toml
  printf 'expect = "opens"\nkeys_before = "ctrl+End KP_Enter"\n' > "$file"
  (
    . "$MAC/run.sh"
    read_case "$file" || exit 1
    reason=$(skip_reason)
    [[ $reason == *KP_Enter* ]] || exit 1
    RESULTS=()
    record_skipped skip "$reason" 2>/dev/null
    printf '%s' "${RESULTS[0]}" | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(not (d["skipped"] is True and d["observed"] == "skipped" and d["expected"] == "opens"))'
  )
}

test_follower() {
  local d=$T/follow f i dead
  mkdir -p "$d/logs"
  echo old > "$d/logs/js-debug-1.log"
  python3 -I "$MAC/follow_log.py" "$d/out" "$d/pid" "$d/logs" "$d/stdout" &
  f=$!
  sleep 1
  echo new-js >> "$d/logs/js-debug-1.log"
  echo second > "$d/logs/js-debug-2.log"
  printf 'backend\npartial' > "$d/stdout"
  sleep 1
  sleep 0 &
  dead=$!
  wait "$dead"
  echo "$dead" > "$d/pid"
  for (( i = 0; i < 20; i++ )); do
    kill -0 "$f" 2>/dev/null || break
    sleep 0.5
  done
  kill "$f" 2>/dev/null && return 1
  grep -qx new-js "$d/out" && grep -qx second "$d/out" && grep -qx backend "$d/out" &&
    grep -qx partial "$d/out" && ! grep -qx old "$d/out"
}

# --- runs against the fake app ---------------------------------------------------

test_install_and_open() {
  : > "$FAKE_LOG"
  run_mac dotx-open --dmg "$DMG" --out "$T/run-open" || return 1
  [ "$(result_value "$T/run-open" dotx-open observed)" = opens ] || return 1
  [ "$(result_package "$T/run-open")" = 9.9.9-selftest ] || return 1
  grep -q '^codesign --verify --deep --strict' "$FAKE_LOG" || return 1
  grep -q 'Document ready' "$T/run-open/dotx-open/app.log" || return 1
  [ "$(result_value "$T/run-open" dotx-open screenshot)" = None ] || return 1
  app_gone
}

test_bad_signature_stops() {
  rm -rf "$HARNESS_MAC_APP"
  FAKE_CODESIGN_STATUS=1 run_mac dotx-open --dmg "$DMG" --out "$T/run-badsig"
  [ $? -eq 2 ] && [ ! -f "$T/run-badsig/result.json" ]
}

test_rejection_with_dialog() {
  FAKE_APP_MODE=fail FAKE_DIALOG="Cannot open" run_mac docx-encrypted --out "$T/run-reject" || return 1
  [ "$(result_value "$T/run-reject" docx-encrypted observed)" = rejected-with-dialog ] || return 1
  result_value "$T/run-reject" docx-encrypted detail | grep -q "OPEN\] failed.*dialog: 'Cannot open'" || return 1
  app_gone
}

test_failure_takes_screenshot() {
  FAKE_APP_MODE=fail run_mac docx-plain-text --out "$T/run-fail"
  [ $? -eq 1 ] || return 1
  [ "$(result_value "$T/run-fail" docx-plain-text observed)" = rejected-without-dialog ] || return 1
  [ "$(result_value "$T/run-fail" docx-plain-text screenshot)" = docx-plain-text/screen.png ] || return 1
  [ -s "$T/run-fail/docx-plain-text/screen.png" ]
}

test_close_falls_back_to_kill() {
  FAKE_IGNORE_QUIT=1 run_mac dotx-open --out "$T/run-forced" || return 1
  result_value "$T/run-forced" dotx-open detail | grep -q 'app close: forced' || return 1
  app_gone
}

test_refuses_while_running() {
  FAKE_IGNORE_QUIT=1 "$HARNESS_MAC_APP/Contents/MacOS/Euro-Office-Lite" >/dev/null 2>&1 &
  local pid=$!
  sleep 0.5
  run_mac dotx-open --out "$T/run-busy"
  local status=$?
  kill "$pid"
  wait "$pid" 2>/dev/null
  [ $status -eq 2 ]
}

test_status() {
  local out
  out=$(bash "$MAC/status.sh" "$DMG")
  grep -q 'version 9.9.9-selftest' <<< "$out" && grep -q 'IS the installed one' <<< "$out" &&
    grep -q 'codesign --verify: valid' <<< "$out"
}

test_backup_restore() {
  local data="$HOME/Library/Application Support/org.euro-office.lite"
  local cache="$HOME/Library/Caches/org.euro-office.lite"
  mkdir -p "$data/recover/s1" "$cache"
  echo a > "$data/recent-files.json"
  echo f > "$cache/fonts.bin"
  bash "$MAC/backup.sh" "$T/backup" >/dev/null || return 1
  echo b > "$data/recent-files.json"
  mkdir -p "$data/recover/s2"
  rm -f "$cache/fonts.bin"
  bash "$MAC/restore.sh" "$T/backup" | grep -q 'sha256 match' || return 1
  [ "$(cat "$data/recent-files.json")" = a ] && [ ! -e "$data/recover/s2" ] && [ -f "$cache/fonts.bin" ]
}

make_fakes
check "every script parses" test_syntax
check "../run.sh still runs on its own" test_linux_entry
check "every case file reads and maps to macOS keys" test_cases_parse_and_map
check "xdotool chords become System Events lines" test_chords
check "typed text goes to osascript as an argument" test_text_is_an_argument
check "a case with an unknown key is skipped with its reason" test_skip_unknown_key
check "follow_log.py copies only the new lines" test_follower
check "install from .dmg, open, quit" test_install_and_open
check "a failed codesign --verify stops the run" test_bad_signature_stops
check "a refused file with a dialog" test_rejection_with_dialog
check "a failed case gets a screenshot and exit 1" test_failure_takes_screenshot
check "an app that ignores quit is killed" test_close_falls_back_to_kill
check "the run refuses to start while the app runs" test_refuses_while_running
check "status.sh reads version, sha256 and .dmg" test_status
check "backup.sh and restore.sh round trip" test_backup_restore
echo "$PASSED passed, $FAILED failed"
[ "$FAILED" -eq 0 ]
