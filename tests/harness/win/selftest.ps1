# selftest.ps1 [-Inputs DIR]: checks the parts of the Windows harness that need
# no window, on any system with PowerShell: the case files, the key table, the
# byte and zip checks, and a whole run of every case against a stand-in for the
# app that writes the log lines and files a fixed build would.
# It does not start the app, send a key or open a window.
# Exit status: 0 every check passed, 1 some check failed.
param(
    [string]$Inputs
)

Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'
# run.ps1 has a parameter of the same name, and dot-sourcing it resets it.
$GivenInputs = $Inputs
. (Join-Path $PSScriptRoot 'run.ps1')

$Failures = New-Object System.Collections.ArrayList
$Work = Join-Path ([System.IO.Path]::GetTempPath()) ('eol-harness-selftest-' + [System.Diagnostics.Process]::GetCurrentProcess().Id)

function Assert-Equal([string]$what, $actual, $expected) {
    if ([string]$actual -ceq [string]$expected) {
        Write-Output ('ok   ' + $what)
        return
    }
    [void]$Failures.Add($what)
    Write-Output ('FAIL ' + $what + ': got <' + $actual + '>, wanted <' + $expected + '>')
}

function Format-Codes($codes) {
    return (@($codes | ForEach-Object { '{0:X2}' -f $_ }) -join ' ')
}

function Write-Bytes([string]$name, [byte[]]$bytes) {
    $path = Join-Path $Work $name
    [System.IO.File]::WriteAllBytes($path, $bytes)
    return $path
}

function New-Zip([string]$path, $parts) {
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    $zip = [System.IO.Compression.ZipFile]::Open($path, [System.IO.Compression.ZipArchiveMode]::Create)
    foreach ($name in $parts.Keys) {
        $writer = New-Object System.IO.StreamWriter($zip.CreateEntry($name).Open())
        $writer.Write($parts[$name])
        $writer.Dispose()
    }
    $zip.Dispose()
}

# --- checks without the app ----------------------------------------------------

function Test-Scripts {
    foreach ($script in (Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1')) {
        $tokens = $null
        $errors = $null
        [void][System.Management.Automation.Language.Parser]::ParseFile($script.FullName, [ref]$tokens, [ref]$errors)
        Assert-Equal ('parses: ' + $script.Name) @($errors).Count 0
        $text = [System.IO.File]::ReadAllText($script.FullName)
        Assert-Equal ('ASCII only: ' + $script.Name) ([regex]::IsMatch($text, '[^\x00-\x7F]')) $false
    }
    Initialize-Native
    Assert-Equal 'the user32 declarations compile' ($null -ne ('EolNative' -as [type])) $true
}

function Test-CaseFiles {
    $names = @(Get-SelectedCases 'all')
    Assert-Equal 'number of cases' $names.Count 21
    foreach ($name in $names) {
        $case = Read-CaseFile (Join-Path (Join-Path $HarnessDir 'cases') ($name + '.toml'))
        $complete = ($null -ne $case) -and (Get-CaseValue $case 'input') -and (Get-CaseValue $case 'generate') -and (Get-CaseValue $case 'expect')
        Assert-Equal ('case reads: ' + $name) ([bool]$complete) $true
        Assert-Equal ('case keys have codes: ' + $name) (Test-CaseKeys $case) $true
    }
    $xlsx = Read-CaseFile (Join-Path (Join-Path $HarnessDir 'cases') 'xlsx-save.toml')
    $matches4 = @(Get-CaseMatches $xlsx)
    Assert-Equal 'xlsx-save has four matches' $matches4.Count 4
    Assert-Equal 'matches come in order' $matches4[3] 'xl/sharedStrings.xml Harness75'
    Assert-Equal 'a single-quoted value is literal' (Get-CaseValue $xlsx 'match_1') 'xl/worksheets/sheet1.xml <c r="A1"[^>]*>[[:space:]]*<v>1234.5</v>'
    $emoji = Read-CaseFile (Join-Path (Join-Path $HarnessDir 'cases') 'txt-emoji.toml')
    Assert-Equal 'expect' (Get-ExpectedOutcome $emoji $false) 'rejected-with-dialog'
    Assert-Equal 'control_expect with -Control' (Get-ExpectedOutcome $emoji $true) 'opens-empty'
    Assert-Equal 'expect when the case has no control_expect' (Get-ExpectedOutcome $xlsx $true) 'saved-as-expected'
    $bad = Join-Path $Work 'bad.toml'
    [System.IO.File]::WriteAllText($bad, 'input = unquoted')
    Assert-Equal 'a line outside the format is refused' ($null -eq (Read-CaseFile $bad)) $true
}

function Test-Keys {
    Assert-Equal 'ctrl+End' (Format-Codes (ConvertTo-KeyChord 'ctrl+End')) '11 23'
    Assert-Equal 'ctrl+Home' (Format-Codes (ConvertTo-KeyChord 'ctrl+Home')) '11 24'
    Assert-Equal 'ctrl+s' (Format-Codes (ConvertTo-KeyChord 'ctrl+s')) '11 53'
    Assert-Equal 'F2' (Format-Codes (ConvertTo-KeyChord 'F2')) '71'
    Assert-Equal 'F12' (Format-Codes (ConvertTo-KeyChord 'F12')) '7B'
    Assert-Equal 'End' (Format-Codes (ConvertTo-KeyChord 'End')) '23'
    Assert-Equal 'Down' (Format-Codes (ConvertTo-KeyChord 'Down')) '28'
    Assert-Equal 'Return' (Format-Codes (ConvertTo-KeyChord 'Return')) '0D'
    Assert-Equal 'ctrl+shift+7' (Format-Codes (ConvertTo-KeyChord 'ctrl+shift+7')) '11 10 37'
    $chords = ConvertTo-KeyChords 'ctrl+End F2 End'
    Assert-Equal 'a key list is one chord per word' $chords.Count 3
    Assert-Equal 'first chord of the list' (Format-Codes $chords[0]) '11 23'
    Assert-Equal 'an empty key list has no chords' (ConvertTo-KeyChords '').Count 0
    $refused = $false
    try { [void](ConvertTo-KeyChord 'ctrl+end') } catch { $refused = $true }
    Assert-Equal 'an unknown key name is refused' $refused $true
}

function Test-ByteChecks {
    $whole = Write-Bytes 'a.txt' ([byte[]](0x61, 0x0D, 0x0A, 0x62))
    $same = Write-Bytes 'b.txt' ([byte[]](0x61, 0x0D, 0x0A, 0x62))
    $other = Write-Bytes 'c.txt' ([byte[]](0x61, 0x0D, 0x0A, 0x63))
    $short = Write-Bytes 'd.txt' ([byte[]](0x61, 0x0D))
    $bomOnly = Write-Bytes 'e.txt' ([byte[]](0xEF, 0xBB, 0xBF, 0x0A))
    $utf16Break = Write-Bytes 'f.txt' ([byte[]](0xFF, 0xFE, 0x0D, 0x00, 0x0A, 0x00))
    Assert-Equal 'sha256 of a known file' (Get-Sha256 (Write-Bytes 'abc.txt' ([byte[]](0x61, 0x62, 0x63)))) 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
    Assert-Equal 'sha256 of a missing file is empty' (Get-Sha256 (Join-Path $Work 'none')) ''
    Assert-Equal 'equal files' (Test-SameBytes $whole $same) $true
    Assert-Equal 'different files' (Test-SameBytes $whole $other) $false
    Assert-Equal 'first different byte' (Get-FirstDifference $other $whole) 'byte 4: saved 0x63, expected 0x62'
    Assert-Equal 'a shorter file' (Get-FirstDifference $short $whole) 'byte 3: one file ends before it'
    Assert-Equal 'payload of text' (Get-TextPayloadCount $whole) 2
    Assert-Equal 'payload of a BOM and a line break' (Get-TextPayloadCount $bomOnly) 0
    Assert-Equal 'payload of a UTF-16 line break' (Get-TextPayloadCount $utf16Break) 0
}

function Test-ZipChecks([string]$inputs) {
    $docx = Join-Path (Join-Path $inputs 'docx-edit-save') 'edit.docx'
    $xlsx = Join-Path (Join-Path $inputs 'xlsx-save') 'cells.xlsx'
    Assert-Equal 'docx part has a paragraph' (Test-ZipPartMatch $docx 'word/document.xml' 'Paragraph 2: the quick brown fox') $true
    Assert-Equal 'docx part lacks the typed word' (Test-ZipPartMatch $docx 'word/document.xml' 'Harness75edit') $false
    Assert-Equal 'a missing part does not match' (Test-ZipPartMatch $docx 'word/none.xml' 'Paragraph') $false
    Assert-Equal 'a file that is no zip does not match' (Test-ZipPartMatch (Write-Bytes 'no.zip' ([byte[]](1, 2, 3))) 'a' 'a') $false
    Assert-Equal 'POSIX class in a pattern' (Test-ZipPartMatch $xlsx 'xl/worksheets/sheet1.xml' '<c r="B1"[^>]*>[[:space:]]*<v>1234.5</v>') $true
    Assert-Equal 'a pattern that is not there' (Test-ZipPartMatch $xlsx 'xl/worksheets/sheet1.xml' '<c r="A2"') $false
    Assert-Equal 'a pattern matches within one line, like grep' (Test-ZipPartMatch $xlsx 'xl/worksheets/sheet1.xml' '<sheetData>[[:space:]]*<row') $false
}

# The ways an open that failed shows, and those of an open that worked.
function Test-OpenSignals {
    $Fake.Alive = $true
    $log = Join-Path $Work 'signals.log'
    $run = New-Run 'signals'
    $run.Log = $log
    [System.IO.File]::WriteAllText($log, '[STARTUP] begin')
    Assert-Equal 'no signal yet' (Get-OpenState $run) ''
    [System.IO.File]::WriteAllText($log, '[EO] Document ready: docType=cell')
    Assert-Equal 'Document ready' (Get-OpenState $run) 'ready'
    [System.IO.File]::WriteAllText($log, 'load failed errorCode=-82 errorDescription=x')
    Assert-Equal 'errorCode= from the editor' (Get-OpenState $run) 'failed'
    [System.IO.File]::WriteAllText($log, '[OPEN] failed file=a.txt error=x')
    Assert-Equal '[OPEN] failed' (Get-OpenState $run) 'failed'
    [System.IO.File]::WriteAllText($log, '[OPEN] markdown success bytes=68')
    Assert-Equal '[OPEN] markdown success' (Get-OpenState $run) 'ready'
    [System.IO.File]::WriteAllText($log, '[OPEN] markdown failed: not a Markdown file')
    Assert-Equal '[OPEN] markdown failed' (Get-OpenState $run) 'failed'
    [System.IO.File]::WriteAllText($log, '[STARTUP] begin')
    $Fake.Dialogs = @(New-Object PSObject -Property @{ Handle = [IntPtr]2; Title = 'Error' })
    Assert-Equal 'a native dialog' (Get-OpenState $run) 'failed'
    Assert-Equal 'a line of the log' (Get-LogLine $log 'STARTUP') '[STARTUP] begin'
    $Fake.Dialogs = @()
    $Fake.Alive = $false
}

function Test-Json {
    $text = 'a"b\c' + [char]10 + [char]9 + [char]1 + 'd'
    Assert-Equal 'JSON string escapes' (ConvertTo-JsonString $text) '"a\"b\\c\n\td"'
}

# --- a stand-in for the app -----------------------------------------------------

$Fake = @{ ProcessId = 70000; Alive = $false; File = ''; Typed = ''; Dialogs = @(); Chords = @(); Refuses = $true; WritesNothing = $false }

function Start-Sleep { }
function Test-Windows { return $true }
function Stop-Harness([string]$message) { throw ('stopped: ' + $message) }
function Get-AppExe { return (Join-Path $Work 'fake-app.exe') }
function Get-InstalledVersion { return '0.0.0-selftest' }
function Get-AppProcessIds { return @() }
function Test-ProcessAlive([int]$processId) { return $Fake.Alive }
function Set-Foreground([IntPtr]$window) { return $true }
function Click-WindowCenter([IntPtr]$window) { }
function Click-YesButton([IntPtr]$dialog) { return $false }
function Close-Window([IntPtr]$window) { $Fake.Alive = $false }
function Stop-ProcessTree([int]$processId) { $Fake.Alive = $false }
function Get-Dialogs([int]$processId) { return $Fake.Dialogs }
function Send-Text([string]$text) { $Fake.Typed += $text }

function Get-MainWindow([int]$processId) {
    if ($Fake.Alive) { return [IntPtr]1 }
    return [IntPtr]::Zero
}

function Save-Screenshot([string]$path) {
    [System.IO.File]::WriteAllText($path, 'not a picture')
    return $true
}

# Writes the log an instance would: a TXT with an emoji is refused with a
# dialog unless the stand-in plays a build without that fix, and a .md opens
# in the Markdown page, which logs only the backend's line.
function Start-App([string]$exe, [string]$file) {
    $Fake.ProcessId++
    $Fake.Alive = $true
    $Fake.File = $file
    $Fake.Typed = ''
    $Fake.Dialogs = @()
    $Fake.Chords = @()
    $name = Split-Path -Leaf $file
    $lines = @('[OPEN] success editor_bin=10 bytes media_files=0', '[EO] Document ready: docType=word')
    if ($name -like '*.md') { $lines = @('[OPEN] markdown success bytes=10') }
    if ($Fake.Refuses -and $name -like '*emoji*.txt') {
        $lines = @('[OPEN] failed file=' + $name + ' error=character above U+FFFF')
        $Fake.Dialogs = @(New-Object PSObject -Property @{ Handle = [IntPtr]2; Title = 'Cannot open' })
    }
    New-Item -ItemType Directory -Force -Path (Get-TempRoot) | Out-Null
    $log = Join-Path (Get-TempRoot) ('js-debug-1700000000000-{0}.log' -f $Fake.ProcessId)
    [System.IO.File]::WriteAllLines($log, [string[]]$lines)
    return $Fake.ProcessId
}

# What Ctrl+S leaves on disk: nothing without an edit, else the expected file
# of the case, or a package with the typed text.
function Save-FakeDocument {
    $name = Split-Path -Leaf $Fake.File
    if ($Fake.WritesNothing) { return }
    if (-not $Fake.Refuses -and $name -like '*emoji*.txt') {
        [System.IO.File]::WriteAllBytes($Fake.File, [byte[]](0x0A))
        return
    }
    if (-not $Fake.Typed) { return }
    $expected = Get-ExpectedFile (Join-Path (Join-Path (Split-Path -Parent $Fake.File) 'source') $name)
    if (Test-Path -LiteralPath $expected) {
        Copy-Item -LiteralPath $expected -Destination $Fake.File -Force
        return
    }
    if ($name -like '*.docx') {
        New-Zip $Fake.File @{ 'word/document.xml' = '<w:t>Paragraph 0: the quick brown fox</w:t><w:t>Paragraph 2: the quick brown fox' + $Fake.Typed + '</w:t>' }
        return
    }
    $row = '<row r="1"><c r="A1"><v>1234.5</v></c><c r="B1" s="1">  <v>1234.5</v></c><c r="C1" s="2"><v>46086</v></c></row>'
    New-Zip $Fake.File @{ 'xl/worksheets/sheet1.xml' = $row; 'xl/sharedStrings.xml' = '<t>' + $Fake.Typed + '</t>' }
}

function Send-KeyChord([int[]]$codes) {
    $chord = Format-Codes $codes
    $Fake.Chords += $chord
    if ($chord -eq '0D') { $Fake.Dialogs = @() }
    if ($chord -eq '11 53') { Save-FakeDocument }
}

function New-FakeOptions([string]$selection, [string]$inputs, [bool]$control, [string]$outName) {
    $options = New-Options $selection '' $inputs $control $false (Join-Path $Work $outName)
    $options.SaveWait = 1
    return $options
}

function Read-Result($options) {
    return (Get-Content -LiteralPath (Join-Path $options.Out 'result.json') -Raw | ConvertFrom-Json)
}

function Test-FullRun([string]$inputs) {
    $options = New-FakeOptions 'all' $inputs $false 'run-all'
    $status = Invoke-Harness $options
    $result = Read-Result $options
    $notOk = @($result.cases | Where-Object { -not $_.ok } | ForEach-Object { $_.case + '=' + $_.observed })
    Assert-Equal 'all: exit status' $status 0
    Assert-Equal 'all: cases in result.json' @($result.cases).Count 21
    Assert-Equal 'all: cases not ok' ($notOk -join ' ') ''
    Assert-Equal 'all: mode' $result.mode 'normal'
    Assert-Equal 'all: package' $result.package '0.0.0-selftest'
    Assert-Equal 'all: binary_sha256 is the exe' $result.binary_sha256 (Get-Sha256 (Get-AppExe))
    Assert-Equal 'all: date has an offset' ([regex]::IsMatch([string](Get-Content -LiteralPath (Join-Path $options.Out 'result.json') -Raw), '"date": "\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d"')) $true
    $first = $result.cases[0]
    Assert-Equal 'all: fields of a case' (@($first.PSObject.Properties | ForEach-Object { $_.Name }) -join ' ') 'case ok expected observed detail screenshot seconds'
    Assert-Equal 'all: no screenshot for a case that is ok' ($null -eq $first.screenshot) $true
    Assert-Equal 'all: the log of a case is kept' (Test-Path -LiteralPath (Join-Path (Join-Path $options.Out 'csv-bmp') 'app.log')) $true
    Assert-Equal 'all: keys of the last case' ($Fake.Chords -join ', ') '11 24, 28, 0D, 11 53'
    Assert-Equal 'all: the app of the last case was closed' $Fake.Alive $false
}

function Test-FailingRun([string]$inputs) {
    $Fake.WritesNothing = $true
    $options = New-FakeOptions 'txt-crlf' $inputs $false 'run-fail'
    $status = Invoke-Harness $options
    $Fake.WritesNothing = $false
    $only = (Read-Result $options).cases[0]
    Assert-Equal 'fail: exit status' $status 1
    Assert-Equal 'fail: observed' $only.observed 'save-not-written'
    Assert-Equal 'fail: ok' $only.ok $false
    Assert-Equal 'fail: screenshot path' $only.screenshot 'txt-crlf/screen.png'
}

function Test-ControlRun([string]$inputs) {
    $Fake.Refuses = $false
    $options = New-FakeOptions 'txt-emoji' $inputs $true 'run-control'
    $status = Invoke-Harness $options
    $Fake.Refuses = $true
    $result = Read-Result $options
    Assert-Equal 'control: exit status' $status 0
    Assert-Equal 'control: mode' $result.mode 'control'
    Assert-Equal 'control: observed' $result.cases[0].observed 'opens-empty'
}

function Test-BadInputs([string]$inputs) {
    $copy = Join-Path $Work 'inputs-bad'
    Copy-Item -LiteralPath $inputs -Destination $copy -Recurse
    [System.IO.File]::WriteAllText((Join-Path (Join-Path $copy 'txt-crlf') 'crlf.txt'), 'changed')
    Remove-Item -LiteralPath (Join-Path (Join-Path $copy 'txt-html') 'expected-html.txt')
    foreach ($name in 'txt-crlf', 'txt-html', 'no-such-case') {
        $options = New-FakeOptions $name $copy $false ('run-bad-' + $name)
        $status = Invoke-Harness $options
        $observed = (Read-Result $options).cases[0].observed
        $wanted = 'no-input'
        if ($name -eq 'no-such-case') { $wanted = 'bad-case-file' }
        Assert-Equal ('bad input: ' + $name) ($observed + ' ' + $status) ($wanted + ' 1')
    }
}

function Test-RefusesBusyMachine([string]$inputs) {
    $recover = Join-Path (Get-ConfigDir) 'recover'
    New-Item -ItemType Directory -Force -Path (Join-Path $recover '1000-1') | Out-Null
    $message = ''
    try { [void](Invoke-Harness (New-FakeOptions 'txt-crlf' $inputs $false 'run-busy')) } catch { $message = $_.Exception.Message }
    Remove-Item -LiteralPath $recover -Recurse -Force
    Assert-Equal 'refuses to start with recovery entries' $message.StartsWith('stopped: ') $true
}

# --- backup.ps1 and restore.ps1, as scripts, on a config folder of the test --------

function Invoke-Script([string]$name, [string]$argument) {
    $shell = [System.Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
    $output = & $shell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot $name) $argument
    return ($LASTEXITCODE.ToString() + ' ' + (@($output) -join ' | '))
}

function Test-BackupRestore {
    $config = Get-ConfigDir
    $backup = Join-Path $Work 'backup'
    New-Item -ItemType Directory -Force -Path (Join-Path (Join-Path $config 'recover') '1000-1') | Out-Null
    [System.IO.File]::WriteAllText((Join-Path (Join-Path (Join-Path $config 'recover') '1000-1') 'Editor.bin'), 'DOCY')
    [System.IO.File]::WriteAllText((Join-Path $config 'recent-files.json'), '[]')
    Assert-Equal 'backup' (Invoke-Script 'backup.ps1' $backup).StartsWith('0 saved 4 paths') $true
    Assert-Equal 'entries saved' ((Read-Lines (Join-Path $backup 'entries.txt')) -join ' ') 'recover recent-files.json'
    Remove-Item -LiteralPath (Join-Path $config 'recover') -Recurse -Force
    [System.IO.File]::WriteAllText((Join-Path $config 'recent-files.json'), '["changed"]')
    [System.IO.File]::WriteAllText((Join-Path $config '.window-state.json'), '{}')
    Assert-Equal 'restore' (Invoke-Script 'restore.ps1' $backup).StartsWith('0 restored 4 paths') $true
    Assert-Equal 'restored file' ([System.IO.File]::ReadAllText((Join-Path $config 'recent-files.json'))) '[]'
    Assert-Equal 'restored folder' (Test-Path -LiteralPath (Join-Path (Join-Path (Join-Path $config 'recover') '1000-1') 'Editor.bin')) $true
    Assert-Equal 'an entry that was not there is removed' (Test-Path -LiteralPath (Join-Path $config '.window-state.json')) $false
    Add-Content -LiteralPath (Join-Path (Join-Path (Join-Path $backup 'state') 'recover') '1000-1\Editor.bin'.Replace('\', [System.IO.Path]::DirectorySeparatorChar)) -Value 'x'
    Assert-Equal 'restore tells a damaged backup' (Invoke-Script 'restore.ps1' $backup).StartsWith('1 ') $true
    Assert-Equal 'restore without a backup' (Invoke-Script 'restore.ps1' (Join-Path $Work 'none')).StartsWith('2 ') $true
    Remove-Item -LiteralPath (Join-Path $config 'recover') -Recurse -Force
}

function Get-TestInputs([string]$inputs) {
    if ($inputs) { return (Resolve-FullPath $inputs) }
    $python = Find-Python
    if (-not $python) { throw 'no Python: give a folder made by gen/mkinputs.py with -Inputs' }
    $made = Join-Path $Work 'inputs'
    & $python -I (Join-Path (Join-Path $HarnessDir 'gen') 'mkinputs.py') $made | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'gen/mkinputs.py failed' }
    return $made
}

New-Item -ItemType Directory -Force -Path $Work | Out-Null
[System.IO.File]::WriteAllText((Join-Path $Work 'fake-app.exe'), 'stand-in')
# The config folder of the test, so the user's own state is never read or changed.
$env:APPDATA = Join-Path $Work 'appdata'
New-Item -ItemType Directory -Force -Path (Get-ConfigDir) | Out-Null
$TestInputs = Get-TestInputs $GivenInputs

Test-Scripts
Test-CaseFiles
Test-Keys
Test-ByteChecks
Test-ZipChecks $TestInputs
Test-Json
Test-OpenSignals
Test-FullRun $TestInputs
Test-FailingRun $TestInputs
Test-ControlRun $TestInputs
Test-BadInputs $TestInputs
Test-RefusesBusyMachine $TestInputs
Test-BackupRestore

Remove-Item -LiteralPath $Work -Recurse -Force
if ($Failures.Count -gt 0) {
    Write-Output ($Failures.Count.ToString() + ' checks failed')
    exit 1
}
Write-Output 'every check passed'
exit 0
