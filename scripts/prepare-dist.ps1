<#
.SYNOPSIS
  Stages only the runtime-needed files from src/ into src-dist/.
  Run before "npx tauri build" so frontendDist points to a slim tree.
#>
param(
    [string]$ProjectRoot = (Split-Path $PSScriptRoot)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$src     = Join-Path $ProjectRoot 'src'
$dist    = Join-Path $ProjectRoot 'src-dist'
$logDir  = Join-Path $env:TEMP 'euro-office-lite'
$logFile = Join-Path $logDir 'prepare-dist.log'

if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir -Force | Out-Null }

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$totalFiles = 0
$totalBytes = 0

function Log($msg) {
    $line = "$(Get-Date -Format 'HH:mm:ss') $msg"
    $line | Out-File -FilePath $logFile -Append -Encoding utf8
    Write-Host $line
}

# Start fresh log
"" | Out-File -FilePath $logFile -Encoding utf8
Log "=== prepare-dist.ps1 ==="
Log "Source:      $src"
Log "Destination: $dist"

# Clean previous dist (junction-safe: don't recurse into a dev-mode symlink)
if (Test-Path $dist) {
    $item = Get-Item $dist -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        Log "Removing previous src-dist/ (junction)"
        $item.Delete()
    } else {
        Log "Removing previous src-dist/ (directory)"
        Remove-Item -Recurse -Force $dist
    }
}

function Copy-Tree {
    param(
        [string]$Source,
        [string]$Dest,
        [string[]]$ExcludeDirs = @(),
        [string]$Label
    )

    if (-not (Test-Path $Source)) {
        Log "SKIP (not found): $Label -> $Source"
        return
    }

    $xdArgs = @('.git') + $ExcludeDirs
    $roboArgs = @($Source, $Dest, '/E', '/NJH', '/NJS', '/NP', '/NDL', '/NFL', '/R:0', '/W:0')
    foreach ($xd in $xdArgs) {
        $roboArgs += '/XD'
        $roboArgs += $xd
    }

    $null = & robocopy @roboArgs 2>&1

    $copied = Get-ChildItem -Path $Dest -Recurse -File -ErrorAction SilentlyContinue
    $count  = ($copied | Measure-Object).Count
    $size   = ($copied | Measure-Object -Property Length -Sum).Sum
    if ($null -eq $size) { $size = 0 }

    $script:totalFiles += $count
    $script:totalBytes += $size

    Log ("COPY: {0,-55} {1,6} files  {2,8:N1} MB" -f $Label, $count, ($size / 1MB))
}

function Copy-SingleFile {
    param([string]$Source, [string]$Dest, [string]$Label)

    if (-not (Test-Path $Source)) {
        Log "SKIP (not found): $Label"
        return
    }

    $destDir = Split-Path $Dest
    if (-not (Test-Path $destDir)) { New-Item -ItemType Directory -Path $destDir -Force | Out-Null }
    Copy-Item -Path $Source -Destination $Dest -Force

    $size = (Get-Item $Dest).Length
    $script:totalFiles += 1
    $script:totalBytes += $size
    Log ("COPY: {0,-55} {1,6} files  {2,8:N1} MB" -f $Label, 1, ($size / 1MB))
}

# --- Root files ---
Copy-SingleFile "$src\index.html"  "$dist\index.html"  "index.html"
Copy-SingleFile "$src\bridge.js"   "$dist\bridge.js"   "bridge.js"

# --- Fonts ---
Copy-Tree "$src\fonts" "$dist\fonts" @() "src/fonts"

# --- web-apps editors (main/ without help/) ---
$editors = @('documenteditor', 'spreadsheeteditor', 'presentationeditor')
foreach ($ed in $editors) {
    Copy-Tree "$src\web-apps\apps\$ed\main" "$dist\web-apps\apps\$ed\main" @('help') "web-apps/apps/$ed/main"
}

# --- web-apps shared ---
Copy-Tree "$src\web-apps\apps\api"    "$dist\web-apps\apps\api"    @() "web-apps/apps/api"
Copy-Tree "$src\web-apps\apps\common" "$dist\web-apps\apps\common" @() "web-apps/apps/common"

# --- web-apps vendor (only needed libraries) ---
$vendorLibs = @('backbone', 'underscore', 'xregexp', 'jquery', 'jquery.browser',
                'requirejs', 'requirejs-text', 'es6-promise', 'fetch',
                'perfect-scrollbar', 'svg-injector', 'socketio', 'less')
foreach ($lib in $vendorLibs) {
    Copy-Tree "$src\web-apps\vendor\$lib" "$dist\web-apps\vendor\$lib" @() "web-apps/vendor/$lib"
}

$excludedVendor = @('ace', 'framework7-react', 'monaco')
foreach ($lib in $excludedVendor) {
    Log ("EXCL: {0,-55} (not needed at runtime)" -f "web-apps/vendor/$lib")
}

# --- sdkjs modules ---
$sdkModules = @('word', 'cell', 'slide', 'common')
foreach ($mod in $sdkModules) {
    Copy-Tree "$src\sdkjs\$mod" "$dist\sdkjs\$mod" @() "sdkjs/$mod"
}

# --- sdkjs/pdf (referenced by word editor's scripts.js) ---
Copy-Tree "$src\sdkjs\pdf" "$dist\sdkjs\pdf" @('build', 'test', '.git') "sdkjs/pdf"

# --- sdkjs/vendor (polyfill.js, string.js, etc.) ---
Copy-Tree "$src\sdkjs\vendor" "$dist\sdkjs\vendor" @() "sdkjs/vendor"

# --- sdkjs/develop (scripts.js per editor) ---
$developModules = @('word', 'cell', 'slide')
foreach ($mod in $developModules) {
    Copy-Tree "$src\sdkjs\develop\sdkjs\$mod" "$dist\sdkjs\develop\sdkjs\$mod" @() "sdkjs/develop/sdkjs/$mod"
}

$excludedSdk = @('tests', 'build', 'visio', '.docker', '.github')
foreach ($d in $excludedSdk) {
    Log ("EXCL: {0,-55} (not needed at runtime)" -f "sdkjs/$d")
}

$excludedWebApps = @('pdfeditor', 'visioeditor', 'build', '.docker', '.github', 'test')
foreach ($d in $excludedWebApps) {
    Log ("EXCL: {0,-55} (not needed at runtime)" -f "web-apps/$d")
}

# --- Summary ---
$sw.Stop()
Log ""
Log "========================================"
Log ("Total files:  {0:N0}" -f $totalFiles)
Log ("Total size:   {0:N1} MB" -f ($totalBytes / 1MB))
Log ("Elapsed:      {0:N1} s" -f $sw.Elapsed.TotalSeconds)
Log "========================================"
Log "Done. frontendDist should point to ../src-dist"

exit 0
