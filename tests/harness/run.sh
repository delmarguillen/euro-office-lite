#!/bin/bash
# run.sh CASE|all [--deb FILE] [--control] [--screenshots] [--keep-display] [--out DIR]
#
# Runs harness cases against the installed app on an X display of its own
# (Xvfb + openbox) and writes OUT/result.json. With --deb, installs that
# package first unless its binary is already the installed one. --control
# judges each case by its control_expect (what a build without the fix does).
# Exit status: 0 every case ok, 1 some case not ok, 2 the harness could not run.

HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/lib.sh"

SELECTION=""
DEB=""
CONTROL=false
ALWAYS_SHOOT=false
KEEP_DISPLAY=false
OUT=""

parse_args() {
  while [ $# -gt 0 ]; do
    case $1 in
      --deb) DEB=$2; shift ;;
      --control) CONTROL=true ;;
      --screenshots) ALWAYS_SHOOT=true ;;
      --keep-display) KEEP_DISPLAY=true ;;
      --out) OUT=$2; shift ;;
      -*) die "unknown option $1" ;;
      *) SELECTION=$1 ;;
    esac
    shift
  done
  [ -n "$SELECTION" ] || die "usage: run.sh CASE|all [--deb FILE] [--control] [--screenshots] [--keep-display] [--out DIR]"
  [ -n "$OUT" ] || OUT=${HARNESS_WORK:-$HOME/eol-harness-work}/run-$(date +%Y%m%d-%H%M%S)
}

selected_cases() {
  if [ "$SELECTION" != all ]; then
    echo "$SELECTION"
    return
  fi
  local f
  for f in "$HERE"/cases/*.toml; do
    basename "$f" .toml
  done
}

# --- case files: one key = "value" (or 'value') per line, comments with # ---------

declare -A CASE

read_case() {
  local file=$1 line
  CASE=()
  [ -f "$file" ] || { log "no case file $file"; return 1; }
  while IFS= read -r line || [ -n "$line" ]; do
    # A checkout with CRLF line endings must read the same.
    line=${line%$'\r'}
    [[ $line =~ ^[[:space:]]*(#.*)?$ ]] && continue
    if [[ $line =~ ^([a-z0-9_]+)[[:space:]]*=[[:space:]]*\"(.*)\"[[:space:]]*$ ]] ||
       [[ $line =~ ^([a-z0-9_]+)[[:space:]]*=[[:space:]]*\'(.*)\'[[:space:]]*$ ]]; then
      CASE[${BASH_REMATCH[1]}]=${BASH_REMATCH[2]}
      continue
    fi
    log "$file: cannot read line: $line"
    return 1
  done < "$file"
}

case_value() { echo "${CASE[$1]:-}"; }

case_matches() {
  local key
  for key in $(printf '%s\n' "${!CASE[@]}" | sort -V); do
    [[ $key == match_* ]] && echo "${CASE[$key]}"
  done
}

expected_outcome() {
  local control
  control=$(case_value control_expect)
  if $CONTROL && [ -n "$control" ]; then
    echo "$control"
    return
  fi
  case_value expect
}

# --- preparing a run -----------------------------------------------------------

check_machine_free() {
  local running
  running=$(app_pids | tr '\n' ' ')
  [ -z "$running" ] || die "the app is already running (pids $running): another test may be in progress"
  [ -z "$(recover_entries)" ] || die "$CONFIG_DIR/recover is not empty: save it with backup.sh and empty it first"
}

ensure_installed() {
  local deb want
  deb=$(readlink -f "$1")
  [ -f "$deb" ] || die "no package at $1"
  want=$(deb_binary_sha256 "$deb")
  if [ "$want" = "$(installed_sha256)" ]; then
    log "the binary of $(basename "$deb") is already installed"
    return 0
  fi
  log "installing $(basename "$deb")"
  sudo -n apt-get install -y --reinstall --allow-downgrades "$deb" >"$OUT/apt.log" 2>&1 ||
    die "apt-get failed, see $OUT/apt.log"
  [ "$want" = "$(installed_sha256)" ] || die "the installed binary is not the one in $deb"
}

# Without the app every case would wait a minute for it and end launch-failed.
check_app_installed() {
  [ -x "$APP_BIN" ] || die "$APP_BIN is not installed: give a package with --deb"
}

# --- one case --------------------------------------------------------------------

# Set by each step of a case.
OBSERVED=""
DETAIL=""
# The pid of the app a case started again after killing it.
RESTARTED_PID=""

note() { DETAIL="${DETAIL:+$DETAIL; }$*"; }

# generate SPEC OUT SHA256: runs the generator SPEC names into OUT and, when
# SHA256 is set, checks that OUT has it.
generate() {
  local spec=$1 out=$2 want=$3 gen args
  read -r gen args <<< "$spec"
  [ -f "$HERE/gen/$gen" ] || { note "no generator gen/$gen"; return 1; }
  # shellcheck disable=SC2086 # generator arguments are words
  python3 -I "$HERE/gen/$gen" "$out" $args || { note "generator failed: $spec"; return 1; }
  [ -z "$want" ] && return 0
  [ "$(sha256_of "$out")" = "$want" ] && return 0
  note "$(basename "$out") does not have the sha256 the case gives"
  return 1
}

generate_input() {
  generate "$(case_value generate)" "$1" "$(case_value input_sha256)"
}

# The file a save must write, for the expected-bytes check.
expected_file() { echo "$(dirname "$1")/expected-$(basename "$1")"; }

generate_expected() {
  local spec
  spec=$(case_value expected_generate)
  [ -n "$spec" ] || return 0
  generate "$spec" "$(expected_file "$1")" "$(case_value expected_sha256)"
}

observe_rejection() {
  local pid=$1 doc=$2 sha_before=$3 log_file=$4 dialog
  note "log: $(grep -m1 -E '\[OPEN\] (markdown )?failed' "$log_file")"
  if wait_for 10 has_dialog "$pid"; then
    dialog=$(dialogs_of "$pid" | head -1)
    OBSERVED=rejected-with-dialog
    note "dialog: '${dialog#* }'"
  else
    OBSERVED=rejected-without-dialog
  fi
  [ "$(sha256_of "$doc")" = "$sha_before" ] && return 0
  OBSERVED=rejected-but-file-changed
}

edit_document() {
  local pid=$1 w keys_before text keys_after
  keys_before=$(case_value keys_before)
  text=$(case_value type)
  keys_after=$(case_value keys_after)
  w=$(main_window "$pid")
  x xdotool windowactivate --sync "$w"
  sleep 0.5
  click_window_center "$w"
  sleep 0.5
  # shellcheck disable=SC2086 # a key list
  [ -n "$keys_before" ] && x xdotool key --delay 200 $keys_before
  [ -n "$text" ] && x xdotool type --delay 40 "$text"
  # shellcheck disable=SC2086 # a key list
  [ -n "$keys_after" ] && x xdotool key --delay 200 $keys_after
  sleep 1
}

file_changed() { [ "$(sha256_of "$1")" != "$2" ]; }

# new_save_lines LOG COUNT: the [SAVE] markdown lines LOG got after its first
# COUNT lines. The Markdown page saves through the backend, which logs both.
new_save_lines() {
  tail -n +"$(( $2 + 1 ))" "$1" | grep -E '\[SAVE\] markdown (success|failed)'
}

markdown_save_logged() { [ -n "$(new_save_lines "$1" "$2")" ]; }

# save_ended DOC SHA LOG COUNT: the file changed, or a .md save was logged.
save_ended() { file_changed "$1" "$2" || markdown_save_logged "$3" "$4"; }

# save_document PID DOC SHA LOG: Ctrl+S, then waits for the file to change or
# for a logged Markdown save. Returns 1, with OBSERVED set, when there are no
# saved bytes to check: save-not-written, save-failed or saved-unchanged.
save_document() {
  local pid=$1 doc=$2 sha_before=$3 log_file=$4 lines_before logged
  lines_before=$(wc -l < "$log_file")
  x xdotool windowactivate --sync "$(main_window "$pid")" key ctrl+s
  if ! wait_for 30 save_ended "$doc" "$sha_before" "$log_file" "$lines_before"; then
    OBSERVED=save-not-written
    return 1
  fi
  logged=$(new_save_lines "$log_file" "$lines_before")
  if grep -q 'markdown failed' <<< "$logged"; then
    OBSERVED=save-failed
    note "log: $(grep -m1 'markdown failed' <<< "$logged")"
    return 1
  fi
  if ! file_changed "$doc" "$sha_before"; then
    OBSERVED=saved-unchanged
    note "log: $(head -1 <<< "$logged")"
    return 1
  fi
  # Let the writer finish: the hash must hold for two seconds.
  local last=""
  while [ "$last" != "$(sha256_of "$doc")" ]; do
    last=$(sha256_of "$doc")
    sleep 2
  done
}

# Bytes of a text file that are not its BOM or line breaks.
text_payload_bytes() {
  local head skip=0
  head=$(head -c 3 "$1" | od -An -tx1 | tr -d ' \n')
  case $head in
    efbbbf*) skip=3 ;;
    fffe*|feff*) skip=2 ;;
  esac
  tail -c +$(( skip + 1 )) "$1" | tr -d '\r\n\000' | wc -c
}

check_same_text() {
  local doc=$1 source=$2
  note "saved $(wc -c < "$doc") bytes, original $(wc -c < "$source") bytes"
  if cmp -s "$doc" "$source"; then
    OBSERVED=opens-whole
  elif [ "$(text_payload_bytes "$doc")" -eq 0 ]; then
    OBSERVED=opens-empty
  else
    OBSERVED=saved-changed
  fi
}

check_expected_bytes() {
  local doc=$1 expected
  expected=$(expected_file "$2")
  note "saved $(wc -c < "$doc") bytes, expected $(wc -c < "$expected") bytes"
  if cmp -s "$doc" "$expected"; then
    OBSERVED=saved-as-expected
    return
  fi
  OBSERVED=saved-changed
  note "first difference: $(cmp "$doc" "$expected" 2>&1 | head -1)"
}

check_zip_match() {
  local doc=$1 match part pattern missing=""
  while IFS= read -r match; do
    part=${match%% *}
    pattern=${match#* }
    unzip -p "$doc" "$part" 2>/dev/null | grep -Eq -- "$pattern" && continue
    missing="${missing:+$missing, }$part: $pattern"
  done < <(case_matches)
  if [ -z "$missing" ]; then
    OBSERVED=saved-as-expected
    return
  fi
  OBSERVED=saved-missing-content
  note "not found: $missing"
}

# --- exporting to PDF ------------------------------------------------------------

# The PDF an open-export-pdf case writes, next to the document it opened.
export_file() { echo "$(dirname "$1")/exported.pdf"; }

has_content() { [ -s "$1" ]; }

# Let the writer finish: the hash must hold for two seconds.
wait_file_settled() {
  local last=""
  while [ "$last" != "$(sha256_of "$1")" ]; do
    last=$(sha256_of "$1")
    sleep 2
  done
}

export_ended() { has_content "$1" || grep -q 'SaveAs failed' "$2"; }

# save_as_pdf PID PDF LOG: Ctrl+Shift+S opens the native Save As dialog, and
# the app takes the format from the extension of the name typed there.
save_as_pdf() {
  local pid=$1 pdf=$2 log_file=$3 dialog
  rm -f "$pdf"
  x xdotool windowactivate --sync "$(main_window "$pid")" key ctrl+shift+s
  if ! wait_for 20 has_dialog "$pid"; then
    OBSERVED=export-no-dialog
    return 1
  fi
  dialog=$(dialogs_of "$pid" | head -1)
  note "dialog: '${dialog#* }'"
  x xdotool windowactivate --sync "${dialog%% *}"
  sleep 1
  # The dialog offers the name of the document, selected or not: replace it.
  x xdotool key --delay 200 ctrl+a
  x xdotool type --delay 40 "$pdf"
  x xdotool key --delay 200 Return
  # The first export of a fresh install builds the font cache before it writes.
  wait_for 120 export_ended "$pdf" "$log_file"
  if ! has_content "$pdf"; then
    OBSERVED=export-not-written
    note "log: $(grep -m1 'SaveAs failed' "$log_file")"
    return 1
  fi
  wait_file_settled "$pdf"
}

# Every match_N of the case is an extended regex some line of the PDF's text
# must contain. The text stays next to the PDF, for a case that fails.
check_pdf_text() {
  local pdf=$1 text=${1%.pdf}.txt pattern missing=""
  note "exported $(wc -c < "$pdf") bytes"
  if [ "$(head -c 5 "$pdf")" != "%PDF-" ]; then
    OBSERVED=exported-not-pdf
    return
  fi
  if ! command -v pdftotext >/dev/null; then
    OBSERVED=no-pdftotext
    return
  fi
  pdftotext -q -enc UTF-8 "$pdf" "$text"
  while IFS= read -r pattern; do
    grep -Eq -- "$pattern" "$text" && continue
    missing="${missing:+$missing, }$pattern"
  done < <(case_matches)
  if [ -z "$missing" ]; then
    OBSERVED=exported-as-expected
    return
  fi
  OBSERVED=exported-missing-content
  note "not found: $missing"
}

# Says in the log which pdftotext the pdf-text check uses, or that there is none.
log_pdftotext() {
  local tool version
  if ! tool=$(command -v pdftotext); then
    log "pdftotext: none found; pdf-text cases end in no-pdftotext"
    return
  fi
  # poppler prints its version to standard error.
  version=$(pdftotext -v 2>&1 | head -1)
  log "pdftotext: $tool ($version)"
}

export_pdf() {
  local pid=$1 pdf log_file=$3
  pdf=$(export_file "$2")
  save_as_pdf "$pid" "$pdf" "$log_file" || return 0
  case $(case_value check) in
    pdf-text) check_pdf_text "$pdf" ;;
    *) OBSERVED=exported; note "no check for this case" ;;
  esac
}

# --- recovery after a kill ---------------------------------------------------------

# The id in "[RECOVER] begin session=ID": the folder of the document under recover/.
recovery_session() {
  sed -n 's/.*\[RECOVER\] begin session=\([0-9]*-[0-9]*\).*/\1/p' "$1" | head -1
}

has_recovery_session() { [ -n "$(recovery_session "$1")" ]; }

# The file the app appends every change of a recovery session to.
recovery_changes() { echo "$CONFIG_DIR/recover/$1/changes/changes0.json"; }

# The number in "[RECOVER] candidates=N": the sessions the start screen got to offer.
recovery_candidates() {
  sed -n 's/.*\[RECOVER\] candidates=\([0-9]*\).*/\1/p' "$1" | head -1
}

has_recovery_candidates() { [ -n "$(recovery_candidates "$1")" ]; }

# Edits the document, waits for the app to record the changes, kills it and
# starts it again with no document. The case does not press Recover.
kill_and_recover() {
  local pid=$1 dir=$2 log_file=$3 session changes offered
  local restart_log=$dir/app-restart.log
  if ! wait_for 10 has_recovery_session "$log_file"; then
    OBSERVED=recovery-not-started
    return
  fi
  session=$(recovery_session "$log_file")
  changes=$(recovery_changes "$session")
  edit_document "$pid"
  if ! wait_for 30 has_content "$changes"; then
    OBSERVED=recovery-not-written
    return
  fi
  wait_file_settled "$changes"
  note "session $session: $(wc -c < "$changes") bytes of changes"
  kill -9 "$pid"
  wait_for 10 process_gone "$pid"
  if ! RESTARTED_PID=$(launch_app "" "$restart_log"); then
    OBSERVED=launch-failed
    RESTARTED_PID=$(app_pids | head -1)
    return
  fi
  OBSERVED=recovery-not-offered
  wait_for 60 has_recovery_candidates "$restart_log" || return 0
  offered=$(recovery_candidates "$restart_log")
  note "log: [RECOVER] candidates=$offered"
  [ "$offered" -ge 1 ] && OBSERVED=recovery-offered
  return 0
}

observe_open_document() {
  local pid=$1 doc=$2 source=$3 sha_before=$4 log_file=$5 action
  action=$(case_value action)
  note "log: $(grep -m1 -E '\[OPEN\] (markdown )?success' "$log_file")"
  # The editor keeps loading for a moment after it says it is ready.
  sleep 3
  if [ "$action" = open ]; then
    OBSERVED=opens
    return
  fi
  if [ "$action" = open-export-pdf ]; then
    export_pdf "$pid" "$doc" "$log_file"
    return
  fi
  if [ "$action" = edit-kill-recover ]; then
    kill_and_recover "$pid" "$(dirname "$doc")" "$log_file"
    return
  fi
  [ "$action" = open-edit-save ] && edit_document "$pid"
  save_document "$pid" "$doc" "$sha_before" "$log_file" || return
  case $(case_value check) in
    same-text) check_same_text "$doc" "$source" ;;
    zip-match) check_zip_match "$doc" ;;
    expected-bytes) check_expected_bytes "$doc" "$source" ;;
    *) OBSERVED=saved; note "no check for this case" ;;
  esac
}

wants_screenshot() {
  [ "$1" = true ] || return 0
  $ALWAYS_SHOOT && return 0
  [ "$(case_value screenshot)" = always ]
}

# Leaves the user's recovery folder as the run found it: empty.
clear_new_recovery() {
  local entries
  entries=$(recover_entries | tr '\n' ' ')
  [ -z "$entries" ] && return 0
  note "the case left recovery data, removed: $entries"
  recover_entries | xargs -r rm -rf --
}

RESULTS=()

record_case() {
  local name=$1 ok=$2 expected=$3 shot=$4 seconds=$5 shot_json=null
  [ -n "$shot" ] && shot_json=$(json_str "$shot")
  RESULTS+=("{\"case\": $(json_str "$name"), \"ok\": $ok, \"expected\": $(json_str "$expected"), \"observed\": $(json_str "$OBSERVED"), \"detail\": $(json_str "$DETAIL"), \"screenshot\": $shot_json, \"seconds\": $seconds}")
  log "$name: ok=$ok observed=$OBSERVED"
}

run_case() {
  local name=$1 started=$SECONDS expected dir source doc sha_before pid state ok shot=""
  OBSERVED=""
  DETAIL=""
  RESTARTED_PID=""
  if ! read_case "$HERE/cases/$name.toml"; then
    OBSERVED=bad-case-file
    record_case "$name" false "" "" 0
    return
  fi
  expected=$(expected_outcome)
  dir=$OUT/$name
  source=$dir/source/$(case_value input)
  doc=$dir/$(case_value input)
  mkdir -p "$dir/source"
  if ! generate_input "$source" || ! generate_expected "$source"; then
    OBSERVED=no-input
    record_case "$name" false "$expected" "" $(( SECONDS - started ))
    return
  fi
  cp "$source" "$doc"
  sha_before=$(sha256_of "$doc")
  if ! pid=$(launch_app "$doc" "$dir/app.log"); then
    OBSERVED=launch-failed
    pid=$(app_pids | head -1)
  else
    state=$(wait_open "$dir/app.log" 120)
    case $state in
      failed) observe_rejection "$pid" "$doc" "$sha_before" "$dir/app.log" ;;
      ready) observe_open_document "$pid" "$doc" "$source" "$sha_before" "$dir/app.log" ;;
      *) OBSERVED=open-timeout ;;
    esac
  fi
  # A case that started the app again leaves that one to close.
  [ -n "$RESTARTED_PID" ] && pid=$RESTARTED_PID
  ok=false
  [ "$OBSERVED" = "$expected" ] && ok=true
  if wants_screenshot "$ok"; then
    shot=$name/screen.png
    screenshot "$OUT/$shot" || shot=""
  fi
  if [ -n "$pid" ]; then
    dismiss_dialogs "$pid"
    local closed
    closed=$(close_app "$pid")
    [ "$closed" = closed ] || note "app close: $closed"
  fi
  clear_new_recovery
  record_case "$name" "$ok" "$expected" "$shot" $(( SECONDS - started ))
}

# --- result ---------------------------------------------------------------------

write_result() {
  local file=$OUT/result.json i mode=normal
  $CONTROL && mode=control
  {
    echo "{"
    echo "  \"date\": $(json_str "$(date -Is)"),"
    echo "  \"mode\": $(json_str "$mode"),"
    echo "  \"package\": $(json_str "$(installed_version)"),"
    echo "  \"binary_sha256\": $(json_str "$(installed_sha256)"),"
    echo "  \"cases\": ["
    for (( i = 0; i < ${#RESULTS[@]}; i++ )); do
      local comma=,
      [ $(( i + 1 )) -eq ${#RESULTS[@]} ] && comma=""
      echo "    ${RESULTS[$i]}$comma"
    done
    echo "  ]"
    echo "}"
  } > "$file"
  echo "$file"
}

all_ok() {
  local r
  for r in "${RESULTS[@]}"; do
    [[ $r == *'"ok": false'* ]] && return 1
  done
  return 0
}

main() {
  parse_args "$@"
  mkdir -p "$OUT" || die "cannot create $OUT"
  check_machine_free
  [ -n "$DEB" ] && ensure_installed "$DEB"
  check_app_installed
  log_pdftotext
  $KEEP_DISPLAY || trap stop_display EXIT
  start_display
  local name
  for name in $(selected_cases); do
    run_case "$name"
  done
  write_result
  all_ok || exit 1
}

main "$@"
