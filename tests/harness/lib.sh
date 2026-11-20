# lib.sh: helpers shared by the harness scripts. Sourced, never run on its own.

APP_BIN=/usr/bin/euro-office-lite
APP_PACKAGE=euro-office-lite
# pgrep -x compares the kernel's 15-character process name.
APP_PROC=euro-office-lit
CONFIG_DIR=$HOME/.config/org.euro-office.lite
HARNESS_DISPLAY=${HARNESS_DISPLAY:-:99}

log() { echo "[harness] $*" >&2; }
die() { log "$*"; exit 2; }

x() { DISPLAY=$HARNESS_DISPLAY "$@"; }

iso_now() { date -Is; }

# wait_for SECONDS COMMAND...: retries COMMAND every half second.
wait_for() {
  local tries=$(( $1 * 2 ))
  shift
  local i
  for (( i = 0; i < tries; i++ )); do
    "$@" && return 0
    sleep 0.5
  done
  return 1
}

sha256_of() {
  [ -f "$1" ] || return 0
  local sum
  read -r sum _ < <(sha256sum "$1")
  echo "$sum"
}

installed_version() { dpkg-query -W -f='${Version}' "$APP_PACKAGE" 2>/dev/null; }
installed_sha256() { sha256_of "$APP_BIN"; }

# The sha256 of the app binary inside a .deb, without installing it.
deb_binary_sha256() {
  local sum
  read -r sum _ < <(dpkg-deb --fsys-tarfile "$1" | tar -xO usr/bin/euro-office-lite | sha256sum)
  echo "$sum"
}

app_pids() { pgrep -x "$APP_PROC"; }
process_gone() { ! kill -0 "$1" 2>/dev/null; }

recover_entries() { find "$CONFIG_DIR/recover" -mindepth 1 -maxdepth 1 2>/dev/null; }

# --- display -----------------------------------------------------------------

display_ready() { x xdpyinfo >/dev/null 2>&1; }

# Pids of the display processes this run started, for stop_display.
STARTED_XVFB=""
STARTED_OPENBOX=""

start_display() {
  if ! display_ready; then
    setsid nohup Xvfb "$HARNESS_DISPLAY" -screen 0 1600x1000x24 >/dev/null 2>&1 </dev/null &
    STARTED_XVFB=$!
    wait_for 10 display_ready || die "Xvfb did not start on $HARNESS_DISPLAY"
  fi
  pgrep -x openbox >/dev/null && return 0
  DISPLAY=$HARNESS_DISPLAY setsid nohup openbox >/dev/null 2>&1 </dev/null &
  STARTED_OPENBOX=$!
  sleep 1
}

# Stops only what start_display started; a display that was already there stays.
stop_display() {
  [ -n "$STARTED_OPENBOX" ] && kill "$STARTED_OPENBOX" 2>/dev/null
  [ -n "$STARTED_XVFB" ] && kill "$STARTED_XVFB" 2>/dev/null
  return 0
}

screenshot() { x import -window root "$1" 2>/dev/null; }

# --- windows -------------------------------------------------------------------

windows_of() { x xdotool search --all --onlyvisible --pid "$1" 2>/dev/null; }

# window_geometry WINDOW: prints "X Y WIDTH HEIGHT".
window_geometry() {
  local WINDOW X Y WIDTH HEIGHT SCREEN
  eval "$(x xdotool getwindowgeometry --shell "$1" 2>/dev/null)"
  echo "${X:-0} ${Y:-0} ${WIDTH:-0} ${HEIGHT:-0}"
}

window_width() {
  local left top width height
  read -r left top width height < <(window_geometry "$1")
  echo "$width"
}

# The editor window of a process: its widest visible window.
main_window() {
  local w width best="" best_width=0
  for w in $(windows_of "$1"); do
    width=$(window_width "$w")
    [ "$width" -gt "$best_width" ] || continue
    best=$w
    best_width=$width
  done
  [ -n "$best" ] && echo "$best"
}

# The process's other titled windows, one "ID TITLE" per line: its dialogs.
dialogs_of() {
  local main w name
  main=$(main_window "$1")
  for w in $(windows_of "$1"); do
    [ "$w" = "$main" ] && continue
    name=$(x xdotool getwindowname "$w" 2>/dev/null)
    [ -n "$name" ] && echo "$w $name"
  done
}

has_dialog() { [ -n "$(dialogs_of "$1")" ]; }

click_window_center() {
  local left top width height
  read -r left top width height < <(window_geometry "$1")
  x xdotool mousemove $(( left + width / 2 )) $(( top + height / 2 )) click 1
}

# --- app -----------------------------------------------------------------------

# find_new_pid " PID PID ": an app process that is not in the list.
find_new_pid() {
  local p
  for p in $(app_pids); do
    case $1 in *" $p "*) ;; *) echo "$p"; return 0 ;; esac
  done
  return 1
}

has_main_window() { [ -n "$(main_window "$1")" ]; }

# --- input ---------------------------------------------------------------------

# focus_document PID: brings the editor window up and clicks in its middle.
focus_document() {
  local w
  w=$(main_window "$1")
  x xdotool windowactivate --sync "$w"
  sleep 0.5
  click_window_center "$w"
  sleep 0.5
}

# send_keys KEYS: an xdotool key list, as the case files write it.
send_keys() {
  # shellcheck disable=SC2086 # a key list
  x xdotool key --delay 200 $1
}

send_text() { x xdotool type --delay 40 "$1"; }

# send_keys_to PID KEYS: KEYS to the editor window of PID.
send_keys_to() {
  # shellcheck disable=SC2086 # a key list
  x xdotool windowactivate --sync "$(main_window "$1")" key $2
}

# type_save_path DIALOG PATH: the dialog offers the name of the document,
# selected or not: replace it with PATH and confirm.
type_save_path() {
  x xdotool windowactivate --sync "$1"
  sleep 1
  send_keys ctrl+a
  send_text "$2"
  send_keys Return
}

# launch_app FILE LOG: starts the app on FILE and prints its pid once it has a window.
# With an empty FILE the app starts on its start screen, with no document.
# Its own session bus: the desktop portal of a systemd user session without
# DISPLAY delays every GTK start by about 30 s.
launch_app() {
  local file=$1 out=$2 before pid="" i
  before=" $(app_pids | tr '\n' ' ') "
  (
    cd "$(dirname "$file")" || exit 1
    env -u WAYLAND_DISPLAY DISPLAY="$HARNESS_DISPLAY" GDK_BACKEND=x11 XDG_SESSION_TYPE=x11 \
      setsid nohup dbus-run-session -- "$APP_BIN" ${file:+"$file"} >"$out" 2>&1 </dev/null &
  )
  for (( i = 0; i < 120; i++ )); do
    pid=$(find_new_pid "$before") && break
    sleep 0.5
  done
  [ -n "$pid" ] || return 1
  wait_for 60 has_main_window "$pid" || return 1
  echo "$pid"
}

# open_state LOG: "failed" or "ready" once the app log says so. A .md opens
# in the Markdown page, which says neither: the backend's [OPEN] markdown
# lines stand for them.
open_state() {
  grep -q '\[OPEN\] failed' "$1" && { echo failed; return 0; }
  grep -q '\[OPEN\] markdown failed:' "$1" && { echo failed; return 0; }
  grep -q 'Document ready' "$1" && { echo ready; return 0; }
  grep -q '\[OPEN\] markdown success' "$1" && { echo ready; return 0; }
  return 1
}

# wait_open LOG SECONDS: prints failed, ready or timeout.
wait_open() {
  local i
  for (( i = 0; i < $2 * 2; i++ )); do
    open_state "$1" && return 0
    sleep 0.5
  done
  echo timeout
}

dismiss_dialogs() {
  local id name
  while read -r id name; do
    [ -n "$id" ] || continue
    x xdotool windowactivate --sync "$id" key Return
    sleep 1
  done < <(dialogs_of "$1")
}

# close_app PID: prints closed, forced (SIGTERM) or killed (SIGKILL).
close_app() {
  local pid=$1 w
  w=$(main_window "$pid")
  [ -n "$w" ] && x wmctrl -i -c "$w"
  wait_for 20 process_gone "$pid" && { echo closed; return 0; }
  kill "$pid" 2>/dev/null
  wait_for 10 process_gone "$pid" && { echo forced; return 0; }
  kill -9 "$pid" 2>/dev/null
  echo killed
}

# --- JSON ----------------------------------------------------------------------

json_str() {
  local s
  s=$(printf '%s' "$1" | tr -d '\000-\010\013\014\016-\037')
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  s=${s//$'\n'/\\n}
  s=${s//$'\r'/\\r}
  s=${s//$'\t'/\\t}
  printf '"%s"' "$s"
}
