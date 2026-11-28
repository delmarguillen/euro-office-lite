# mac/lib.sh: the macOS side of the harness. Sourced, never run on its own.
# It takes from ../lib.sh the helpers that need no window (log, die,
# wait_for, find_new_pid, open_state, wait_open, json_str) and replaces the
# ones that use X11, dpkg or GNU tools. Works with the bash 3.2 of macOS.

MAC_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
. "$MAC_DIR/../lib.sh"

HARNESS_PLATFORM=macos
PLATFORM_LABEL=macOS
APP_NAME=Euro-Office-Lite
# HARNESS_MAC_APP and HARNESS_MAC_LOGDIR are there for selftest.sh.
APP_DIR=${HARNESS_MAC_APP:-/Applications/$APP_NAME.app}
LOG_DIR=${HARNESS_MAC_LOGDIR:-/tmp/euro-office-lite-$(id -u)}
CONFIG_DIR="$HOME/Library/Application Support/org.euro-office.lite"
CACHE_DIR="$HOME/Library/Caches/org.euro-office.lite"
PLISTBUDDY=${PLISTBUDDY:-/usr/libexec/PlistBuddy}
# The modifier the "ctrl" of a case file presses. Cmd+S saves on macOS; Cmd
# for the other shortcuts is an assumption, HARNESS_MAC_CTRL=control tries Ctrl.
MAC_CTRL=${HARNESS_MAC_CTRL:-command}
# The pids of the log followers of a run, one per line; set by mac/run.sh.
FOLLOWERS_FILE=${FOLLOWERS_FILE:-}

plist_value() { "$PLISTBUDDY" -c "Print :$2" "$1" 2>/dev/null; }

# bundle_binary APP: the executable the Info.plist of APP names.
bundle_binary() {
  local name
  name=$(plist_value "$1/Contents/Info.plist" CFBundleExecutable) || return 1
  echo "$1/Contents/MacOS/$name"
}

APP_BIN=$(bundle_binary "$APP_DIR") || APP_BIN=$APP_DIR/Contents/MacOS/$APP_NAME

sha256_of() {
  [ -f "$1" ] || return 0
  local sum
  read -r sum _ < <(shasum -a 256 "$1")
  echo "$sum"
}

iso_now() { date -u +%Y-%m-%dT%H:%M:%S+00:00; }

installed_version() { plist_value "$APP_DIR/Contents/Info.plist" CFBundleShortVersionString; }
installed_sha256() { sha256_of "$APP_BIN"; }

# Processes started from the bundle's MacOS folder.
app_pids() { pgrep -f "$APP_DIR/Contents/MacOS/"; }

screenshot() { screencapture -x "$1" 2>/dev/null; }

# --- disk images ---------------------------------------------------------------

# dmg_attach DMG: mounts DMG read-only on a new folder and prints the folder.
dmg_attach() {
  local mnt
  mnt=$(mktemp -d "${TMPDIR:-/tmp}/eol-dmg.XXXXXX") || return 1
  if ! hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$mnt" "$1" >/dev/null; then
    rmdir "$mnt"
    return 1
  fi
  echo "$mnt"
}

dmg_detach() {
  hdiutil detach "$1" -quiet || hdiutil detach "$1" -force -quiet
  rmdir "$1" 2>/dev/null
  return 0
}

dmg_app() { find "$1" -maxdepth 1 -name '*.app' -print -quit; }

# The sha256 of the app binary inside a .dmg, without installing it.
dmg_binary_sha256() {
  local mnt app bin sum=""
  mnt=$(dmg_attach "$1") || return 1
  app=$(dmg_app "$mnt")
  [ -n "$app" ] && bin=$(bundle_binary "$app") && sum=$(sha256_of "$bin")
  dmg_detach "$mnt"
  [ -n "$sum" ] && echo "$sum"
}

# --- windows -------------------------------------------------------------------

# window_lines PID: "SUBROLE<tab>TITLE" per window of PID and per sheet.
window_lines() { osascript "$MAC_DIR/windows.applescript" "$1" 2>/dev/null; }

has_main_window() { window_lines "$1" | grep -q '^AXStandardWindow'; }

# dialogs_of PID: "N TITLE" per window that is not a standard window:
# alerts, sheets and save panels.
dialogs_of() {
  local n=0 subrole title
  while IFS=$'\t' read -r subrole title; do
    [ -n "$subrole$title" ] || continue
    n=$(( n + 1 ))
    [ "$subrole" = AXStandardWindow ] && continue
    echo "$n ${title:-$subrole}"
  done < <(window_lines "$1")
}

activate_app() { osascript -e "tell application \"$APP_NAME\" to activate" >/dev/null; }

# --- keys ----------------------------------------------------------------------

# key_code NAME: the macOS virtual key code of an xdotool key name.
key_code() {
  case $1 in
    Return) echo 36 ;;
    Tab) echo 48 ;;
    space) echo 49 ;;
    BackSpace) echo 51 ;;
    Escape) echo 53 ;;
    Home) echo 115 ;;
    Page_Up) echo 116 ;;
    Delete) echo 117 ;;
    End) echo 119 ;;
    F2) echo 120 ;;
    Page_Down) echo 121 ;;
    Left) echo 123 ;;
    Right) echo 124 ;;
    Down) echo 125 ;;
    Up) echo 126 ;;
    *) return 1 ;;
  esac
}

modifier_of() {
  case $1 in
    ctrl) echo "$MAC_CTRL down" ;;
    shift) echo "shift down" ;;
    alt) echo "option down" ;;
    super) echo "command down" ;;
    *) return 1 ;;
  esac
}

# chord_statements CHORD: the System Events lines that press an xdotool
# chord such as ctrl+End. Fails for a key it has no equivalent for.
chord_statements() {
  local chord=$1 key rest part modifier modifiers="" using="" code
  local parts=()
  # A lone Shift press: the Markdown cases use it to wait.
  if [ "$chord" = shift ]; then
    echo "key down shift"
    echo "key up shift"
    return 0
  fi
  key=${chord##*+}
  rest=${chord%"$key"}
  rest=${rest%+}
  [ -n "$rest" ] && IFS=+ read -r -a parts <<< "$rest"
  for part in ${parts[@]+"${parts[@]}"}; do
    modifier=$(modifier_of "$part") || return 1
    modifiers="${modifiers:+$modifiers, }$modifier"
  done
  [ -n "$modifiers" ] && using=" using {$modifiers}"
  if code=$(key_code "$key"); then
    echo "key code $code$using"
    return 0
  fi
  [[ $key =~ ^[A-Za-z0-9]$ ]] || return 1
  echo "keystroke \"$key\"$using"
}

# system_events LINE...: runs the lines inside tell application "System Events".
system_events() {
  local args=(-e 'tell application "System Events"') line
  for line in "$@"; do
    args+=(-e "$line")
  done
  args+=(-e 'end tell')
  osascript "${args[@]}" >/dev/null
}

# send_keys KEYS: an xdotool key list, as the case files write it.
send_keys() {
  local chord line lines
  for chord in $1; do
    lines=()
    while IFS= read -r line; do
      lines+=("$line")
    done < <(chord_statements "$chord")
    [ ${#lines[@]} -gt 0 ] || { log "no macOS key for $chord"; return 1; }
    system_events "${lines[@]}"
    sleep 0.2
  done
}

# send_text TEXT: typed by System Events; TEXT goes in as an argument, never
# as script source.
send_text() {
  osascript -e 'on run argv' -e 'tell application "System Events" to keystroke (item 1 of argv)' -e 'end run' "$1" >/dev/null
}

send_keys_to() {
  activate_app
  sleep 0.3
  send_keys "$2"
}

# press_named PID LABEL: one try of press.applescript; its error, if any, goes
# to the harness log.
press_named() { osascript "$MAC_DIR/press.applescript" "$1" "$2" >/dev/null; }

# press_button PID LABEL: clicks the element of the window whose accessible
# name is LABEL. A few tries: the web view may still be filling its tree.
press_button() {
  local i
  activate_app
  for (( i = 0; i < 3; i++ )); do
    press_named "$1" "$2" && return 0
    sleep 2
  done
  return 1
}

is_presentation_window() {
  window_lines "$1" | grep -qE '^AXStandardWindow.*\.(pptx|ppsx|potx|ppt|pps|odp|otp)$'
}

# close_feature_tip PID: the presentation editor of a fresh profile opens
# with a "New: GIF playback in slideshow mode" tip over the slide. It is
# closed with its own button, not Escape, which the editor may take as a command.
close_feature_tip() {
  is_presentation_window "$1" || return 0
  if press_button "$1" "Got it"; then
    log "closed the tip with Got it"
    return 0
  fi
  log "no Got it tip in the window"
}

# click_window_center PID: a click of the mouse in the middle of the window:
# center.applescript says where and click.js posts the events of the mouse.
# The click of System Events (click.applescript) presses the UI element at the
# point and does nothing on the canvas of an editor, where a presentation
# needs the click to get into the text box; it is only what is left when the
# mouse click cannot be made, and the log says so.
click_window_center() {
  local point
  point=$(osascript "$MAC_DIR/center.applescript" "$1") || point=""
  # Unquoted on purpose: the two numbers are the two arguments of click.js.
  if [[ $point =~ ^-?[0-9]+\ -?[0-9]+$ ]] && osascript -l JavaScript "$MAC_DIR/click.js" $point >/dev/null; then
    return 0
  fi
  log "no mouse click in the middle of the window (point '$point'): System Events clicks there instead"
  osascript "$MAC_DIR/click.applescript" "$1" >/dev/null
}

# focus_document PID: brings the app to the front and clicks in the middle
# of its window, like the Linux harness.
focus_document() {
  activate_app
  sleep 0.5
  close_feature_tip "$1"
  click_window_center "$1"
  sleep 0.5
}

# type_save_path DIALOG PATH: the save panel takes the folder from Go to
# Folder (Cmd+Shift+G) and the name from its name field.
type_save_path() {
  local target=$2
  send_keys super+shift+g
  sleep 1
  send_text "$(dirname "$target")"
  send_keys Return
  sleep 1
  send_keys super+a
  send_text "$(basename "$target")"
  send_keys Return
}

dismiss_dialogs() {
  local i
  for (( i = 0; i < 5; i++ )); do
    has_dialog "$1" || return 0
    activate_app
    send_keys Return
    sleep 1
  done
}

# --- app -----------------------------------------------------------------------

# start_log_follower OUT: copies to OUT the lines the next app writes to its
# js-debug log and, through open --stdout and --stderr, to OUT.stdout and
# OUT.stderr. It stops when the pid launch_app writes to OUT.pid is gone.
start_log_follower() {
  local out=$1
  python3 -I "$MAC_DIR/follow_log.py" "$out" "$out.pid" "$LOG_DIR" "$out.stdout" "$out.stderr" >/dev/null 2>&1 </dev/null &
  [ -n "$FOLLOWERS_FILE" ] && echo $! >> "$FOLLOWERS_FILE"
  return 0
}

stop_log_followers() {
  local p
  [ -f "$FOLLOWERS_FILE" ] || return 0
  while read -r p; do
    kill "$p" 2>/dev/null
  done < "$FOLLOWERS_FILE"
  : > "$FOLLOWERS_FILE"
}

# launch_app FILE LOG: opens FILE with open -a and prints the app's pid once
# it has a window. With an empty FILE the app starts with no document.
launch_app() {
  local file=$1 out=$2 before pid="" i
  local args=(-a "$APP_DIR" --stdout "$out.stdout" --stderr "$out.stderr")
  [ -n "$file" ] && args+=("$file")
  before=" $(app_pids | tr '\n' ' ') "
  : > "$out"
  start_log_follower "$out"
  open "${args[@]}" || return 1
  for (( i = 0; i < 120; i++ )); do
    pid=$(find_new_pid "$before") && break
    sleep 0.5
  done
  [ -n "$pid" ] || return 1
  echo "$pid" > "$out.pid"
  wait_for 60 has_main_window "$pid" || return 1
  echo "$pid"
}

# close_app PID: prints closed (quit), forced (SIGTERM) or killed (SIGKILL).
close_app() {
  local pid=$1 how=killed
  osascript -e 'ignoring application responses' -e "tell application \"$APP_NAME\" to quit" -e 'end ignoring' >/dev/null 2>&1
  if wait_for 20 process_gone "$pid"; then
    how=closed
  else
    kill "$pid" 2>/dev/null
    if wait_for 10 process_gone "$pid"; then
      how=forced
    else
      kill -9 "$pid" 2>/dev/null
    fi
  fi
  stop_log_followers
  echo "$how"
}
