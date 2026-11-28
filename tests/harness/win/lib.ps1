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
# The read shares write and delete: on Windows the app refuses to save over a
# file another handle holds without them (Get-FileHash shares only read), and a
# hash taken while it saves would make the save fail.
function Get-Sha256([string]$path) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return '' }
    $share = [System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete
    try {
        $stream = [System.IO.File]::Open($path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, $share)
    } catch {
        return ''
    }
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $digest = $hasher.ComputeHash($stream)
    } finally {
        $hasher.Dispose()
        $stream.Dispose()
    }
    return ([System.BitConverter]::ToString($digest).Replace('-', '').ToLowerInvariant())
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

# --- PDF -----------------------------------------------------------------------

# Whether a file starts with the five bytes every PDF starts with.
function Test-PdfHeader([string]$path) {
    $stream = [System.IO.File]::Open($path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
    try {
        $head = New-Object byte[] 5
        $count = $stream.Read($head, 0, 5)
    } finally {
        $stream.Dispose()
    }
    if ($count -lt 5) { return $false }
    return ([System.Text.Encoding]::ASCII.GetString($head) -ceq '%PDF-')
}

# The pdftotext of poppler or xpdf: the one on the PATH, else the one Git for
# Windows has in mingw64\bin. Nothing when the machine has neither.
function Find-PdfToText {
    $command = Get-Command pdftotext -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    $git = Get-Command git -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $git) { return '' }
    $gitRoot = Split-Path -Parent (Split-Path -Parent $git.Source)
    $bundled = Join-Path (Join-Path (Join-Path $gitRoot 'mingw64') 'bin') 'pdftotext.exe'
    if (Test-Path -LiteralPath $bundled -PathType Leaf) { return $bundled }
    return ''
}

# The first line pdftotext -v prints: poppler writes it to standard error and
# xpdf to standard output. A Process object, because 5.1 turns the standard
# error of a native command into errors that stop the script.
function Get-PdfToTextVersion([string]$tool) {
    $start = New-Object System.Diagnostics.ProcessStartInfo $tool, '-v'
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.CreateNoWindow = $true
    $process = [System.Diagnostics.Process]::Start($start)
    $errorText = $process.StandardError.ReadToEnd()
    $outputText = $process.StandardOutput.ReadToEnd()
    $process.WaitForExit()
    $lines = @(($errorText + $outputText) -split '\r?\n' | Where-Object { $_ })
    if ($lines.Count -eq 0) { return '' }
    return $lines[0]
}

# Says in the log which pdftotext the pdf-text check uses, or that there is none.
function Write-PdfToTextInUse {
    $tool = Find-PdfToText
    if (-not $tool) {
        Write-Log 'pdftotext: none found; pdf-text cases end in no-pdftotext'
        return
    }
    Write-Log ('pdftotext: {0} ({1})' -f $tool, (Get-PdfToTextVersion $tool))
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
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window, uint which);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll", EntryPoint = "FindWindowExW", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string className, string title);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out int value, int size);

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

    public static uint ProcessOf(IntPtr window)
    {
        uint owner;
        GetWindowThreadProcessId(window, out owner);
        return owner;
    }

    // Every top-level window, from the front to the back. EnumWindows gives
    // only the windows of desktop apps; this walk also gives the screens the
    // system shows above the desktop.
    public static IntPtr[] TopLevelWindows()
    {
        List<IntPtr> found = new List<IntPtr>();
        IntPtr window = FindWindowEx(IntPtr.Zero, IntPtr.Zero, null, null);
        while (window != IntPtr.Zero && found.Count < 10000)
        {
            found.Add(window);
            window = FindWindowEx(IntPtr.Zero, window, null, null);
        }
        return found.ToArray();
    }

    // The top-level window a window belongs to (GA_ROOT), or itself.
    public static IntPtr RootOf(IntPtr window)
    {
        if (window == IntPtr.Zero) return window;
        IntPtr root = GetAncestor(window, 2);
        return root == IntPtr.Zero ? window : root;
    }

    // DWMWA_CLOAKED: the window exists and counts as visible, but is not drawn.
    public static bool IsCloaked(IntPtr window)
    {
        int cloaked;
        if (DwmGetWindowAttribute(window, 14, out cloaked, 4) != 0) return false;
        return cloaked != 0;
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

# A key no keyboard has (0xE8). Pressed while ALT is down, it makes the ALT a
# chord: the window that sees ALT go up does not open its menu.
$MenuMaskKey = 0xE8

# Whether run.ps1 got -ClearDesktop: only then is anything of the desktop closed.
$ClearsDesktop = $false

# Brings a window to the foreground and says whether it got there. Keys go to
# the foreground window, so nothing is typed when this is false.
function Set-Foreground([IntPtr]$window) {
    if ($window -eq [IntPtr]::Zero) { return $false }
    # The Start menu may open while the cases run: the first sign-in goes on.
    # A panel that could not be closed does not stop the case: its detail says
    # what kept the foreground.
    if ($ClearsDesktop) {
        try {
            Close-ShellPanels
        } catch {
            Write-Log ('the Start menu or the Search panel could not be closed: ' + $_.Exception.Message)
        }
    }
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
    if (Test-Foreground $window) { return $true }
    # Last try: the system lets SetForegroundWindow through once ALT is
    # pressed (LockSetForegroundWindow, Remarks).
    Send-KeyEvent 0x12 $false
    [void][EolNative]::SetForegroundWindow($window)
    Send-KeyChord ([int[]]@($MenuMaskKey))
    Send-KeyEvent 0x12 $true
    Start-Sleep -Milliseconds 200
    return (Test-Foreground $window)
}

# Class, title and process of a window, for the log and the detail of a case.
function Get-WindowDescription([IntPtr]$window) {
    if ($window -eq [IntPtr]::Zero) { return 'no window' }
    $ownerId = [EolNative]::ProcessOf($window)
    $name = '?'
    $process = Get-Process -Id $ownerId -ErrorAction SilentlyContinue
    if ($process) { $name = $process.ProcessName }
    return ("class '{0}', title '{1}', process {2} (pid {3})" -f [EolNative]::ClassOf($window), [EolNative]::TitleOf($window), $name, $ownerId)
}

# What has the foreground now: said in the detail of a case that could not get it.
function Get-ForegroundDescription {
    return (Get-WindowDescription ([EolNative]::GetForegroundWindow()))
}

# --- screens over the desktop ----------------------------------------------------

# Whether a rectangle (left, top, width, height) covers a screen of that size
# placed at 0,0.
function Test-RectCoversScreen([int[]]$rect, [int]$screenWidth, [int]$screenHeight) {
    if ($screenWidth -le 0 -or $screenHeight -le 0) { return $false }
    if ($rect[0] -gt 0 -or $rect[1] -gt 0) { return $false }
    return ((($rect[0] + $rect[2]) -ge $screenWidth) -and (($rect[1] + $rect[3]) -ge $screenHeight))
}

# A window that is shown and covers the primary screen (SM_CXSCREEN, SM_CYSCREEN).
function Test-CoversScreen([IntPtr]$window) {
    if (-not [EolNative]::IsWindowVisible($window)) { return $false }
    if ([EolNative]::IsCloaked($window)) { return $false }
    return (Test-RectCoversScreen ([EolNative]::RectOf($window)) ([EolNative]::GetSystemMetrics(0)) ([EolNative]::GetSystemMetrics(1)))
}

# The programs that show the screens of a first sign-in ("Choose privacy
# settings for your device" and the like) over the desktop of a new user.
$FirstRunHosts = 'WWAHost', 'CloudExperienceHost', 'CloudExperienceHostBroker', 'UserOOBEBroker', 'FirstLogonAnim', 'msoobe'

# The folders of Windows whose programs show such screens.
function Get-FirstRunFolders([string]$windowsDir) {
    $root = $windowsDir.TrimEnd('\')
    return @(($root + '\SystemApps'), ($root + '\System32\oobe'))
}

# Whether a process is one that shows a screen of a first sign-in: by its name,
# or, for the window that has the foreground, by the folder of its program.
# The path may be empty: not every process lets its path be read.
function Test-FirstRunProcess([string]$name, [string]$path, [bool]$hasForeground, [string]$windowsDir) {
    if ($FirstRunHosts -contains $name) { return $true }
    if (-not $hasForeground -or -not $path -or -not $windowsDir) { return $false }
    foreach ($folder in (Get-FirstRunFolders $windowsDir)) {
        if ($path.StartsWith($folder + '\', [System.StringComparison]::OrdinalIgnoreCase)) { return $true }
    }
    return $false
}

function Get-ProcessPath($process) {
    try {
        return [string]$process.Path
    } catch {
        return ''
    }
}

# The screens of a first sign-in that cover the desktop, each with Handle,
# ProcessId and Description. Such a screen keeps the foreground, whatever
# SetForegroundWindow is called with, and takes the clicks meant for the app.
function Get-BlockingWindows {
    $foreground = [EolNative]::RootOf([EolNative]::GetForegroundWindow())
    $seen = @{}
    $found = @()
    foreach ($window in (@($foreground) + @([EolNative]::TopLevelWindows()))) {
        if ($window -eq [IntPtr]::Zero) { continue }
        $key = $window.ToInt64()
        if ($seen.ContainsKey($key)) { continue }
        $seen[$key] = $true
        if (-not (Test-CoversScreen $window)) { continue }
        $ownerId = [EolNative]::ProcessOf($window)
        if ($ownerId -eq $PID) { continue }
        $process = Get-Process -Id $ownerId -ErrorAction SilentlyContinue
        if (-not $process) { continue }
        $first = Test-FirstRunProcess $process.ProcessName (Get-ProcessPath $process) ($window -eq $foreground) $env:WINDIR
        if (-not $first) { continue }
        $found += New-Object PSObject -Property @{ Handle = $window; ProcessId = [int]$ownerId; Description = (Get-WindowDescription $window) }
    }
    return $found
}

# The programs of the Start menu and of its search box, which is also the
# Search panel of the taskbar. Windows 11 opens the Start menu at the first
# sign-in of a user; while it is open the search box has the foreground and
# SetForegroundWindow does not take it away.
$ShellPanelHosts = 'SearchHost', 'StartMenuExperienceHost'

# Escape closes the panel, as it does for a person; a panel that is still
# there after this many gets its process ended, which closes its window.
$ShellPanelEscapes = 3
$ShellPanelAttempts = 5

function Test-ShellPanelProcess([string]$name) {
    return ($ShellPanelHosts -contains $name)
}

# What an attempt to close a panel does: escape or end.
function Get-ShellPanelStep([int]$attempt) {
    if ($attempt -le $ShellPanelEscapes) { return 'escape' }
    return 'end'
}

# The Start menu or the Search panel when it has the foreground, with
# ProcessId and Description; nothing when another window has it, or none.
function Get-ForegroundShellPanel {
    $window = [EolNative]::RootOf([EolNative]::GetForegroundWindow())
    if ($window -eq [IntPtr]::Zero) { return $null }
    $ownerId = [EolNative]::ProcessOf($window)
    $process = Get-Process -Id $ownerId -ErrorAction SilentlyContinue
    if (-not $process) { return $null }
    if (-not (Test-ShellPanelProcess $process.ProcessName)) { return $null }
    return (New-Object PSObject -Property @{ ProcessId = [int]$ownerId; Description = (Get-WindowDescription $window) })
}

function Test-ShellPanelClosed {
    return ($null -eq (Get-ForegroundShellPanel))
}

# Closes the Start menu or the Search panel while it has the foreground, and
# says in the log what it did and what has the foreground afterwards. It sends
# Escape only after reading that the panel has the foreground, and does
# nothing when another window has it. For -ClearDesktop of run.ps1.
function Close-ShellPanels {
    $attempts = 0
    foreach ($attempt in 1..$ShellPanelAttempts) {
        $panel = Get-ForegroundShellPanel
        if (-not $panel) { break }
        $attempts++
        if ((Get-ShellPanelStep $attempt) -eq 'escape') {
            Write-Log ('the Start menu or the Search panel has the foreground, pressing Escape: ' + $panel.Description)
            Send-Keys 'Escape'
        } else {
            Write-Log ('Escape did not close it, ending its process: ' + $panel.Description)
            Stop-Process -Id $panel.ProcessId -Force -ErrorAction SilentlyContinue
        }
        [void](Wait-Until 5 { Test-ShellPanelClosed })
    }
    if ($attempts -eq 0) { return }
    $panel = Get-ForegroundShellPanel
    if ($panel) {
        Write-Log ('still has the foreground: ' + $panel.Description)
        return
    }
    Write-Log ('foreground after closing it: ' + (Get-ForegroundDescription))
}

# Ends the processes of the screens of a first sign-in that cover the desktop,
# then closes the Start menu or the Search panel that has the foreground, and
# says in the log what it found and what has the foreground afterwards.
# For a machine that is thrown away after the run: -ClearDesktop of run.ps1.
function Clear-Desktop {
    Write-Log ('foreground before the cases: ' + (Get-ForegroundDescription))
    $ended = 0
    foreach ($round in 1..3) {
        $blocking = @(Get-BlockingWindows)
        if ($blocking.Count -eq 0) { break }
        foreach ($entry in $blocking) {
            Write-Log ('a first sign-in screen covers the desktop, ending its process: ' + $entry.Description)
            Stop-Process -Id $entry.ProcessId -Force -ErrorAction SilentlyContinue
            $ended++
        }
        [void](Wait-Until 10 { @(Get-BlockingWindows).Count -eq 0 })
    }
    foreach ($entry in @(Get-BlockingWindows)) {
        Write-Log ('still covers the desktop: ' + $entry.Description)
    }
    if ($ended -gt 0) {
        Write-Log ('foreground after ending them: ' + (Get-ForegroundDescription))
        # The window under the screen that went takes the foreground in a moment.
        [void](Wait-Until 3 { -not (Test-ShellPanelClosed) })
    }
    Close-ShellPanels
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

# The keys that press a button of the page from where the click in the middle
# of the window leaves the focus, or nothing for a label without them. UI
# Automation does not find the buttons of the page in WebView2.
function Get-ButtonKeys([string]$label) {
    # The Markdown page opens in its preview: four Shift+Tab reach Edit.
    if ($label -ceq 'Edit') { return 'shift+Tab shift+Tab shift+Tab shift+Tab space' }
    return ''
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

# Starts the app with no document, on its start screen, and returns its process id.
function Start-AppEmpty([string]$exe, [string]$folder) {
    $process = Start-Process -FilePath $exe -WorkingDirectory $folder -PassThru
    return $process.Id
}

# Ends the app at once, as a crash would: no window is closed, nothing is saved.
function Stop-AppAbruptly([int]$processId) {
    Stop-Process -Id $processId -Force
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

# The first group of a regex in the text of a log, or nothing.
function Get-LogValue([string]$path, [string]$pattern) {
    $found = [regex]::Match((Read-SharedText $path), $pattern)
    if (-not $found.Success) { return '' }
    return $found.Groups[1].Value
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
