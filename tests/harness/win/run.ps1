# run.ps1 CASE|all [-Installer FILE] [-Inputs DIR] [-Control] [-Screenshots] [-Out DIR]
#
# Runs the harness cases of ..\cases against the installed app on the desktop
# of this session and writes OUT\result.json, with the schema run.sh writes.
# -Installer runs that NSIS installer silently first. -Inputs takes the input
# files from a folder made by gen/mkinputs.py instead of running Python here.
# -Control judges each case by its control_expect (what a build without the fix does).
# Exit status: 0 every case ok, 1 some case not ok, 2 the harness could not run.
param(
    [Parameter(Position = 0)][string]$Selection,
    [string]$Installer,
    [string]$Inputs,
    [switch]$Control,
    [switch]$Screenshots,
    [string]$Out
)

$ErrorActionPreference = 'Stop'
$HarnessDir = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib.ps1')

$Usage = 'usage: run.ps1 CASE|all [-Installer FILE] [-Inputs DIR] [-Control] [-Screenshots] [-Out DIR]'

function Get-DefaultOut {
    $work = $env:HARNESS_WORK
    if (-not $work) { $work = Join-Path $HOME 'eol-harness-work' }
    return (Join-Path $work ('run-' + (Get-Date).ToString('yyyyMMdd-HHmmss')))
}

# Seconds a case waits for Ctrl+S to change the file: HARNESS_SAVE_WAIT, or 30.
function Get-SaveWait {
    if ($env:HARNESS_SAVE_WAIT) { return [int]$env:HARNESS_SAVE_WAIT }
    return 30
}

function New-Options([string]$selection, [string]$installer, [string]$inputs, [bool]$control, [bool]$screenshots, [string]$out) {
    if (-not $out) { $out = Get-DefaultOut }
    if ($inputs) { $inputs = Resolve-FullPath $inputs }
    return @{
        Selection = $selection
        Installer = $installer
        Inputs = $inputs
        Control = $control
        Screenshots = $screenshots
        Out = Resolve-FullPath $out
        SaveWait = Get-SaveWait
        Python = ''
        Exe = ''
        Results = New-Object System.Collections.ArrayList
    }
}

function Get-SelectedCases([string]$selection) {
    if ($selection -ne 'all') { return @($selection) }
    $names = @(Get-ChildItem -LiteralPath (Join-Path $HarnessDir 'cases') -Filter '*.toml' | ForEach-Object { $_.BaseName })
    [Array]::Sort($names, [StringComparer]::Ordinal)
    return $names
}

# --- case files: one key = "value" (or 'value') per line, comments with # ---------

function Read-CaseFile([string]$path) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Write-Log ('no case file ' + $path)
        return $null
    }
    $case = @{}
    foreach ($line in [System.IO.File]::ReadAllLines($path)) {
        if ([regex]::IsMatch($line, '^\s*(#.*)?$')) { continue }
        $found = [regex]::Match($line, '^([a-z0-9_]+)\s*=\s*"(.*)"\s*$')
        if (-not $found.Success) { $found = [regex]::Match($line, "^([a-z0-9_]+)\s*=\s*'(.*)'\s*$") }
        if (-not $found.Success) {
            Write-Log ($path + ': cannot read line: ' + $line)
            return $null
        }
        $case[$found.Groups[1].Value] = $found.Groups[2].Value
    }
    return $case
}

function Get-CaseValue($case, [string]$key) {
    if ($case.ContainsKey($key)) { return [string]$case[$key] }
    return ''
}

# The match_N values in the order of N.
function Get-CaseMatches($case) {
    $keys = @($case.Keys | Where-Object { $_ -like 'match_*' })
    $ordered = @($keys | Sort-Object { [regex]::Replace($_, '\d+', { param($number) $number.Value.PadLeft(10, '0') }) })
    return @($ordered | ForEach-Object { $case[$_] })
}

function Get-ExpectedOutcome($case, [bool]$control) {
    $controlExpect = Get-CaseValue $case 'control_expect'
    if ($control -and $controlExpect) { return $controlExpect }
    return (Get-CaseValue $case 'expect')
}

# A key name Windows has no code for must fail before the app starts.
function Test-CaseKeys($case) {
    try {
        [void](ConvertTo-KeyChords (Get-CaseValue $case 'keys_before'))
        [void](ConvertTo-KeyChords (Get-CaseValue $case 'keys_after'))
        return $true
    } catch {
        Write-Log $_.Exception.Message
        return $false
    }
}

# --- preparing a run -----------------------------------------------------------

function Test-MachineFree {
    $running = @(Get-AppProcessIds)
    if ($running.Count -gt 0) {
        Stop-Harness ('the app is already running (pids ' + ($running -join ' ') + '): another test may be in progress')
    }
    if (@(Get-RecoverEntries).Count -gt 0) {
        Stop-Harness ((Join-Path (Get-ConfigDir) 'recover') + ' is not empty: save it with backup.ps1 and empty it first')
    }
}

# Runs the NSIS installer silently (/S) and checks what it left installed.
function Install-App([string]$installer) {
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { Stop-Harness ('no installer at ' + $installer) }
    $path = (Get-Item -LiteralPath $installer).FullName
    Write-Log ('installing ' + (Split-Path -Leaf $path))
    $process = Start-Process -FilePath $path -ArgumentList '/S' -PassThru
    # Reading the handle now keeps the exit code readable after the process ends.
    $null = $process.Handle
    if (-not $process.WaitForExit(600000)) {
        Stop-ProcessTree $process.Id
        Stop-Harness 'the installer did not finish in 10 minutes'
    }
    if ($process.ExitCode -ne 0) { Stop-Harness ('the installer ended with exit code ' + $process.ExitCode) }
    $exe = Get-AppExe
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { Stop-Harness ('the installer ended well but there is no ' + $exe) }
    $want = (Get-FileProductVersion $path).Trim()
    $have = (Get-InstalledVersion).Trim()
    if ($want -and $have -and ($want -ne $have)) {
        Stop-Harness ('the installed version is ' + $have + ', the installer is ' + $want)
    }
    Write-Log ('installed version ' + $have + ', exe sha256 ' + (Get-InstalledSha256))
}

# Without the app every case would end launch-failed.
function Test-AppInstalled([string]$exe) {
    if (Test-Path -LiteralPath $exe -PathType Leaf) { return }
    Stop-Harness ($exe + ' is not installed: give an installer with -Installer')
}

# A Python that runs: the python of the Microsoft Store is a name without a program.
function Find-Python {
    foreach ($name in 'python', 'python3', 'py') {
        $command = Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $command) { continue }
        try {
            & $command.Source -c 'import sys' | Out-Null
            if ($LASTEXITCODE -eq 0) { return $command.Source }
        } catch {
            continue
        }
    }
    return ''
}

# Where the inputs come from: the folder -Inputs gives, or the generators.
function Set-InputSource($options) {
    if ($options.Inputs) {
        if (Test-Path -LiteralPath $options.Inputs -PathType Container) { return }
        Stop-Harness ('no folder ' + $options.Inputs)
    }
    $options.Python = Find-Python
    if (-not $options.Python) { Stop-Harness 'no Python to generate the inputs: make them with gen/mkinputs.py elsewhere and give the folder with -Inputs' }
}

# --- one case --------------------------------------------------------------------

function New-Run([string]$name) {
    return @{
        Name = $name
        Case = @{}
        Expected = ''
        Observed = ''
        Detail = ''
        Dir = ''
        Source = ''
        Doc = ''
        ShaBefore = ''
        ProcessId = 0
        Log = ''
        # Lines the log had when Ctrl+S was sent.
        LogLinesBefore = 0
        DialogsAnswered = 0
    }
}

function Add-Note($run, [string]$text) {
    if ($run.Detail) { $run.Detail += '; ' }
    $run.Detail += $text
}

function Test-FileSha($run, [string]$path, [string]$want) {
    if (-not $want) { return $true }
    if ((Get-Sha256 $path) -eq $want) { return $true }
    Add-Note $run ((Split-Path -Leaf $path) + ' does not have the sha256 the case gives')
    return $false
}

# Runs the generator SPEC names ("SCRIPT ARGS") into TARGET.
function Invoke-Generator($options, $run, [string]$spec, [string]$target) {
    $words = @($spec -split '\s+' | Where-Object { $_ })
    $script = Join-Path (Join-Path $HarnessDir 'gen') $words[0]
    if (-not (Test-Path -LiteralPath $script -PathType Leaf)) {
        Add-Note $run ('no generator gen/' + $words[0])
        return $false
    }
    $arguments = @($words | Select-Object -Skip 1)
    & $options.Python -I $script $target @arguments | Out-Host
    if ($LASTEXITCODE -eq 0) { return $true }
    Add-Note $run ('generator failed: ' + $spec)
    return $false
}

# Takes TARGET from INPUTS\CASE\, where gen/mkinputs.py writes it.
function Copy-PreparedFile($options, $run, [string]$target) {
    $prepared = Join-Path (Join-Path $options.Inputs $run.Name) (Split-Path -Leaf $target)
    if (-not (Test-Path -LiteralPath $prepared -PathType Leaf)) {
        Add-Note $run ('no ' + $prepared)
        return $false
    }
    Copy-Item -LiteralPath $prepared -Destination $target -Force
    return $true
}

# Makes TARGET, generated or taken from -Inputs, and checks the sha256 the case gives.
function New-CaseFile($options, $run, [string]$spec, [string]$target, [string]$want) {
    if ($options.Inputs) {
        $made = Copy-PreparedFile $options $run $target
    } else {
        $made = Invoke-Generator $options $run $spec $target
    }
    if (-not $made) { return $false }
    return (Test-FileSha $run $target $want)
}

# The file a save must write, for the expected-bytes check.
function Get-ExpectedFile([string]$source) {
    return (Join-Path (Split-Path -Parent $source) ('expected-' + (Split-Path -Leaf $source)))
}

function New-CaseInputs($options, $run) {
    $case = $run.Case
    $made = New-CaseFile $options $run (Get-CaseValue $case 'generate') $run.Source (Get-CaseValue $case 'input_sha256')
    if (-not $made) { return $false }
    $expectedSpec = Get-CaseValue $case 'expected_generate'
    if (-not $expectedSpec) { return $true }
    return (New-CaseFile $options $run $expectedSpec (Get-ExpectedFile $run.Source) (Get-CaseValue $case 'expected_sha256'))
}

# What the app has said about the open so far: failed, ready or nothing.
# A failed open shows as [OPEN] failed in the log, as a native dialog, or, when
# the editor reports it inside the web view, only as errorCode= in the log.
# A .md opens in the Markdown page, which says neither: the backend's
# [OPEN] markdown lines stand for them.
function Get-OpenState($run) {
    if (-not $run.Log) { $run.Log = Find-InstanceLog $run.ProcessId }
    $text = Read-SharedText $run.Log
    if ($text.Contains('[OPEN] failed')) { return 'failed' }
    if ($text.Contains('[OPEN] markdown failed:')) { return 'failed' }
    if ($text.Contains('errorCode=')) { return 'failed' }
    if (Test-Dialog $run.ProcessId) { return 'failed' }
    if ($text.Contains('Document ready')) { return 'ready' }
    if ($text.Contains('[OPEN] markdown success')) { return 'ready' }
    return ''
}

function Format-Seconds([double]$seconds) {
    return $seconds.ToString('0.0', [System.Globalization.CultureInfo]::InvariantCulture)
}

# Polls every 100 ms and returns failed, ready or timeout. The log lines carry
# no time of their own, so the stopwatch gives the time to the answer.
function Wait-Open($run, [int]$limitSeconds) {
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    while ($watch.Elapsed.TotalSeconds -lt $limitSeconds) {
        $state = Get-OpenState $run
        if ($state) {
            Add-Note $run ($state + ' after ' + (Format-Seconds $watch.Elapsed.TotalSeconds) + ' s')
            return $state
        }
        if (-not (Test-ProcessAlive $run.ProcessId)) {
            Add-Note $run 'the app ended before the document was ready'
            return 'timeout'
        }
        Start-Sleep -Milliseconds 100
    }
    return 'timeout'
}

function Observe-Rejection($run) {
    Add-Note $run ('log: ' + (Get-LogLine $run.Log '\[OPEN\] (markdown )?failed|errorCode='))
    $run.Observed = 'rejected-without-dialog'
    $dialogShown = Wait-Until 10 { Test-Dialog $run.ProcessId }
    if ($dialogShown) {
        $dialog = @(Get-Dialogs $run.ProcessId)[0]
        $run.Observed = 'rejected-with-dialog'
        Add-Note $run ("dialog: '" + $dialog.Title + "'")
    }
    if ((Get-Sha256 $run.Doc) -eq $run.ShaBefore) { return }
    $run.Observed = 'rejected-but-file-changed'
}

# Presses the button the case names, if it names one, with its keys (Get-ButtonKeys).
function Press-CaseButton($run) {
    $label = Get-CaseValue $run.Case 'button'
    if (-not $label) { return }
    $keys = Get-ButtonKeys $label
    if (-not $keys) {
        Add-Note $run ('button ' + $label + ': not pressed')
        return
    }
    Send-Keys $keys
}

# Click in the middle of the window, the button of the case, keys_before,
# type, keys_after. False when the window could not be brought to the
# foreground: nothing was typed.
function Edit-Document($run) {
    $window = Get-MainWindow $run.ProcessId
    if (-not (Set-Foreground $window)) { return $false }
    Start-Sleep -Milliseconds 500
    Click-WindowCenter $window
    Start-Sleep -Milliseconds 500
    Press-CaseButton $run
    Send-Keys (Get-CaseValue $run.Case 'keys_before')
    Send-Text (Get-CaseValue $run.Case 'type')
    Send-Keys (Get-CaseValue $run.Case 'keys_after')
    Start-Sleep -Seconds 1
    return $true
}

# A dialog after Ctrl+S: a question is answered Yes by control id, anything
# else gets Return on its default button (Save, in a Save As dialog). No label
# is read, so the language of the system does not matter.
function Answer-SaveDialogs($run) {
    foreach ($dialog in @(Get-Dialogs $run.ProcessId)) {
        if ($run.DialogsAnswered -ge 4) { return }
        $run.DialogsAnswered++
        Add-Note $run ("dialog after Ctrl+S: '" + $dialog.Title + "'")
        if (Click-YesButton $dialog.Handle) { continue }
        if (Set-Foreground $dialog.Handle) { Send-Keys 'Return' }
    }
}

function Get-LogLineCount([string]$path) {
    return [regex]::Matches((Read-SharedText $path), "`n").Count
}

# The [SAVE] markdown lines the log got after its first $count lines. The
# Markdown page saves through the backend, which logs both.
function Get-NewSaveLines([string]$path, [int]$count) {
    $lines = @((Read-SharedText $path) -split '\r?\n' | Select-Object -Skip $count)
    return @($lines | Where-Object { $_ -match '\[SAVE\] markdown (success|failed)' })
}

function Test-SaveWritten($run) {
    if ((Get-Sha256 $run.Doc) -ne $run.ShaBefore) { return $true }
    if (@(Get-NewSaveLines $run.Log $run.LogLinesBefore).Count -gt 0) { return $true }
    Answer-SaveDialogs $run
    return $false
}

# Let the writer finish: the hash must hold for two seconds.
function Wait-StableFile([string]$path) {
    $last = 'unread'
    while ($last -ne (Get-Sha256 $path)) {
        $last = Get-Sha256 $path
        Start-Sleep -Seconds 2
    }
}

# Ctrl+S, then waits for the file to change or for a logged Markdown save.
# Returns saved, save-not-written, save-failed, saved-unchanged or
# not-in-foreground.
function Save-Document($options, $run) {
    if (-not (Set-Foreground (Get-MainWindow $run.ProcessId))) { return 'not-in-foreground' }
    $run.LogLinesBefore = Get-LogLineCount $run.Log
    Send-Keys 'ctrl+s'
    $written = Wait-Until $options.SaveWait { Test-SaveWritten $run }
    if (-not $written) { return 'save-not-written' }
    $logged = @(Get-NewSaveLines $run.Log $run.LogLinesBefore)
    $failed = @($logged | Where-Object { $_ -match 'markdown failed' })
    if ($failed.Count -gt 0) {
        Add-Note $run ('log: ' + $failed[0])
        return 'save-failed'
    }
    if ((Get-Sha256 $run.Doc) -eq $run.ShaBefore) {
        Add-Note $run ('log: ' + $logged[0])
        return 'saved-unchanged'
    }
    Wait-StableFile $run.Doc
    return 'saved'
}

# Bytes of a text file that are not its BOM, line breaks or NUL.
function Get-TextPayloadCount([string]$path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    $skip = 0
    if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) { $skip = 2 }
    if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFE -and $bytes[1] -eq 0xFF) { $skip = 2 }
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) { $skip = 3 }
    $count = 0
    for ($index = $skip; $index -lt $bytes.Length; $index++) {
        if (0, 10, 13 -notcontains $bytes[$index]) { $count++ }
    }
    return $count
}

function Get-FileLength([string]$path) {
    return (Get-Item -LiteralPath $path).Length
}

function Test-SameBytes([string]$first, [string]$second) {
    return ((Get-Sha256 $first) -eq (Get-Sha256 $second))
}

# Where two files first differ, counting bytes from 1 like cmp.
function Get-FirstDifference([string]$saved, [string]$expected) {
    $left = [System.IO.File]::ReadAllBytes($saved)
    $right = [System.IO.File]::ReadAllBytes($expected)
    $common = [Math]::Min($left.Length, $right.Length)
    for ($index = 0; $index -lt $common; $index++) {
        if ($left[$index] -eq $right[$index]) { continue }
        return ('byte {0}: saved 0x{1:x2}, expected 0x{2:x2}' -f ($index + 1), $left[$index], $right[$index])
    }
    return ('byte {0}: one file ends before it' -f ($common + 1))
}

function Check-SameText($run) {
    Add-Note $run ('saved {0} bytes, original {1} bytes' -f (Get-FileLength $run.Doc), (Get-FileLength $run.Source))
    if (Test-SameBytes $run.Doc $run.Source) {
        $run.Observed = 'opens-whole'
    } elseif ((Get-TextPayloadCount $run.Doc) -eq 0) {
        $run.Observed = 'opens-empty'
    } else {
        $run.Observed = 'saved-changed'
    }
}

function Check-ExpectedBytes($run) {
    $expected = Get-ExpectedFile $run.Source
    Add-Note $run ('saved {0} bytes, expected {1} bytes' -f (Get-FileLength $run.Doc), (Get-FileLength $expected))
    if (Test-SameBytes $run.Doc $expected) {
        $run.Observed = 'saved-as-expected'
        return
    }
    $run.Observed = 'saved-changed'
    Add-Note $run ('first difference: ' + (Get-FirstDifference $run.Doc $expected))
}

# The case patterns are POSIX extended regexes, for grep -E. Their bracket
# classes are the one thing the cases use that .NET writes differently.
$PosixClasses = @{
    '[:space:]' = '\s'
    '[:digit:]' = '0-9'
    '[:alpha:]' = 'A-Za-z'
    '[:alnum:]' = 'A-Za-z0-9'
    '[:upper:]' = 'A-Z'
    '[:lower:]' = 'a-z'
    '[:xdigit:]' = '0-9A-Fa-f'
}

function ConvertTo-DotNetRegex([string]$pattern) {
    foreach ($class in $PosixClasses.Keys) {
        $pattern = $pattern.Replace($class, $PosixClasses[$class])
    }
    return $pattern
}

# The text of a part of a zip, or nothing when the zip has no such part.
function Read-ZipPart([string]$zipPath, [string]$part) {
    $zip = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        $entry = $zip.GetEntry($part)
        if (-not $entry) { return $null }
        $reader = New-Object System.IO.StreamReader($entry.Open(), [System.Text.Encoding]::UTF8)
        try {
            return $reader.ReadToEnd()
        } finally {
            $reader.Dispose()
        }
    } finally {
        $zip.Dispose()
    }
}

# Like unzip -p ZIP PART | grep -E PATTERN: some line of the part matches.
function Test-ZipPartMatch([string]$zipPath, [string]$part, [string]$pattern) {
    try {
        $text = Read-ZipPart $zipPath $part
    } catch [System.IO.InvalidDataException] {
        return $false
    }
    if ($null -eq $text) { return $false }
    $regex = New-Object System.Text.RegularExpressions.Regex (ConvertTo-DotNetRegex $pattern)
    foreach ($line in $text.Split([char]10)) {
        if ($regex.IsMatch($line)) { return $true }
    }
    return $false
}

function Check-ZipMatch($run) {
    $missing = @()
    foreach ($match in (Get-CaseMatches $run.Case)) {
        $part, $pattern = $match -split ' ', 2
        if (Test-ZipPartMatch $run.Doc $part $pattern) { continue }
        $missing += ($part + ': ' + $pattern)
    }
    if ($missing.Count -eq 0) {
        $run.Observed = 'saved-as-expected'
        return
    }
    $run.Observed = 'saved-missing-content'
    Add-Note $run ('not found: ' + ($missing -join ', '))
}

# --- exporting to PDF ------------------------------------------------------------

# The first export of a fresh install builds the font cache before it writes.
$ExportWaitSeconds = 120

# The PDF an open-export-pdf case writes, next to the document it opened.
function Get-ExportFile([string]$doc) {
    return (Join-Path (Split-Path -Parent $doc) 'exported.pdf')
}

function Test-HasContent([string]$path) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $false }
    return ((Get-FileLength $path) -gt 0)
}

function Test-ExportEnded($run, [string]$pdf) {
    if (Test-HasContent $pdf) { return $true }
    return ((Get-LogLine $run.Log 'SaveAs failed') -ne '')
}

# Ctrl+Shift+S opens the native Save As dialog, and the app takes the format
# from the extension of the name typed there. Returns exported, or the outcome
# of a case whose PDF was not written.
function Save-AsPdf($run, [string]$pdf) {
    if (Test-Path -LiteralPath $pdf) { Remove-Item -LiteralPath $pdf -Force }
    if (-not (Set-Foreground (Get-MainWindow $run.ProcessId))) { return 'not-in-foreground' }
    Send-Keys 'ctrl+shift+s'
    $dialogShown = Wait-Until 20 { Test-Dialog $run.ProcessId }
    if (-not $dialogShown) { return 'export-no-dialog' }
    $dialog = @(Get-Dialogs $run.ProcessId)[0]
    Add-Note $run ("dialog: '" + $dialog.Title + "'")
    if (-not (Set-Foreground $dialog.Handle)) { return 'not-in-foreground' }
    Start-Sleep -Seconds 1
    # The dialog offers the name of the document, selected or not: replace it.
    Send-Keys 'ctrl+a'
    Send-Text $pdf
    Send-Keys 'Return'
    [void](Wait-Until $ExportWaitSeconds { Test-ExportEnded $run $pdf })
    if (-not (Test-HasContent $pdf)) {
        Add-Note $run ('log: ' + (Get-LogLine $run.Log 'SaveAs failed'))
        return 'export-not-written'
    }
    Wait-StableFile $pdf
    return 'exported'
}

# The lines of the text of a PDF. The text stays next to the PDF, for a case
# that fails.
function Read-PdfText([string]$tool, [string]$pdf) {
    $textFile = [System.IO.Path]::ChangeExtension($pdf, '.txt')
    & $tool -q -enc UTF-8 $pdf $textFile | Out-Null
    if (-not (Test-Path -LiteralPath $textFile -PathType Leaf)) { return @() }
    return @([System.IO.File]::ReadAllLines($textFile, [System.Text.Encoding]::UTF8))
}

# Like grep -E PATTERN: some line matches.
function Test-LineMatch($lines, [string]$pattern) {
    $regex = New-Object System.Text.RegularExpressions.Regex (ConvertTo-DotNetRegex $pattern)
    foreach ($line in $lines) {
        if ($regex.IsMatch($line)) { return $true }
    }
    return $false
}

# Every match_N of the case is a regex some line of the PDF's text must
# contain. Without pdftotext the case fails: the header alone says nothing
# about the text.
function Check-PdfText($run, [string]$pdf) {
    Add-Note $run ('exported {0} bytes' -f (Get-FileLength $pdf))
    if (-not (Test-PdfHeader $pdf)) {
        $run.Observed = 'exported-not-pdf'
        return
    }
    $tool = Find-PdfToText
    if (-not $tool) {
        $run.Observed = 'no-pdftotext'
        return
    }
    $lines = Read-PdfText $tool $pdf
    $missing = @(Get-CaseMatches $run.Case | Where-Object { -not (Test-LineMatch $lines $_) })
    if ($missing.Count -eq 0) {
        $run.Observed = 'exported-as-expected'
        return
    }
    $run.Observed = 'exported-missing-content'
    Add-Note $run ('not found: ' + ($missing -join ', '))
}

function Export-Pdf($run) {
    $pdf = Get-ExportFile $run.Doc
    $exported = Save-AsPdf $run $pdf
    if ($exported -ne 'exported') {
        $run.Observed = $exported
        return
    }
    if ((Get-CaseValue $run.Case 'check') -eq 'pdf-text') {
        Check-PdfText $run $pdf
        return
    }
    $run.Observed = 'exported'
    Add-Note $run 'no check for this case'
}

# --- recovery after a kill ---------------------------------------------------------

$RecoveryWaitSeconds = 30

# The id of the recovery session of a document: its folder under recover\.
$SessionPattern = '\[RECOVER\] begin session=(\d+-\d+)'
# The number of sessions the start screen got to offer.
$CandidatesPattern = '\[RECOVER\] candidates=(\d+)'

# The file the app appends every change of a recovery session to.
function Get-RecoveryChanges([string]$session) {
    $folder = Join-Path (Join-Path (Get-ConfigDir) 'recover') $session
    return (Join-Path (Join-Path $folder 'changes') 'changes0.json')
}

# Starts the app with no document and returns the number of sessions its start
# screen got to offer, or nothing when the log never says. The log of this
# second instance is kept as app-restart.log.
function Get-OfferedAfterRestart($options, $run) {
    $run.ProcessId = Start-AppEmpty $options.Exe $run.Dir
    $hasWindow = Wait-Until 60 { Test-MainWindow $run.ProcessId }
    if (-not $hasWindow) { return '' }
    [void](Wait-Until 60 { (Get-LogValue (Find-InstanceLog $run.ProcessId) $CandidatesPattern) -ne '' })
    $restartLog = Find-InstanceLog $run.ProcessId
    if ($restartLog) { Copy-Item -LiteralPath $restartLog -Destination (Join-Path $run.Dir 'app-restart.log') -Force }
    return (Get-LogValue $restartLog $CandidatesPattern)
}

# Edits the document, waits for the app to record the changes, kills it and
# starts it again with no document. The case does not press Recover.
function Recover-AfterKill($options, $run) {
    $began = Wait-Until 10 { (Get-LogValue $run.Log $SessionPattern) -ne '' }
    if (-not $began) {
        $run.Observed = 'recovery-not-started'
        return
    }
    $session = Get-LogValue $run.Log $SessionPattern
    $changes = Get-RecoveryChanges $session
    if (-not (Edit-Document $run)) {
        $run.Observed = 'not-in-foreground'
        return
    }
    $recorded = Wait-Until $RecoveryWaitSeconds { Test-HasContent $changes }
    if (-not $recorded) {
        $run.Observed = 'recovery-not-written'
        return
    }
    Wait-StableFile $changes
    Add-Note $run ('session {0}: {1} bytes of changes' -f $session, (Get-FileLength $changes))
    Stop-AppAbruptly $run.ProcessId
    [void](Wait-Until 10 { -not (Test-ProcessAlive $run.ProcessId) })
    $offered = Get-OfferedAfterRestart $options $run
    $run.Observed = 'recovery-not-offered'
    if (-not $offered) { return }
    Add-Note $run ('log: [RECOVER] candidates=' + $offered)
    if ([int]$offered -ge 1) { $run.Observed = 'recovery-offered' }
}

function Observe-OpenDocument($options, $run) {
    $action = Get-CaseValue $run.Case 'action'
    Add-Note $run ('log: ' + (Get-LogLine $run.Log '\[OPEN\] (markdown )?success'))
    # The editor keeps loading for a moment after it says it is ready.
    Start-Sleep -Seconds 3
    if ($action -eq 'open') {
        $run.Observed = 'opens'
        return
    }
    if ($action -eq 'open-export-pdf') {
        Export-Pdf $run
        return
    }
    if ($action -eq 'edit-kill-recover') {
        Recover-AfterKill $options $run
        return
    }
    if ($action -eq 'open-edit-save' -and -not (Edit-Document $run)) {
        $run.Observed = 'not-in-foreground'
        return
    }
    $saved = Save-Document $options $run
    if ($saved -ne 'saved') {
        $run.Observed = $saved
        return
    }
    $check = Get-CaseValue $run.Case 'check'
    if ($check -eq 'same-text') { Check-SameText $run; return }
    if ($check -eq 'zip-match') { Check-ZipMatch $run; return }
    if ($check -eq 'expected-bytes') { Check-ExpectedBytes $run; return }
    $run.Observed = 'saved'
    Add-Note $run 'no check for this case'
}

# Starts the app on the document and sets the outcome of the case.
function Observe-Case($options, $run) {
    try {
        $run.ProcessId = Start-App $options.Exe $run.Doc
    } catch {
        $run.Observed = 'launch-failed'
        Add-Note $run $_.Exception.Message
        return
    }
    $hasWindow = Wait-Until 60 { Test-MainWindow $run.ProcessId }
    if (-not $hasWindow) {
        $run.Observed = 'launch-failed'
        return
    }
    $state = Wait-Open $run 120
    if ($state -eq 'failed') { Observe-Rejection $run; return }
    if ($state -eq 'ready') { Observe-OpenDocument $options $run; return }
    $run.Observed = 'open-timeout'
}

# Takes the screenshot of a case that needs one and returns its path relative
# to the folder of result.json, or nothing.
function Get-CaseScreenshot($options, $run, [bool]$ok) {
    $always = $options.Screenshots -or ((Get-CaseValue $run.Case 'screenshot') -eq 'always')
    if ($ok -and -not $always) { return '' }
    if (Save-Screenshot (Join-Path $run.Dir 'screen.png')) { return ($run.Name + '/screen.png') }
    return ''
}

# Leaves the user's recovery folder as the run found it: empty.
function Clear-NewRecovery($run) {
    $entries = @(Get-RecoverEntries)
    if ($entries.Count -eq 0) { return }
    Add-Note $run ('the case left recovery data, removed: ' + ($entries -join ' '))
    foreach ($entry in $entries) { Remove-Item -LiteralPath $entry -Recurse -Force }
}

# Closes the app of the case by its pid and keeps its log as app.log.
function Complete-Case($run) {
    if ($run.ProcessId) {
        Dismiss-Dialogs $run.ProcessId
        $closed = Close-App $run.ProcessId
        if ($closed -ne 'closed') { Add-Note $run ('app close: ' + $closed) }
    }
    if ($run.Log) { Copy-Item -LiteralPath $run.Log -Destination (Join-Path $run.Dir 'app.log') -Force }
    Clear-NewRecovery $run
}

function Add-CaseResult($options, $run, [bool]$ok, [string]$shot, [int]$seconds) {
    $okJson = 'false'
    if ($ok) { $okJson = 'true' }
    $shotJson = 'null'
    if ($shot) { $shotJson = ConvertTo-JsonString $shot }
    $fields = @(
        ('"case": ' + (ConvertTo-JsonString $run.Name)),
        ('"ok": ' + $okJson),
        ('"expected": ' + (ConvertTo-JsonString $run.Expected)),
        ('"observed": ' + (ConvertTo-JsonString $run.Observed)),
        ('"detail": ' + (ConvertTo-JsonString $run.Detail)),
        ('"screenshot": ' + $shotJson),
        ('"seconds": ' + $seconds)
    )
    [void]$options.Results.Add('{' + ($fields -join ', ') + '}')
    Write-Log ($run.Name + ': ok=' + $okJson + ' observed=' + $run.Observed)
}

function Invoke-Case($options, [string]$name) {
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $run = New-Run $name
    $case = Read-CaseFile (Join-Path (Join-Path $HarnessDir 'cases') ($name + '.toml'))
    if ($null -eq $case -or -not (Test-CaseKeys $case)) {
        $run.Observed = 'bad-case-file'
        Add-CaseResult $options $run $false '' 0
        return
    }
    $run.Case = $case
    $run.Expected = Get-ExpectedOutcome $case $options.Control
    $run.Dir = Join-Path $options.Out $name
    $run.Source = Join-Path (Join-Path $run.Dir 'source') (Get-CaseValue $case 'input')
    $run.Doc = Join-Path $run.Dir (Get-CaseValue $case 'input')
    New-Item -ItemType Directory -Force -Path (Join-Path $run.Dir 'source') | Out-Null
    if (-not (New-CaseInputs $options $run)) {
        $run.Observed = 'no-input'
        Add-CaseResult $options $run $false '' ([Math]::Floor($watch.Elapsed.TotalSeconds))
        return
    }
    Copy-Item -LiteralPath $run.Source -Destination $run.Doc -Force
    $run.ShaBefore = Get-Sha256 $run.Doc
    try {
        Observe-Case $options $run
    } catch {
        $run.Observed = 'harness-error'
        Add-Note $run $_.Exception.Message
    }
    $ok = ($run.Observed -eq $run.Expected)
    $shot = Get-CaseScreenshot $options $run $ok
    try {
        Complete-Case $run
    } catch {
        Add-Note $run ('closing the case: ' + $_.Exception.Message)
    }
    Add-CaseResult $options $run $ok $shot ([Math]::Floor($watch.Elapsed.TotalSeconds))
}

# --- result ---------------------------------------------------------------------

function Write-Result($options) {
    $file = Join-Path $options.Out 'result.json'
    $mode = 'normal'
    if ($options.Control) { $mode = 'control' }
    $date = (Get-Date).ToString('yyyy-MM-ddTHH:mm:sszzz', [System.Globalization.CultureInfo]::InvariantCulture)
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add('{')
    $lines.Add('  "date": ' + (ConvertTo-JsonString $date) + ',')
    $lines.Add('  "mode": ' + (ConvertTo-JsonString $mode) + ',')
    $lines.Add('  "package": ' + (ConvertTo-JsonString (Get-InstalledVersion)) + ',')
    $lines.Add('  "binary_sha256": ' + (ConvertTo-JsonString (Get-InstalledSha256)) + ',')
    $lines.Add('  "cases": [')
    $last = $options.Results.Count - 1
    for ($index = 0; $index -le $last; $index++) {
        $comma = ','
        if ($index -eq $last) { $comma = '' }
        $lines.Add('    ' + $options.Results[$index] + $comma)
    }
    $lines.Add('  ]')
    $lines.Add('}')
    $newline = [string][char]10
    $text = ($lines.ToArray() -join $newline) + $newline
    [System.IO.File]::WriteAllText($file, $text, (New-Object System.Text.UTF8Encoding($false)))
    [Console]::Out.WriteLine($file)
}

function Test-AllOk($options) {
    foreach ($result in $options.Results) {
        if ($result.Contains('"ok": false')) { return $false }
    }
    return $true
}

# Runs the selected cases and returns the exit status.
function Invoke-Harness($options) {
    if (-not $options.Selection) { Stop-Harness $Usage }
    if (-not (Test-Windows)) { Stop-Harness 'run.ps1 drives the Windows app: run it on Windows' }
    New-Item -ItemType Directory -Force -Path $options.Out | Out-Null
    Test-MachineFree
    if ($options.Installer) { Install-App $options.Installer }
    $options.Exe = Get-AppExe
    Test-AppInstalled $options.Exe
    Set-InputSource $options
    Write-PdfToTextInUse
    Initialize-Native
    foreach ($name in (Get-SelectedCases $options.Selection)) {
        # Nothing a case prints may end up in the exit status this function returns.
        Invoke-Case $options $name | Out-Null
    }
    Write-Result $options
    if (Test-AllOk $options) { return 0 }
    return 1
}

# Dot-sourced (selftest.ps1): only the functions are wanted.
if ($MyInvocation.InvocationName -eq '.') { return }

$options = New-Options $Selection $Installer $Inputs $Control.IsPresent $Screenshots.IsPresent $Out
exit (Invoke-Harness $options)
