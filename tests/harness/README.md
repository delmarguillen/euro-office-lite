# Linux GUI harness

Runs the app from the `.deb` on a Linux x64 test machine with no screen
(Xvfb and openbox) through a list of cases, and writes what happened to
`result.json`. Every case is a data file; deciding whether a case passed
uses only text: the app log, the windows it opens (`xdotool`) and the bytes
on disk. A screenshot is taken only when a case fails, or always with
`--screenshots`.

The harness runs on the test machine itself. Copy this folder there (for
example with `scp -r`) and run it over SSH; it holds no host names or
credentials.

Needs: `xvfb`, `openbox`, `xdotool`, `wmctrl`, `x11-utils`, `imagemagick`
(screenshots), `unzip`, `python3` (input generators), `dbus-run-session`,
`pdftotext` from `poppler-utils` (the `pdf-text` check), and
`sudo -n apt-get` when it has to install a package.

## Use

```
./backup.sh ~/harness-backup          # recovery data, recent files, window state
./status.sh some.deb other.deb        # installed version and binary sha256
./run.sh all --deb some.deb           # all cases, installing some.deb if needed
./run.sh txt-emoji --control          # one case, judged by its control_expect
./restore.sh ~/harness-backup         # put the user state back, checked by sha256
```

`run.sh CASE|all [--deb FILE] [--control] [--screenshots] [--keep-display] [--out DIR]`. It
refuses to start while the app is running or while the recovery folder has
entries, so a recovery prompt never gets in the way. Without `--deb` the app
must be installed already; when it is not, run.sh exits 2. Results go to `--out`,
by default `$HARNESS_WORK/run-<date>` (`HARNESS_WORK` defaults to
`~/eol-harness-work`); each case has its folder there with the generated input
(`source/`), the copy the app opened and the app log. The display is
`$HARNESS_DISPLAY`, `:99` by default; when run.sh started Xvfb or openbox, it stops them on exit unless `--keep-display` is given. Exit status: 0 every case ok, 1 some
case failed, 2 the harness could not run.

The binary has no commit id in it, so `status.sh` tells builds apart by the
sha256 of `/usr/bin/euro-office-lite`, compared with the one inside each
`.deb` it is given. A Dev Build and a release can share a version string.

## Running in CI

`.github/workflows/harness-linux.yml` ("Harness (Linux)") runs `run.sh all`
against the `.deb` of a "Linux Dev Build" run, once per architecture, each on
a clean runner of its own: `ubuntu-22.04` with the artifact
`deb-x86_64-unknown-linux-gnu`, and `ubuntu-22.04-arm` with
`deb-aarch64-unknown-linux-gnu`. It starts:

- by hand, with the id of that build run in the `run_id` input;
- on its own after every Linux Dev Build that succeeds. The cases are then
  those of the default branch, whatever branch was built.

A job fails when run.sh exits 1 or 2. Every job has `result.json` in the
artifact `harness-linux-<arch>-result-<attempt>` (`<arch>` is `x86_64` or
`aarch64`), unless the harness could not run. A failed job also has
`harness-linux-<arch>-diagnostics-<attempt>`: `apt.log` and, per case, the
`app.log`, `app-restart.log`, `exported.txt` and `screen.png` it has. The job
installs `poppler-utils` for `pdftotext`, and the log of the harness step
says which `pdftotext` the `pdf-text` cases used.

The jobs are red today: `docx-xlsx-content` opens instead of being refused
because of a known defect of the app (see [Cases](#cases)).

The workflow needs nothing from a test machine: it installs the package with
`--deb` (the runner has `sudo` without a password), writes to a folder under
`$RUNNER_TEMP` with `--out`, and skips backup.sh and restore.sh because the
runner starts with no user state and is discarded after the job.

### Windows

`harness-windows.yml` ("Harness (Windows)") runs `win\run.ps1 all` against
the NSIS installer of a "Windows Dev Build" run, once per architecture, each
on a clean runner of its own: `windows-latest` with the artifact
`installer-x86_64-pc-windows-msvc`, and `windows-11-arm` with
`installer-aarch64-pc-windows-msvc`. It starts the same two ways as the Linux
workflow: by hand with the build run id in `run_id`, or after every Windows
Dev Build that succeeds.

The job makes the inputs with `gen/mkinputs.py` under `$RUNNER_TEMP`, then
starts `run.ps1 all -Installer <exe> -Inputs <dir> -Out <dir>` as a child
process, as described in [Windows](#windows), and fails when it exits 1 or 2.
Every job has `result.json` in the artifact
`harness-windows-<arch>-result-<attempt>`, unless the harness could not run.
A failed job also has `harness-windows-<arch>-diagnostics-<attempt>`: the
output of run.ps1 and, per case, the `app.log`, `app-restart.log`,
`exported.txt` and `screen.png` it has. The output of run.ps1 says which
`pdftotext` the `pdf-text` cases used.

The jobs are red today because of known defects of the app, not of the
harness: `csv-html-emoji` fails to open on Windows, and `docx-xlsx-content`
opens instead of being refused. The cases stay as they are.

## Windows

`win/` is the same harness for the Windows app installed from the NSIS
installer: `run.ps1`, `status.ps1`, `backup.ps1` and `restore.ps1` do what
their `.sh` namesakes do, and write `result.json` with the same schema.
`selftest.ps1` checks the parts that need no window, on any system with
PowerShell.

```
python gen\mkinputs.py C:\h\inputs           # inputs of every case, checked by sha256
win\backup.ps1 C:\h\backup                   # recovery data, recent files, window state
win\status.ps1 some-setup.exe                 # installed version, installer sha256
win\run.ps1 all -Installer some-setup.exe -Inputs C:\h\inputs -Out C:\h\run
win\restore.ps1 C:\h\backup                  # put the user state back, checked by sha256
```

`run.ps1 CASE|all [-Installer FILE] [-Inputs DIR] [-Control] [-Screenshots] [-Out DIR]`.
`-Installer` runs the installer silently (`/S`) first. Exit status: 0 every
case ok, 1 some case failed, 2 the harness could not run.

- The app must run on the desktop of an interactive session: keys and
  `SetForegroundWindow` need it. From a remote shell, start run.ps1 as a
  child process with its output in files, wait with a limit, and kill the
  whole tree when the limit is reached:

  ```
  $p = Start-Process powershell -PassThru `
      -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File C:\h\harness\win\run.ps1 all -Inputs C:\h\inputs -Out C:\h\run' `
      -RedirectStandardOutput C:\h\run.out -RedirectStandardError C:\h\run.err
  $null = $p.Handle
  if (-not $p.WaitForExit(1800000)) { taskkill /T /F /PID $p.Id }
  $p.ExitCode
  ```

  run.ps1 writes its progress to standard error; the exit status of the child
  is that of run.ps1.
- Inputs: `gen/mkinputs.py DIR` writes the input of every case to
  `DIR\CASE\` and stops when a file does not have the sha256 its case gives.
  On a machine without Python, run it elsewhere and copy `DIR` over;
  `-Inputs DIR` then takes the files from there.
- Copy files to the machine; do not move them. A moved file keeps its ACL.
- If the interactive session belongs to another user than the remote shell,
  grant that user the folders the shell created:
  `icacls DIR /grant USER:(OI)(CI)M /T`.
- `backup.ps1` and `restore.ps1` read `%APPDATA%` and `%LOCALAPPDATA%`: run
  them as the user of the session the app runs in. Like run.sh, run.ps1
  refuses to start while the app is running or its recovery folder has
  entries.
- run.ps1 closes the app with `WM_CLOSE`; it kills it only when a modified
  document could not be saved, or when the case is an `edit-kill-recover`.
- The `pdf-text` check reads the PDF with `pdftotext`: the one on the
  `PATH`, else the one Git for Windows has in `mingw64\bin`. run.ps1 writes
  which one, and its version, when it starts. Without either, the case ends
  in `no-pdftotext` and is not ok.
- The 17 TXT, CSV, `docx-edit-save` and `xlsx-save` cases take under
  4 minutes: about 13 s per save, 2-3 s per refused file, 38 s for `txt-nul`.
  The other cases have not been timed.
- `csv-html-emoji` fails today on Windows: the app does not open the file,
  a known defect of the app. With `docx-xlsx-content` (see [Cases](#cases)),
  `run.ps1 all` exits 1.

## macOS

`mac/` is the same harness for the app installed from the `.dmg` of the
"macOS Dev Build": `mac/run.sh`, `mac/status.sh`, `mac/backup.sh` and
`mac/restore.sh` do what their namesakes do, read the same case files and
write `result.json` with the same schema. `mac/run.sh` sources `../run.sh`
for the case logic (reading cases, generating inputs, the checks, the
result) and `mac/lib.sh` for everything that touches the system.
`mac/selftest.sh` checks it on any system with bash 4, python3 and pgrep,
Linux included, with fakes for `open`, `osascript`, `hdiutil`, `codesign`,
`ditto`, `screencapture`, `shasum` and `PlistBuddy`; it does not show that
keys reach a real window.

```
mac/backup.sh ~/harness-backup        # Application Support and Caches folders of the app
mac/status.sh some.dmg                # version from Info.plist, binary sha256, codesign, .dmg match
mac/run.sh all --dmg some.dmg         # all cases, installing some.dmg if needed
mac/restore.sh ~/harness-backup       # put the user state back, checked by sha256
mac/selftest.sh                       # no macOS needed
```

`mac/run.sh CASE|all [--dmg FILE] [--control] [--screenshots] [--out DIR]`.
Exit status as in run.sh.

- Bash 4 or later: `../run.sh` keeps a case in an associative array. Started
  with the bash 3.2 of macOS, `mac/run.sh` runs itself again with
  `/opt/homebrew/bin/bash` or `/usr/local/bin/bash` (`brew install bash`).
  The other macOS scripts work with bash 3.2.
- Permissions: the terminal or process that starts the harness needs
  Accessibility and Automation (System Events) for `osascript`, and Screen
  Recording for `screencapture`. The GitHub macOS images grant them to
  `/bin/bash` and `/usr/bin/osascript`.
- `--dmg` mounts the image with `hdiutil`, copies the `.app` to
  `/Applications/Euro-Office-Lite.app` with `ditto` when its binary differs
  from the installed one, and stops (exit 2) unless
  `codesign --verify --deep --strict` passes.
- The app is started with `open -a`; its log is
  `/tmp/euro-office-lite-<uid>/js-debug-*.log`. `mac/follow_log.py` copies
  to the `app.log` of the case the lines that log gets after the start, and
  the standard output and error of the app, which `open --stdout --stderr`
  write next to it (`app.log.stdout`, `app.log.stderr`).
- Keys go through System Events. The xdotool names of the case files map to
  key codes in `mac/lib.sh` (`key_code`); `ctrl` presses Cmd
  (`HARNESS_MAC_CTRL=control` makes it Ctrl), and `ctrl+End` is Cmd+End.
  A case with a key that has no mapping is skipped: `"ok": true`,
  `"observed": "skipped"`, `"skipped": true` and the key in `detail`. No case
  is skipped today.
- Dialogs are the windows of the app that are not `AXStandardWindow`, and
  their sheets (`mac/windows.applescript`).
- Save As to PDF types the folder into Go to Folder (Cmd+Shift+G) and the
  name into the name field: a `/` typed into the name field opens Go to
  Folder by itself.
- The app is closed with `quit` through AppleScript, then `kill` and
  `kill -9` when it is still there after 20 s and 10 s. `edit-kill-recover`
  kills it with `kill -9` as on Linux.
- `backup.sh` saves `~/Library/Application Support/org.euro-office.lite`
  (recovery, recent files) and `~/Library/Caches/org.euro-office.lite`
  (fonts) whole; `restore.sh` removes both and puts the saved ones back.
- The Markdown cases reach the Edit button with Shift+Tab. macOS moves the
  focus only between text fields unless Full Keyboard Access is on
  (`defaults write NSGlobalDomain AppleKeyboardUIMode -int 2`, before the
  app starts); the CI workflow turns it on.

## Case files

`cases/NAME.toml`, one `key = "value"` (or `key = 'value'`, taken
literally) per line, `#` comments. No other TOML syntax and no escapes.

| key | meaning |
|---|---|
| `description` | one line for humans |
| `generate` | `SCRIPT ARGS`: runs `python3 gen/SCRIPT OUT ARGS` to make the input |
| `input` | file name of the input |
| `input_sha256` | optional: the generated input must have this sha256. Left out for the deflated ZIPs of `mkdocx.py` and `mkxlsx.py`, whose bytes depend on the zlib of the Python that writes them |
| `action` | `open`, `open-save` (Ctrl+S), `open-edit-save`, `open-export-pdf` or `edit-kill-recover` |
| `keys_before`, `type`, `keys_after` | for `open-edit-save` and `edit-kill-recover`: `xdotool key` list, text typed, `xdotool key` list, after a click in the middle of the window |
| `check` | how a saved file is judged: `same-text`, `expected-bytes` or `zip-match`; for `open-export-pdf`, `pdf-text` |
| `expected_generate`, `expected_sha256` | for `expected-bytes`: the generator of the file the save must write, and optionally its sha256 |
| `match_N` | for `zip-match`: `PART REGEX`, an extended regex the zip part must contain; for `pdf-text`: `REGEX`, an extended regex a line of the PDF's text must contain |
| `expect` | the outcome that makes the case ok |
| `control_expect` | optional: the outcome with `--control`, for a build without the fix |
| `screenshot` | `on-failure` or `always` |

A case waits up to 120 s for the app log to say how the open went. Ready:
`Document ready` from an editor, or `[OPEN] markdown success` for a `.md`,
which opens on the Markdown page and has no editor to say it. Failed:
`[OPEN] failed` or `[OPEN] markdown failed:`; on Windows also `errorCode=`
in the log or a native dialog.

Outcomes:

- `rejected-with-dialog`: the open failed (see above), a dialog opens and the
  file is unchanged. `rejected-without-dialog` and `rejected-but-file-changed`
  otherwise.
- `opens`: the document is ready (action `open`).
- `opens-whole`, `opens-empty`, `saved-changed` (`same-text`): after Ctrl+S
  the file is the same as the input, has nothing but a BOM and line breaks,
  or is something else.
- `saved-as-expected`, `saved-changed` (`expected-bytes`): the saved file is
  byte for byte the expected one, or not.
- `saved-as-expected`, `saved-missing-content` (`zip-match`).
- `save-not-written`: within 30 s Ctrl+S neither changed the file nor, for a
  `.md`, left a new `[SAVE] markdown` line in the log. The editors do not
  write a document with no changes, so a case that needs a save types
  something first.
- `saved-unchanged`, `save-failed` (a `.md`): a `[SAVE] markdown success`
  line logged after the Ctrl+S and the file byte for byte as before, or a
  new `[SAVE] markdown failed:` line. Lines already in the log before the
  Ctrl+S do not count.
- `exported-as-expected`, `exported-missing-content` (`pdf-text`): every
  `match_N` is in the text of the PDF, or not. `exported-not-pdf`: the file
  does not start with `%PDF-`. `no-pdftotext`: the harness found no
  `pdftotext`; run.sh and run.ps1 write which one they use when they start.
- `export-no-dialog`: no Save As dialog within 20 s of Ctrl+Shift+S.
  `export-not-written`: no PDF within 120 s of the name being typed.
- `recovery-offered`, `recovery-not-offered` (`edit-kill-recover`): the log
  of the app started after the kill has `[RECOVER] candidates=N` with N at
  least 1, or not. `recovery-not-started`: the log of the first start has no
  `[RECOVER] begin session=`. `recovery-not-written`: the changes file of
  that session was still empty 30 s after the edit.
- `open-timeout`, `launch-failed`, `no-input`, `bad-case-file`.

### Export to PDF

`open-export-pdf` opens the input, presses Ctrl+Shift+S (Save As) and types
the path of `exported.pdf`, in the folder of the case, into the native
dialog, then Return. The app takes the format from the extension of the name.
The case waits until the file has content and its sha256 holds for two
seconds. The text `pdftotext` reads from it is kept as `exported.txt`.

### Recovery after a kill

`edit-kill-recover` opens the input and reads the id of its recovery session
from `[RECOVER] begin session=ID` in the log. It edits the document like
`open-edit-save`, then waits until `recover/ID/changes/changes0.json` in the
config folder of the app has content and its sha256 holds for two seconds.
It kills the app (`kill -9`, `Stop-Process -Force`) and starts it again with
no document; the log of that second start is `app-restart.log`. The case is
judged by the `[RECOVER] candidates=N` line the start screen leaves there.
It does not press Recover, so it does not show that the typed text comes
back. The second app is closed and the recovery folder is emptied, as after
every case.

## Cases

A TXT or CSV case that saves first changes the document: `Z` typed at the end
of the last line of a TXT, or added to the last cell of a CSV. The save must
then be the input with that `Z` and nothing else changed.

A Markdown case does the same on the Markdown page, which opens in preview:
its keys go from the preview to Edit with Shift+Tab, press it with Space,
wait for the editor with lone Shift presses and go to the end with
Ctrl+End. The `Z` lands after the final line break, so the save must be the
input with `Z` appended (`gen/mkmd.py`).

The inputs `gen/mkooxml.py`, `gen/mkodf.py` and `gen/mkcfb.py` write are
stored ZIPs (no compression) or plain bytes, so each has the same sha256 on
every system. A template (`.dotx`, `.xltx`, `.potx`) is only opened: Ctrl+S
writes in place the formats of `SAVE_IN_PLACE_EXTENSIONS`
(`src-tauri/src/file_dialogs.rs`), and the others go through Save As.

`docx-xlsx-content` fails today on both systems: the app takes any ZIP under
`.docx` and the converter reads this one as a spreadsheet, so the open is not
refused and `run.sh all` and `run.ps1 all` exit 1. The case stays as it is.

| case | input | ok when |
|---|---|---|
| `csv-bmp` | UTF-8 CSV, every character inside the BMP | saved with the `Z`, byte for byte |
| `csv-emoji` | UTF-8 CSV with an emoji and U+20000 | saved with the `Z`, byte for byte |
| `csv-emoji-bom` | the same CSV with a UTF-8 BOM | saved with the `Z`, BOM kept |
| `csv-emoji-crlf` | the same CSV with CRLF | saved with the `Z`, CRLF kept |
| `csv-emoji-utf16le-bom` | the same CSV in UTF-16 LE with a BOM | saved with the `Z`, encoding and BOM kept |
| `csv-html-emoji` | `html-emoji.csv`: first cell `<html>`, and an emoji | opens as a sheet; saved with the `Z`, byte for byte |
| `docx-edit-save` | DOCX with three paragraphs | the saved XML has the typed word and the paragraphs |
| `docx-encrypted` | `.docx` that is a CFB with the `EncryptionInfo` and `EncryptedPackage` streams of a document with a password; the streams hold filler | refused with the open error dialog, file unchanged |
| `docx-export-pdf` | DOCX with one paragraph | the text of the exported PDF has the paragraph |
| `docx-plain-text` | `.docx` that holds plain text | refused with the open error dialog, file unchanged |
| `docx-recover` | DOCX with two paragraphs | after typing and a kill, the next start gets the document to offer for recovery |
| `docx-truncated-zip` | the first half of a DOCX: a ZIP without its central directory | refused with the open error dialog, file unchanged |
| `docx-xlsx-content` | `.docx` that is an XLSX | refused with the open error dialog, file unchanged |
| `dotx-open` | DOTX template with one paragraph | opens |
| `md-crlf` | Markdown with CRLF | saved with the `Z`, CRLF kept |
| `md-emoji` | UTF-8 Markdown with an emoji and U+20000 | saved with the `Z`, byte for byte |
| `md-mixed-eol` | Markdown with CRLF, LF and a lone CR mixed | Ctrl+S without edits logs a save and keeps every byte (`saved-unchanged`) |
| `md-reject` | Markdown in Latin-1, not UTF-8 | refused with the open error dialog, file unchanged |
| `md-utf8` | UTF-8 Markdown, no BOM | saved with the `Z`, byte for byte |
| `md-utf8-bom` | the same Markdown with a UTF-8 BOM | saved with the `Z`, BOM kept |
| `odp-open` | ODP with one page covered by a text box | the saved `content.xml` has the typed word and the text |
| `ods-open` | ODS with a number in A1 and a text in B1 | after typing in A2, row 1 keeps its values |
| `odt-open` | ODT with one paragraph | the saved `content.xml` has the typed word and the paragraph |
| `potx-open` | POTX template with one slide | opens |
| `pptx-edit-save` | PPTX with one slide covered by a text box | the saved slide has the typed word and the text |
| `txt-big-emoji-end` | TXT over 64 KiB, an emoji only in its last line | refused with the open error dialog |
| `txt-bmp` | UTF-8 TXT inside the BMP | saved with the `Z`, byte for byte |
| `txt-crlf` | ASCII TXT with CRLF | saved with the `Z`, CRLF kept |
| `txt-emoji` | UTF-8 TXT with an emoji | refused with the open error dialog, file unchanged |
| `txt-html` | TXT holding HTML | opens as text; saved with the `Z`, byte for byte |
| `txt-nul` | `hello world` and one NUL | opens, and Ctrl+S without edits writes nothing |
| `txt-utf16-bom-emoji` | UTF-16 LE TXT with a BOM and an emoji | refused with the open error dialog, file unchanged |
| `txt-utf16be-nobom` | UTF-16 BE TXT, no BOM, CRLF | saved with the `Z`, encoding, no BOM and CRLF kept |
| `txt-utf16le-nobom` | UTF-16 LE TXT, no BOM, CRLF | saved with the `Z`, encoding, no BOM and CRLF kept |
| `xlsx-export-pdf` | the XLSX of `xlsx-save` | the text of the exported PDF has 1234.5 as `#,##0.00` shows it, with the separators of any locale |
| `xlsx-save` | XLSX with a number, a formatted number and a date | after typing in A2, row 1 keeps its values |
| `xltx-open` | XLTX template with a number and a text | opens |

## result.json

```json
{
  "date": "2026-11-15T01:00:00+00:00",
  "mode": "normal",
  "package": "0.17.48-alpha",
  "binary_sha256": "...",
  "cases": [
    {"case": "txt-emoji", "ok": true, "expected": "rejected-with-dialog",
     "observed": "rejected-with-dialog", "detail": "log: ...; dialog: '...'",
     "screenshot": null, "seconds": 14}
  ]
}
```

`screenshot` is the screenshot path relative to the folder of `result.json`,
or `null`.
