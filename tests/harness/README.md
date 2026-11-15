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
and `sudo -n apt-get` when it has to install a package.

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

## Case files

`cases/NAME.toml`, one `key = "value"` (or `key = 'value'`, taken
literally) per line, `#` comments. No other TOML syntax and no escapes.

| key | meaning |
|---|---|
| `description` | one line for humans |
| `generate` | `SCRIPT ARGS`: runs `python3 gen/SCRIPT OUT ARGS` to make the input |
| `input` | file name of the input |
| `input_sha256` | optional: the generated input must have this sha256 |
| `action` | `open`, `open-save` (Ctrl+S) or `open-edit-save` |
| `keys_before`, `type`, `keys_after` | for `open-edit-save`: `xdotool key` list, text typed, `xdotool key` list, after a click in the middle of the window |
| `check` | how a saved file is judged: `same-text`, `expected-bytes` or `zip-match` |
| `expected_generate`, `expected_sha256` | for `expected-bytes`: the generator of the file the save must write, and optionally its sha256 |
| `match_N` | for `zip-match`: `PART REGEX`, an extended regex the zip part must contain |
| `expect` | the outcome that makes the case ok |
| `control_expect` | optional: the outcome with `--control`, for a build without the fix |
| `screenshot` | `on-failure` or `always` |

Outcomes:

- `rejected-with-dialog`: the log has `[OPEN] failed`, a dialog opens and the
  file is unchanged. `rejected-without-dialog` and `rejected-but-file-changed`
  otherwise.
- `opens`: the document is ready (action `open`).
- `opens-whole`, `opens-empty`, `saved-changed` (`same-text`): after Ctrl+S
  the file is the same as the input, has nothing but a BOM and line breaks,
  or is something else.
- `saved-as-expected`, `saved-changed` (`expected-bytes`): the saved file is
  byte for byte the expected one, or not.
- `saved-as-expected`, `saved-missing-content` (`zip-match`).
- `save-not-written`: Ctrl+S did not change the file within 30 s. Current
  builds do not write a document with no changes, so a case that needs a save
  types something first.
- `open-timeout`, `launch-failed`, `no-input`, `bad-case-file`.

## Cases

Every case that saves first changes the document: `Z` typed at the end of the
last line of a TXT, or added to the last cell of a CSV. The save must then be
the input with that `Z` and nothing else changed.

| case | input | ok when |
|---|---|---|
| `csv-bmp` | UTF-8 CSV, every character inside the BMP | saved with the `Z`, byte for byte |
| `csv-emoji` | UTF-8 CSV with an emoji and U+20000 | saved with the `Z`, byte for byte |
| `csv-emoji-bom` | the same CSV with a UTF-8 BOM | saved with the `Z`, BOM kept |
| `csv-emoji-crlf` | the same CSV with CRLF | saved with the `Z`, CRLF kept |
| `csv-emoji-utf16le-bom` | the same CSV in UTF-16 LE with a BOM | saved with the `Z`, encoding and BOM kept |
| `csv-html-emoji` | `html-emoji.csv`: first cell `<html>`, and an emoji | opens as a sheet; saved with the `Z`, byte for byte |
| `docx-edit-save` | DOCX with three paragraphs | the saved XML has the typed word and the paragraphs |
| `txt-big-emoji-end` | TXT over 64 KiB, an emoji only in its last line | refused with the open error dialog |
| `txt-bmp` | UTF-8 TXT inside the BMP | saved with the `Z`, byte for byte |
| `txt-crlf` | ASCII TXT with CRLF | saved with the `Z`, CRLF kept |
| `txt-emoji` | UTF-8 TXT with an emoji | refused with the open error dialog, file unchanged |
| `txt-html` | TXT holding HTML | opens as text; saved with the `Z`, byte for byte |
| `txt-nul` | `hello world` and one NUL | opens, and Ctrl+S without edits writes nothing |
| `txt-utf16-bom-emoji` | UTF-16 LE TXT with a BOM and an emoji | refused with the open error dialog, file unchanged |
| `txt-utf16be-nobom` | UTF-16 BE TXT, no BOM, CRLF | saved with the `Z`, encoding, no BOM and CRLF kept |
| `txt-utf16le-nobom` | UTF-16 LE TXT, no BOM, CRLF | saved with the `Z`, encoding, no BOM and CRLF kept |
| `xlsx-save` | XLSX with a number, a formatted number and a date | after typing in A2, row 1 keeps its values |

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
