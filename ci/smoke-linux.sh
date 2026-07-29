#!/usr/bin/env bash
# Level 1 + 2 smoke tests for the Linux build of Euro-Office Lite.
#
# Level 1 (startup): the start screen renders the expected version and the three
# "new document" buttons, and the app log has no JS errors.
# Level 2 (editors): each of the three editors (word, cell, slide) reaches
# "Document ready" with "[OPEN] success" and no errors, with a fresh app process
# per editor.
#
# The app runs under Xvfb, so this works on a CI runner with no graphical
# session. Every assertion is a hard check on the DOM or on the app log: no
# screenshots, no tokens, no model in the loop.
#
# What this does NOT prove: anything that lives in pixels (rendering, GPU,
# system fonts), real keyboard/mouse input (clicks here are synthetic
# .click()), and anything below the webview (clipboard, keyboard layouts).
# Those stay on the physical test machines.
#
# Installation is optional so the assertions can be exercised against an
# already installed build (that is how they were validated before the first CI
# run). The expected version is an argument, never hardcoded: in CI it comes
# from src-tauri/tauri.conf.json of the checkout.
#
# Usage:
#   ci/smoke-linux.sh --version 0.17.9-alpha [--deb path/to/Euro-Office-Lite_*.deb]
#
# Options:
#   --version VER     version string the start screen must contain (required)
#   --deb PATH        install this .deb first (omit to test the installed app)
#   --display N       X display number for Xvfb (default 5)
#   --http-port N     WEBKIT_INSPECTOR_HTTP_SERVER port (default 3000)
#   --socket-port N   WEBKIT_INSPECTOR_SERVER port (default 2999)
#   --app-bin PATH    app binary (default: euro-office-lite from PATH)
#   --diag-dir PATH   where to copy logs for diagnosis (default /tmp/eo-smoke)
#   --skip-font       skip the default font assertion (see FONT_ASSERTION below)
#   --force-kill      kill an already running app instance instead of aborting
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
INSPECT="$SCRIPT_DIR/eo-inspect.py"

EXPECTED_VERSION="${EO_SMOKE_VERSION:-}"
DEB=""
DISPLAY_NUM="${EO_SMOKE_DISPLAY:-5}"
HTTP_PORT="${EO_SMOKE_HTTP_PORT:-3000}"
SOCKET_PORT="${EO_SMOKE_SOCKET_PORT:-2999}"
APP_BIN="${EO_SMOKE_APP_BIN:-euro-office-lite}"
DIAG_DIR="${EO_SMOKE_DIAG_DIR:-/tmp/eo-smoke}"
LAUNCH_LOG="${EO_SMOKE_LAUNCH_LOG:-/tmp/eo-launch.log}"
FORCE_KILL=0

# FONT_ASSERTION: the default font check (issue #32, Calibri in the three blank
# templates). It is a per-bug assertion, not a flow check, so it is kept
# separate: pass --skip-font to run the smoke against a build from before that
# fix.
FONT_ASSERTION=1
EXPECTED_FONT="${EO_SMOKE_FONT:-Calibri}"

# Under Xvfb the inspector takes ~45s to show up (measured twice on the test
# machine; 20-30s gives "no target"). A shared runner can be slower, so the
# budget is generous on purpose: the plan has zero tolerance for flaky tests.
TARGET_TIMEOUT="${EO_SMOKE_TARGET_TIMEOUT:-180}"
READY_TIMEOUT="${EO_SMOKE_READY_TIMEOUT:-180}"

while [ $# -gt 0 ]; do
    case "$1" in
        --version) EXPECTED_VERSION="$2"; shift 2 ;;
        --deb) DEB="$2"; shift 2 ;;
        --display) DISPLAY_NUM="$2"; shift 2 ;;
        --http-port) HTTP_PORT="$2"; shift 2 ;;
        --socket-port) SOCKET_PORT="$2"; shift 2 ;;
        --app-bin) APP_BIN="$2"; shift 2 ;;
        --diag-dir) DIAG_DIR="$2"; shift 2 ;;
        --skip-font) FONT_ASSERTION=0; shift ;;
        --force-kill) FORCE_KILL=1; shift ;;
        -h|--help) sed -n '2,40p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "ERROR: unknown option: $1" >&2; exit 2 ;;
    esac
done

# The app puts its log under $TMPDIR/euro-office-lite, so the launch has to use
# the same TMPDIR this script reads from or the assertions would look at a
# different file.
TMPROOT="${TMPDIR:-/tmp}"
APP_LOG="$TMPROOT/euro-office-lite/js-debug.log"

PASSED=0
XVFB_PID=""

say() { echo "[SMOKE] $*"; }
ok() { PASSED=$((PASSED + 1)); echo "[SMOKE] $1: OK${2:+ - $2}"; }

fail() {
    echo "[SMOKE] $1: FAILED" >&2
    shift
    [ $# -gt 0 ] && printf '%s\n' "$@" >&2
    exit 1
}

stop_app_quiet() {
    pkill -x euro-office-lit >/dev/null 2>&1 || true
    local waited=0
    while pgrep -x euro-office-lit >/dev/null 2>&1; do
        sleep 1
        waited=$((waited + 1))
        if [ "$waited" -eq 15 ]; then
            pkill -9 -x euro-office-lit >/dev/null 2>&1 || true
        fi
        if [ "$waited" -ge 25 ]; then
            echo "[SMOKE] WARNING: app process did not exit" >&2
            return 0
        fi
    done
}

cleanup() {
    local rc=$?
    stop_app_quiet
    if [ -n "$XVFB_PID" ]; then
        kill "$XVFB_PID" >/dev/null 2>&1 || true
    fi
    if [ "$rc" -ne 0 ]; then
        cp "$APP_LOG" "$DIAG_DIR/js-debug-final.log" 2>/dev/null || true
        say "diagnostic logs left in $DIAG_DIR and $LAUNCH_LOG"
    fi
}
trap cleanup EXIT

# --- preflight ---------------------------------------------------------------

[ -n "$EXPECTED_VERSION" ] || { echo "ERROR: --version is required (e.g. --version 0.17.9-alpha)" >&2; exit 2; }
[ -f "$INSPECT" ] || { echo "ERROR: eo-inspect.py not found next to this script ($INSPECT)" >&2; exit 2; }

for tool in python3 curl Xvfb pgrep pkill; do
    command -v "$tool" >/dev/null 2>&1 || { echo "ERROR: missing required tool: $tool" >&2; exit 2; }
done

if pgrep -x euro-office-lit >/dev/null 2>&1; then
    if [ "$FORCE_KILL" -eq 1 ]; then
        say "an app instance was already running, killing it (--force-kill)"
        stop_app_quiet
    else
        echo "ERROR: euro-office-lite is already running; another session may be using this machine. Re-run with --force-kill if it is a leftover." >&2
        exit 2
    fi
fi

mkdir -p "$DIAG_DIR"

# --- installation (optional) -------------------------------------------------

if [ -n "$DEB" ]; then
    DEB=$(readlink -f "$DEB")
    [ -s "$DEB" ] || { echo "ERROR: .deb not found or empty: $DEB" >&2; exit 2; }
    say "installing $(basename "$DEB")"
    # apt-get, not dpkg: it resolves the WebKitGTK runtime dependencies.
    sudo apt-get install -y "$DEB"
    say "installed package version: $(dpkg-query -W -f='${Version}' euro-office-lite 2>/dev/null || echo unknown)"
fi

command -v "$APP_BIN" >/dev/null 2>&1 || [ -x "$APP_BIN" ] || {
    echo "ERROR: app binary not found: $APP_BIN" >&2; exit 2; }

# --- harness -----------------------------------------------------------------

start_xvfb() {
    if [ -e "/tmp/.X11-unix/X$DISPLAY_NUM" ]; then
        echo "ERROR: display :$DISPLAY_NUM is already in use; pick another with --display" >&2
        exit 2
    fi
    Xvfb ":$DISPLAY_NUM" -screen 0 1920x1080x24 >"$DIAG_DIR/xvfb.log" 2>&1 &
    XVFB_PID=$!
    local waited=0
    while [ ! -e "/tmp/.X11-unix/X$DISPLAY_NUM" ]; do
        sleep 1
        waited=$((waited + 1))
        if [ "$waited" -ge 20 ]; then
            fail "xvfb" "Xvfb :$DISPLAY_NUM did not come up in ${waited}s" "$(cat "$DIAG_DIR/xvfb.log" 2>/dev/null || true)"
        fi
    done
    say "Xvfb running on :$DISPLAY_NUM (pid $XVFB_PID)"
}

# Launches the app and waits for the inspector target. Both inspector variables
# are needed: the socket port accepts connections but does not speak HTTP, the
# HTTP port is the one that serves the target list.
launch_app() {
    local phase="$1" waited=0
    : >"$LAUNCH_LOG"
    setsid nohup env \
        DISPLAY=":$DISPLAY_NUM" \
        TMPDIR="$TMPROOT" \
        WEBKIT_INSPECTOR_SERVER="127.0.0.1:$SOCKET_PORT" \
        WEBKIT_INSPECTOR_HTTP_SERVER="127.0.0.1:$HTTP_PORT" \
        "$APP_BIN" >"$LAUNCH_LOG" 2>&1 </dev/null &
    while [ "$waited" -lt "$TARGET_TIMEOUT" ]; do
        if curl -s --max-time 5 "http://127.0.0.1:$HTTP_PORT/" 2>/dev/null \
            | grep -q 'socket/[0-9]*/[0-9]*/WebPage'; then
            ok "inspector-target-$phase" "up after ${waited}s"
            return 0
        fi
        if ! pgrep -x euro-office-lit >/dev/null 2>&1 && [ "$waited" -ge 10 ]; then
            fail "inspector-target-$phase" "the app process died during startup" \
                "--- $LAUNCH_LOG ---" "$(tail -40 "$LAUNCH_LOG" 2>/dev/null || true)"
        fi
        sleep 3
        waited=$((waited + 3))
    done
    fail "inspector-target-$phase" \
        "no inspector target on 127.0.0.1:$HTTP_PORT after ${TARGET_TIMEOUT}s" \
        "--- $LAUNCH_LOG ---" "$(tail -40 "$LAUNCH_LOG" 2>/dev/null || true)"
}

# The inspector target shows up BEFORE the page has painted: measured on the
# test machine, one run had the start screen rendered at 33s and the very next
# one answered with an empty innerText at 30s. Readiness is therefore its own
# retried assertion, so the checks that follow can fail fast on a real mismatch
# instead of retrying a genuine red for a minute.
wait_rendered() {
    assert_js "webview-rendered-$1" 40 <<'JS'
(function () {
  var screen = document.getElementById("start-screen");
  if (!screen) return "FAIL: no #start-screen element yet";
  var box = screen.getBoundingClientRect();
  if (!box.width || !box.height) return "FAIL: #start-screen not rendered yet (" + box.width + "x" + box.height + ")";
  var text = (document.body.innerText || "").trim();
  if (!text) return "FAIL: document.body.innerText is still empty";
  return "OK start screen painted " + Math.round(box.width) + "x" + Math.round(box.height) + ", " + text.length + " chars";
})()
JS
}

# Evaluates a JS snippet (stdin) that must return a string starting with "OK".
# Anything else, or an inspector error, is a failed assertion. Optional second
# argument: how many times to retry before giving up (for state that arrives
# asynchronously).
assert_js() {
    local name="$1" retries="${2:-1}" js out rc attempt=1
    js=$(cat)
    while :; do
        set +e
        out=$(printf '%s' "$js" | python3 "$INSPECT" --port "$HTTP_PORT" --timeout 30 2>&1)
        rc=$?
        set -e
        if [ "$rc" -eq 0 ] && [ "${out#OK}" != "$out" ]; then
            ok "$name" "${out#OK }"
            return 0
        fi
        if [ "$attempt" -ge "$retries" ]; then
            fail "$name" "$out"
        fi
        attempt=$((attempt + 1))
        sleep 3
    done
}

# Waits for a literal line fragment in the app log.
wait_log() {
    local name="$1" pattern="$2" timeout="$3" waited=0
    while [ "$waited" -lt "$timeout" ]; do
        if grep -qF "$pattern" "$APP_LOG" 2>/dev/null; then
            ok "$name" "'$pattern' after ${waited}s"
            return 0
        fi
        sleep 3
        waited=$((waited + 3))
    done
    fail "$name" "'$pattern' never appeared in $APP_LOG after ${timeout}s" \
        "--- tail of $APP_LOG ---" "$(tail -40 "$APP_LOG" 2>/dev/null || true)"
}

assert_log_line() {
    local name="$1" pattern="$2"
    if grep -qF "$pattern" "$APP_LOG" 2>/dev/null; then
        ok "$name" "'$pattern'"
    else
        fail "$name" "'$pattern' not found in $APP_LOG" \
            "--- tail of $APP_LOG ---" "$(tail -40 "$APP_LOG" 2>/dev/null || true)"
    fi
}

assert_log_clean() {
    local name="$1" hits
    hits=$(grep -nE '\[JS-ERROR\]|\[IFRAME-ERROR\]' "$APP_LOG" 2>/dev/null || true)
    if [ -n "$hits" ]; then
        fail "$name" "the app log has JS errors:" "$hits"
    fi
    ok "$name" "no [JS-ERROR] / [IFRAME-ERROR]"
}

# The app truncates js-debug.log on every start, so the copy has to happen
# before the next launch or the evidence is gone.
save_log() {
    cp "$APP_LOG" "$DIAG_DIR/js-debug-$1.log" 2>/dev/null || true
    cp "$LAUNCH_LOG" "$DIAG_DIR/eo-launch-$1.log" 2>/dev/null || true
}

# --- level 1: startup --------------------------------------------------------

start_xvfb

say "expected version: $EXPECTED_VERSION"
say "app log: $APP_LOG"

launch_app "start"

wait_rendered "start"

assert_js "start-screen" <<JS
(function () {
  var want = "$EXPECTED_VERSION";
  // Whitespace normalized on both sides: a button's innerText is "W\nDocument"
  // while the body renders it across lines, so a raw indexOf never matches.
  var body = (document.body.innerText || "").replace(/\s+/g, " ");
  if (body.indexOf(want) < 0) {
    return "FAIL: the start screen does not show version '" + want + "'; innerText starts with: " +
      JSON.stringify(body.slice(0, 120));
  }
  var problems = [], labels = [];
  ["word", "cell", "slide"].forEach(function (type) {
    var btn = document.querySelector('#start-screen button.btn[data-type="' + type + '"]');
    if (!btn) { problems.push(type + ":missing"); return; }
    var box = btn.getBoundingClientRect();
    var label = (btn.innerText || "").trim().replace(/\s+/g, " ");
    if (!box.width || !box.height) problems.push(type + ":not-visible");
    else if (!label) problems.push(type + ":no-label");
    else if (body.indexOf(label) < 0) problems.push(type + ":label-not-in-body-innerText");
    labels.push(type + "='" + label + "'");
  });
  if (problems.length) return "FAIL: start screen buttons: " + problems.join(", ");
  return "OK version=" + want + " lang=" + (window._eoCurrentLang || "?") + " " + labels.join(" ");
})()
JS

assert_log_clean "startup-log-clean"

# --- level 2: the three editors load ----------------------------------------

# Assertion names carry the doc type so a red run says which editor broke.
run_editor() {
    local dtype="$1"
    local phase="editor-$dtype"

    assert_js "click-$dtype" <<JS
(function () {
  var btn = document.querySelector('#start-screen button.btn[data-type="$dtype"]');
  if (!btn) return "FAIL: no start screen button with data-type=$dtype";
  // .click() dispatches the real events; never fake interaction by setting
  // properties (that gives false greens with handlers that never ran).
  btn.click();
  return "OK clicked data-type=$dtype";
})()
JS

    wait_log "editor-ready-$dtype" "Document ready: docType=$dtype" "$READY_TIMEOUT"
    assert_log_line "open-success-$dtype" "[OPEN] success"
    assert_log_clean "$phase-log-clean"

    if [ "$FONT_ASSERTION" -eq 1 ] && [ "$dtype" = "word" ]; then
        # Per-bug assertion (issue #32): a new document must default to the
        # template font, not the format fallback. Retried because the toolbar
        # combo is populated shortly after "Document ready".
        assert_js "default-font-$dtype" 10 <<JS
(function () {
  var frame = document.querySelector("iframe");
  if (!frame) return "FAIL: no editor iframe";
  var box = frame.getBoundingClientRect();
  if (!box.width || !box.height) return "FAIL: the editor iframe is not rendered yet (" + box.width + "x" + box.height + ")";
  var doc = frame.contentDocument;
  var input = doc && doc.querySelector(".combobox.fonts input.form-control");
  if (!input) return "FAIL: font combo not found inside the editor iframe";
  var value = (input.value || "").trim();
  if (value !== "$EXPECTED_FONT") return "FAIL: the font combo shows '" + value + "', expected '$EXPECTED_FONT'";
  return "OK font=" + value;
})()
JS
    fi

    save_log "$dtype"
}

run_editor word

for dtype in cell slide; do
    stop_app_quiet
    launch_app "$dtype"
    wait_rendered "$dtype"
    run_editor "$dtype"
done

save_log "final"
say "all assertions passed ($PASSED)"
