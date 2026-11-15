# lib.ps1: helpers shared by the Windows harness scripts. Dot-sourced, never run on its own.
# Works on Windows PowerShell 5.1 and PowerShell 7. Kept in ASCII: 5.1 reads a
# script without a BOM in the system code page.

$AppName = 'euro-office-lite'
$AppProduct = 'Euro-Office-Lite'
$AppIdentifier = 'org.euro-office.lite'
# What backup.ps1 saves from the config folder.
$StateEntries = 'recover', 'recent-files.json', '.window-state.json'

Add-Type -AssemblyName System.IO.Compression.FileSystem

function Write-Log([string]$message) {
    [Console]::Error.WriteLine('[harness] ' + $message)
}

function Stop-Harness([string]$message) {
    Write-Log $message
    exit 2
}

function Test-Windows {
    return ($env:OS -eq 'Windows_NT')
}

# Wait-Until SECONDS BLOCK: retries BLOCK every half second.
function Wait-Until([double]$waitSeconds, [scriptblock]$condition) {
    $attempts = [int]($waitSeconds * 2)
    for ($attempt = 0; $attempt -lt $attempts; $attempt++) {
        if (& $condition) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

# A path made absolute against the PowerShell location; it need not exist.
function Resolve-FullPath([string]$path) {
    return $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($path)
}

# Lower-case sha256 of a file; empty when it is missing or locked by a writer.
function Get-Sha256([string]$path) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return '' }
    try {
        return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    } catch {
        return ''
    }
}

# --- where the app keeps things ------------------------------------------------

# The app asks Tauri for app_config_dir (recovery.rs, recent.rs, main.rs); on
# Windows that is the roaming AppData folder plus the bundle identifier.
function Get-ConfigDir {
    return (Join-Path $env:APPDATA $AppIdentifier)
}

# temp_layout.rs: logs and per-instance folders live in %TEMP%\euro-office-lite.
function Get-TempRoot {
    return (Join-Path ([System.IO.Path]::GetTempPath()) $AppName)
}

function Get-RecoverEntries {
    $recover = Join-Path (Get-ConfigDir) 'recover'
    if (-not (Test-Path -LiteralPath $recover)) { return @() }
    return @(Get-ChildItem -LiteralPath $recover -Force | ForEach-Object { $_.FullName })
}

# The uninstall entry the NSIS installer writes, for this user or for the machine.
function Get-InstallKey {
    foreach ($hive in 'HKCU:', 'HKLM:') {
        $path = $hive + '\Software\Microsoft\Windows\CurrentVersion\Uninstall\' + $AppProduct
        $key = Get-ItemProperty -Path $path -ErrorAction SilentlyContinue
        if ($key) { return $key }
    }
    return $null
}

function Get-KeyValue($key, [string]$name) {
    if (-not $key) { return '' }
    $property = $key.PSObject.Properties[$name]
    if (-not $property) { return '' }
    return [string]$property.Value
}

# The installed exe: HARNESS_APP_EXE, else the folder of the uninstall entry,
# else the default folder of a per-user install.
function Get-AppExe {
    if ($env:HARNESS_APP_EXE) { return $env:HARNESS_APP_EXE }
    $folder = (Get-KeyValue (Get-InstallKey) 'InstallLocation').Trim('"')
    if (-not $folder) { $folder = Join-Path $env:LOCALAPPDATA $AppProduct }
    return (Join-Path $folder ($AppName + '.exe'))
}

function Get-FileProductVersion([string]$path) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return '' }
    return [string](Get-Item -LiteralPath $path).VersionInfo.ProductVersion
}

function Get-InstalledVersion {
    $version = Get-KeyValue (Get-InstallKey) 'DisplayVersion'
    if ($version) { return $version }
    return (Get-FileProductVersion (Get-AppExe))
}

function Get-InstalledSha256 {
    return (Get-Sha256 (Get-AppExe))
}

function Get-AppProcessIds {
    return @(Get-Process -Name $AppName -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
}

function Test-ProcessAlive([int]$processId) {
    return ($null -ne (Get-Process -Id $processId -ErrorAction SilentlyContinue))
}

# --- snapshots for backup.ps1 and restore.ps1 ------------------------------------

function Get-ExistingEntries([string]$root) {
    return @($StateEntries | Where-Object { Test-Path -LiteralPath (Join-Path $root $_) })
}

function Get-EntryItems([string]$path) {
    $top = Get-Item -LiteralPath $path -Force
    if (-not $top.PSIsContainer) { return @($top) }
    return @($top) + @(Get-ChildItem -LiteralPath $path -Recurse -Force)
}

# Every path under the entries of ROOT, with a sha256 per file, in a fixed order.
function Get-Snapshot([string]$root, [string[]]$entries) {
    $rootPath = (Get-Item -LiteralPath $root -Force).FullName
    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($entry in $entries) {
        foreach ($item in (Get-EntryItems (Join-Path $rootPath $entry))) {
            $relative = $item.FullName.Substring($rootPath.Length).TrimStart('\', '/').Replace('\', '/')
            if ($item.PSIsContainer) {
                $lines.Add('dir ' + $relative)
            } else {
                $lines.Add('file ' + (Get-Sha256 $item.FullName) + ' ' + $relative)
            }
        }
    }
    $sorted = $lines.ToArray()
    [Array]::Sort($sorted, [StringComparer]::Ordinal)
    return $sorted
}

function Write-Lines([string]$path, [string[]]$lines) {
    [System.IO.File]::WriteAllLines($path, [string[]]@($lines | Where-Object { $_ }))
}

function Read-Lines([string]$path) {
    return @([System.IO.File]::ReadAllLines($path) | Where-Object { $_ })
}

# --- JSON ----------------------------------------------------------------------

function ConvertTo-JsonString([string]$text) {
    $clean = [regex]::Replace($text, '[\x00-\x08\x0B\x0C\x0E-\x1F]', '')
    $clean = $clean.Replace('\', '\\').Replace('"', '\"')
    $clean = $clean.Replace([string][char]10, '\n').Replace([string][char]13, '\r').Replace([string][char]9, '\t')
    return '"' + $clean + '"'
}

# --- user32 --------------------------------------------------------------------

$NativeSource = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class EolNative
{
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr lParam);

    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr window, uint relation);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll", EntryPoint = "GetClassNameW", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr window, StringBuilder text, int max);
    [DllImport("user32.dll", EntryPoint = "GetWindowTextW", CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr window, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint from, uint to, bool attach);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll", EntryPoint = "MapVirtualKeyW")] public static extern uint MapVirtualKey(uint code, uint mapType);
    [DllImport("user32.dll", EntryPoint = "VkKeyScanW", CharSet = CharSet.Unicode)] public static extern short VkKeyScan(char character);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [DllImport("user32.dll", EntryPoint = "PostMessageW")]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

    // The visible top-level windows of a process.
    public static IntPtr[] WindowsOf(uint processId)
    {
        List<IntPtr> found = new List<IntPtr>();
        EnumWindows(delegate(IntPtr window, IntPtr unused)
        {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == processId && IsWindowVisible(window)) found.Add(window);
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    public static uint ThreadOf(IntPtr window)
    {
        uint owner;
        return GetWindowThreadProcessId(window, out owner);
    }

    public static string ClassOf(IntPtr window)
    {
        StringBuilder text = new StringBuilder(256);
        GetClassName(window, text, text.Capacity);
        return text.ToString();
    }

    public static string TitleOf(IntPtr window)
    {
        StringBuilder text = new StringBuilder(512);
        GetWindowText(window, text, text.Capacity);
        return text.ToString();
    }

    // Left, top, width, height.
    public static int[] RectOf(IntPtr window)
    {
        RECT rect;
        GetWindowRect(window, out rect);
        return new int[] { rect.Left, rect.Top, rect.Right - rect.Left, rect.Bottom - rect.Top };
    }
}
'@

function Initialize-Native {
    if ('EolNative' -as [type]) { return }
    Add-Type -TypeDefinition $NativeSource
}

# --- windows -------------------------------------------------------------------

# Native dialogs (message boxes, file dialogs) have this window class.
$DialogClass = '#32770'

# The editor window of a process: its widest visible window that is not a
# dialog and has no owner. With several windows the first one found may be another.
function Get-MainWindow([int]$processId) {
    $best = [IntPtr]::Zero
    $bestWidth = 0
    foreach ($window in [EolNative]::WindowsOf([uint32]$processId)) {
        if ([EolNative]::ClassOf($window) -eq $DialogClass) { continue }
        $owner = [EolNative]::GetWindow($window, 4)
        if ($owner -ne [IntPtr]::Zero) { continue }
        $width = [EolNative]::RectOf($window)[2]
        if ($width -le $bestWidth) { continue }
        $best = $window
        $bestWidth = $width
    }
    return $best
}

function Test-MainWindow([int]$processId) {
    return ((Get-MainWindow $processId) -ne [IntPtr]::Zero)
}

# The native dialogs of a process, each with Handle and Title.
function Get-Dialogs([int]$processId) {
    $dialogs = @()
    foreach ($window in [EolNative]::WindowsOf([uint32]$processId)) {
        if ([EolNative]::ClassOf($window) -ne $DialogClass) { continue }
        $dialogs += New-Object PSObject -Property @{ Handle = $window; Title = [EolNative]::TitleOf($window) }
    }
    return $dialogs
}

function Test-Dialog([int]$processId) {
    return (@(Get-Dialogs $processId).Count -gt 0)
}

function Test-Foreground([IntPtr]$window) {
    return ([EolNative]::GetForegroundWindow() -eq $window)
}

# Brings a window to the foreground and says whether it got there. Keys go to
# the foreground window, so nothing is typed when this is false.
function Set-Foreground([IntPtr]$window) {
    if ($window -eq [IntPtr]::Zero) { return $false }
    if ([EolNative]::IsIconic($window)) { [void][EolNative]::ShowWindow($window, 9) }
    [void][EolNative]::SetForegroundWindow($window)
    if (Test-Foreground $window) { return $true }
    # Only the thread that has the foreground may give it away: join its input and retry.
    $own = [EolNative]::GetCurrentThreadId()
    $other = [EolNative]::ThreadOf([EolNative]::GetForegroundWindow())
    [void][EolNative]::AttachThreadInput($own, $other, $true)
    [void][EolNative]::SetForegroundWindow($window)
    [void][EolNative]::AttachThreadInput($own, $other, $false)
    Start-Sleep -Milliseconds 200
    return (Test-Foreground $window)
}

function Click-WindowCenter([IntPtr]$window) {
    $rect = [EolNative]::RectOf($window)
    $x = $rect[0] + [int]($rect[2] / 2)
    $y = $rect[1] + [int]($rect[3] / 2)
    [void][EolNative]::SetCursorPos($x, $y)
    [EolNative]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    [EolNative]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
}

# WM_CLOSE, what the close button of the title bar sends.
function Close-Window([IntPtr]$window) {
    [void][EolNative]::PostMessage($window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
}

# Clicks the Yes button of a dialog by its control id (IDYES), whatever its
# label is in the language of the system. False when the dialog has none.
function Click-YesButton([IntPtr]$dialog) {
    $button = [EolNative]::GetDlgItem($dialog, 6)
    if ($button -eq [IntPtr]::Zero) { return $false }
    # BM_CLICK
    [void][EolNative]::PostMessage($button, 0xF5, [IntPtr]::Zero, [IntPtr]::Zero)
    return $true
}

# The whole desktop as a PNG. False when it cannot be captured.
function Save-Screenshot([string]$path) {
    try {
        Add-Type -AssemblyName System.Windows.Forms
        Add-Type -AssemblyName System.Drawing
        $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
        $bitmap = New-Object System.Drawing.Bitmap($bounds.Width, $bounds.Height)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
        $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        $graphics.Dispose()
        $bitmap.Dispose()
        return $true
    } catch {
        return $false
    }
}

# --- keys ----------------------------------------------------------------------

# xdotool key names, as the case files use them, to Windows virtual-key codes.
# Case-sensitive like xdotool: End is a key, end is not.
$KeyCodes = New-Object System.Collections.Hashtable
$KeyCodes['ctrl'] = 0x11
$KeyCodes['alt'] = 0x12
$KeyCodes['shift'] = 0x10
$KeyCodes['super'] = 0x5B
$KeyCodes['BackSpace'] = 0x08
$KeyCodes['Tab'] = 0x09
$KeyCodes['Return'] = 0x0D
$KeyCodes['Escape'] = 0x1B
$KeyCodes['space'] = 0x20
$KeyCodes['Prior'] = 0x21
$KeyCodes['Page_Up'] = 0x21
$KeyCodes['Next'] = 0x22
$KeyCodes['Page_Down'] = 0x22
$KeyCodes['End'] = 0x23
$KeyCodes['Home'] = 0x24
$KeyCodes['Left'] = 0x25
$KeyCodes['Up'] = 0x26
$KeyCodes['Right'] = 0x27
$KeyCodes['Down'] = 0x28
$KeyCodes['Insert'] = 0x2D
$KeyCodes['Delete'] = 0x2E
foreach ($number in 1..12) { $KeyCodes['F' + $number] = 0x6F + $number }

# Keys of the navigation block: keybd_event needs KEYEVENTF_EXTENDEDKEY for
# them, or they count as keys of the numeric pad.
$ExtendedKeys = 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2D, 0x2E, 0x5B

# One key name to its code. A single letter or digit is its own key.
function ConvertTo-KeyCode([string]$name) {
    if ($KeyCodes.ContainsKey($name)) { return [int]$KeyCodes[$name] }
    if ($name -cmatch '^[A-Za-z0-9]$') { return [int][char]$name.ToUpperInvariant() }
    throw ('unknown key name: ' + $name)
}

# 'ctrl+End' to its codes in the order they go down: 0x11, 0x23.
function ConvertTo-KeyChord([string]$chord) {
    return @($chord.Split('+') | ForEach-Object { ConvertTo-KeyCode $_ })
}

# 'ctrl+End F2 End' to a list of chords.
function ConvertTo-KeyChords([string]$keys) {
    $chords = New-Object System.Collections.ArrayList
    foreach ($chord in @($keys -split '\s+' | Where-Object { $_ })) {
        [void]$chords.Add([int[]](ConvertTo-KeyChord $chord))
    }
    return ,$chords
}

# The chord that types a character on the current keyboard layout (VkKeyScanW:
# low byte the key, high byte 1 Shift, 2 Ctrl, 4 Alt).
function ConvertTo-CharChord([char]$character) {
    $scan = [EolNative]::VkKeyScan($character)
    if ($scan -eq -1) { throw ('the keyboard layout has no key for character U+{0:X4}' -f [int]$character) }
    $codes = @()
    $state = ($scan -shr 8) -band 0xFF
    if ($state -band 2) { $codes += 0x11 }
    if ($state -band 4) { $codes += 0x12 }
    if ($state -band 1) { $codes += 0x10 }
    $codes += ($scan -band 0xFF)
    return $codes
}

function Send-KeyEvent([int]$code, [bool]$up) {
    $flags = 0
    if ($ExtendedKeys -contains $code) { $flags = $flags -bor 1 }
    if ($up) { $flags = $flags -bor 2 }
    $scan = [EolNative]::MapVirtualKey([uint32]$code, 0)
    [EolNative]::keybd_event([byte]$code, [byte]($scan -band 0xFF), [uint32]$flags, [UIntPtr]::Zero)
}

# Presses the codes in order and releases them in reverse.
function Send-KeyChord([int[]]$codes) {
    foreach ($code in $codes) { Send-KeyEvent $code $false }
    for ($index = $codes.Length - 1; $index -ge 0; $index--) { Send-KeyEvent $codes[$index] $true }
}

# xdotool key --delay 200 KEYS
function Send-Keys([string]$keys) {
    foreach ($chord in (ConvertTo-KeyChords $keys)) {
        Send-KeyChord $chord
        Start-Sleep -Milliseconds 200
    }
}

# xdotool type --delay 40 TEXT
function Send-Text([string]$text) {
    foreach ($character in $text.ToCharArray()) {
        Send-KeyChord ([int[]](ConvertTo-CharChord $character))
        Start-Sleep -Milliseconds 40
    }
}

# --- app -----------------------------------------------------------------------

# Starts the app on FILE and returns its process id.
function Start-App([string]$exe, [string]$file) {
    $argument = '"{0}"' -f $file
    $process = Start-Process -FilePath $exe -ArgumentList $argument -WorkingDirectory (Split-Path -Parent $file) -PassThru
    return $process.Id
}

# The log of an instance is js-debug-<start ms>-<pid>.log; the newest for the pid.
function Find-InstanceLog([int]$processId) {
    $root = Get-TempRoot
    if (-not (Test-Path -LiteralPath $root)) { return '' }
    $logs = @(Get-ChildItem -LiteralPath $root -Filter ('js-debug-*-{0}.log' -f $processId) | Sort-Object Name)
    if ($logs.Count -eq 0) { return '' }
    return $logs[$logs.Count - 1].FullName
}

# Reads a file the app may be appending to.
function Read-SharedText([string]$path) {
    if (-not $path) { return '' }
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return '' }
    $stream = [System.IO.File]::Open($path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
    try {
        $reader = New-Object System.IO.StreamReader($stream)
        return $reader.ReadToEnd()
    } finally {
        $stream.Dispose()
    }
}

# The first line of a log that matches a regex, or nothing.
function Get-LogLine([string]$path, [string]$pattern) {
    foreach ($line in ((Read-SharedText $path) -split '\r?\n')) {
        if ([regex]::IsMatch($line, $pattern)) { return $line }
    }
    return ''
}

# Return on each dialog accepts its default button.
function Dismiss-Dialogs([int]$processId) {
    foreach ($dialog in @(Get-Dialogs $processId)) {
        if (-not (Set-Foreground $dialog.Handle)) { continue }
        Send-Keys 'Return'
        Start-Sleep -Seconds 1
    }
}

function Stop-ProcessTree([int]$processId) {
    & taskkill.exe /PID $processId /T /F | Out-Null
}

# Closes the app by its main window and returns closed, or killed when it had
# to be ended after 20 s.
function Close-App([int]$processId) {
    if (-not (Test-ProcessAlive $processId)) { return 'closed' }
    $window = Get-MainWindow $processId
    if ($window -ne [IntPtr]::Zero) { Close-Window $window }
    $gone = Wait-Until 20 { -not (Test-ProcessAlive $processId) }
    if ($gone) { return 'closed' }
    Stop-ProcessTree $processId
    return 'killed'
}
