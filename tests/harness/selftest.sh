#!/bin/bash
# selftest.sh: checks the parts of run.sh that ask the page for its state, on
# any system with bash 4 and python3. It does not start the app: a stand-in
# for ci/eo-inspect.py answers what a file says, or runs the expression in
# node against a page made of a few objects. The checks that need node are
# skipped when it is not there.
# Exit status: 0 every check passed, 1 otherwise.

HARNESS=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/eol-selftest.XXXXXX")
export FAKE_ANSWER=$T/answer.txt
export FAKE_PAGE=$T/page.js

PASSED=0
FAILED=0
SKIPPED=0

cleanup() { rm -rf "$T"; }
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

# check_with TOOLS NAME COMMAND...: a check that needs every tool of TOOLS.
check_with() {
  local tools=$1 tool
  shift
  for tool in $tools; do
    command -v "$tool" >/dev/null && continue
    SKIPPED=$(( SKIPPED + 1 ))
    echo "skip $1 (no $tool)"
    return
  done
  check "$@"
}

# --- a stand-in for ci/eo-inspect.py ---------------------------------------------

# FAKE_MODE says what it does with an expression: answer the text of
# FAKE_ANSWER, name no page, or run it in node after the script FAKE_PAGE,
# which makes the document the expression sees.
make_fake_inspector() {
  cat > "$T/fake-inspect.py" <<'EOF'
import os
import subprocess

DEFAULT_PATH = '/socket'
MODE = os.environ.get('FAKE_MODE', 'answer')


class FakeSocket:
    def settimeout(self, seconds):
        pass


class Inspector:
    def __init__(self, host, port, path, timeout):
        self.sock = FakeSocket()

    def wait_target(self):
        if MODE == 'no-page':
            raise RuntimeError('no Target.targetCreated arrived')
        return 'page-1'

    def evaluate(self, target, expression):
        if MODE == 'node':
            return {'value': run_in_node(expression)}
        with open(os.environ['FAKE_ANSWER'], encoding='utf-8') as answer:
            return {'value': answer.read().rstrip('\n')}

    def close(self):
        pass


def run_in_node(expression):
    with open(os.environ['FAKE_PAGE'], encoding='utf-8') as page:
        script = page.read() + '\nprocess.stdout.write(String(' + expression + '));'
    return subprocess.run(['node', '-e', script], capture_output=True, text=True, check=True).stdout


def render(value):
    return value['value']
EOF
}

# in_run_sh COMMAND...: runs COMMAND with the functions of run.sh, a case made
# of the CASE_* variables of the caller, the stand-in for the inspector and no
# waiting between tries: a second in all for a probe or a `do`.
in_run_sh() {
  (
    . "$HARNESS/run.sh"
    HARNESS_INSPECT=${FAKE_INSPECT-$T/fake-inspect.py}
    sleep() { :; }
    DO_SECONDS=1
    DO_ERROR_SECONDS=1
    declare -A CASE=()
    CASE[probe]=${CASE_PROBE-word-text.js}
    CASE[probe_seconds]=1
    [ -n "${CASE_DO:-}" ] && CASE[do]=$CASE_DO
    [ -n "${CASE_MATCH_1:-}" ] && CASE[match_1]=$CASE_MATCH_1
    [ -n "${CASE_MATCH_2:-}" ] && CASE[match_2]=$CASE_MATCH_2
    "$@"
  )
}

# Prints "OBSERVED | DETAIL" after the probe of the case.
probe_outcome() {
  mkdir -p "$T/case"
  probe_page "$T/case"
  echo "$OBSERVED | $DETAIL"
}

do_outcome() {
  mkdir -p "$T/case"
  run_do "do" "$T/case"
  echo "$OBSERVED | $DETAIL"
}

# --- the scripts ---------------------------------------------------------------------

test_syntax() {
  bash -n "$HARNESS/run.sh" || return 1
  bash -n "$HARNESS/lib.sh" || return 1
  python3 -c 'import ast, sys; ast.parse(open(sys.argv[1]).read())' "$HARNESS/evaluate.py"
}

# Every probe, inside the expression the harness evaluates, is JavaScript.
test_probes_parse() {
  local probe
  for probe in "$HARNESS"/probes/*.js; do
    in_run_sh probe_expression "$probe" A1 > "$T/expression.js" || return 1
    node --check "$T/expression.js" || return 1
  done
}

# Every case with a probe is for Linux alone, and names a probe that exists.
test_probe_cases_are_for_linux() {
  local file
  for file in "$HARNESS"/cases/*.toml; do
    grep -Eq '^(probe|do|do_after_restart) = ' "$file" || continue
    grep -q '^platforms = "linux"$' "$file" || { echo "     $file: not for linux alone"; return 1; }
    probe_files_exist "$file" || return 1
  done
}

probe_files_exist() {
  local file=$1 name
  while read -r name; do
    [ -f "$HARNESS/probes/$name" ] && continue
    echo "     $file: no probes/$name"
    return 1
  done < <(sed -n -E 's/^(probe|do|do_after_restart) = "([^ "]+).*/\2/p' "$file")
}

# --- the outcomes of a probe ---------------------------------------------------------

test_state_as_expected() {
  printf 'pages=3\ntext=1 Preface heading ii\n' > "$FAKE_ANSWER"
  local outcome
  outcome=$(CASE_MATCH_1='^pages=3$' CASE_MATCH_2='heading ii$' in_run_sh probe_outcome)
  [ "$outcome" = "state-as-expected | state: 2 lines" ] || return 1
  cmp -s "$T/case/state.txt" "$FAKE_ANSWER"
}

test_state_mismatch() {
  printf 'pages=3\ntext=1 Preface heading 2\n' > "$FAKE_ANSWER"
  local outcome
  outcome=$(CASE_MATCH_1='^pages=3$' CASE_MATCH_2='heading ii$' in_run_sh probe_outcome)
  [ "$outcome" = "state-mismatch | state: 2 lines; not found: heading ii\$" ]
}

test_probe_error_fails() {
  echo 'PROBE-ERROR no wb in the page: this probe was written for another sdkjs' > "$FAKE_ANSWER"
  local outcome
  outcome=$(CASE_MATCH_1='^A1' in_run_sh probe_outcome)
  [ "$outcome" = "probe-failed | probe: PROBE-ERROR no wb in the page: this probe was written for another sdkjs" ]
}

test_no_page_fails() {
  local outcome
  outcome=$(FAKE_MODE=no-page CASE_MATCH_1='^A1' in_run_sh probe_outcome)
  [[ $outcome == "probe-failed | probe: the inspector on port "*" named no page" ]]
}

test_no_inspector() {
  local outcome
  outcome=$(FAKE_INSPECT=$T/none.py CASE_MATCH_1='^A1' in_run_sh probe_outcome)
  [ "$outcome" = "no-inspector | no $T/none.py" ]
}

test_probe_needs_a_match() {
  echo 'pages=3' > "$FAKE_ANSWER"
  local outcome
  outcome=$(in_run_sh probe_outcome)
  [ "$outcome" = "bad-case-file | a probe needs a match_N" ]
}

test_missing_probe_file() {
  local outcome
  outcome=$(CASE_PROBE=no-such-probe.js CASE_MATCH_1='^A1' in_run_sh probe_outcome)
  [ "$outcome" = "probe-failed | no probe probes/no-such-probe.js" ]
}

test_do_ok() {
  echo 'OK updated, fields=true' > "$FAKE_ANSWER"
  local outcome
  outcome=$(CASE_DO=word-update-fields.js in_run_sh do_outcome)
  [ "$outcome" = " | do: OK updated, fields=true" ]
}

test_do_error_fails() {
  echo 'PROBE-ERROR no asc_setBorderSide in the page: this probe was written for another sdkjs' > "$FAKE_ANSWER"
  local outcome
  outcome=$(CASE_DO=sheet-style-border.js in_run_sh do_outcome)
  [ "$outcome" = "do-failed | do: PROBE-ERROR no asc_setBorderSide in the page: this probe was written for another sdkjs" ]
}

test_do_without_page_is_a_probe_failure() {
  local outcome
  outcome=$(FAKE_MODE=no-page CASE_DO=word-update-fields.js in_run_sh do_outcome)
  [[ $outcome == "probe-failed | do: the inspector on port "*" named no page" ]]
}

# --- the probes, in node, against a page made of a few objects ------------------------

# A page with no editor, and one whose editor is the document editor: it has
# no workbook.
test_probe_names_what_is_missing() {
  local outcome
  echo 'var document = { querySelector: function () { return null; } };' > "$FAKE_PAGE"
  outcome=$(FAKE_MODE=node CASE_PROBE='sheet-cells.js A1' CASE_MATCH_1='^A1' in_run_sh probe_outcome)
  [ "$outcome" = "probe-failed | probe: PROBE-ERROR no editor in the page" ] || return 1
  cat > "$FAKE_PAGE" <<'EOF'
var editor = { asc_HaveFields: function () { return true; } };
var frame = { contentWindow: { Asc: { editor: editor } } };
var document = { querySelector: function () { return frame; } };
EOF
  outcome=$(FAKE_MODE=node CASE_PROBE='sheet-cells.js A1' CASE_MATCH_1='^A1' in_run_sh probe_outcome)
  [ "$outcome" = "probe-failed | probe: PROBE-ERROR no wb in the page: this probe was written for another sdkjs" ]
}

# fonts.js on a page with two families, one of which draws the smiling face.
test_fonts_probe() {
  cat > "$FAKE_PAGE" <<'EOF'
var picker = { getFontBySymbol: function (code) { return code === 0x263A ? "Liberation Sans" : ""; } };
var fonts = { g_font_infos: [{ Name: "Liberation Sans" }, { Name: "Carlito" }], FontPickerByCharacter: picker };
var frame = { contentWindow: { Asc: { editor: {} }, AscFonts: fonts } };
var document = { querySelector: function () { return frame; } };
EOF
  local outcome
  outcome=$(FAKE_MODE=node CASE_PROBE='fonts.js U+263A U+1F44D family:EOLite+Emoji family:Carlito' \
    CASE_MATCH_1='^drawn\.U\+263A=yes$' CASE_MATCH_2='^family\.Carlito=listed$' in_run_sh probe_outcome)
  [ "$outcome" = "state-as-expected | state: 9 lines" ] || return 1
  grep -qx 'pick.U+1F44D=none' "$T/case/state.txt" || return 1
  grep -qx 'drawn.U+1F44D=no' "$T/case/state.txt" || return 1
  grep -qx 'family.EOLite Emoji=missing' "$T/case/state.txt"
}

make_fake_inspector

check "every script parses" test_syntax
check_with node "every probe is JavaScript inside its expression" test_probes_parse
check "every case with a probe is for Linux alone and names probes that exist" test_probe_cases_are_for_linux
check "a probe whose matches are all there: state-as-expected" test_state_as_expected
check "a match that is not there: state-mismatch, named" test_state_mismatch
check "a probe that throws: probe-failed with its reason" test_probe_error_fails
check "an inspector that names no page: probe-failed" test_no_page_fails
check "no inspector script: no-inspector" test_no_inspector
check "a probe without a match_N: bad-case-file" test_probe_needs_a_match
check "a probe file that is not there: probe-failed" test_missing_probe_file
check "a do that answers OK goes on" test_do_ok
check "a do that throws: do-failed with its reason" test_do_error_fails
check "a do the inspector never ran: probe-failed" test_do_without_page_is_a_probe_failure
check_with node "a probe on a page without what it reads says what is missing" test_probe_names_what_is_missing
check_with node "fonts.js: families, picks and drawn" test_fonts_probe

echo "$PASSED passed, $FAILED failed, $SKIPPED skipped"
[ "$FAILED" -eq 0 ]
