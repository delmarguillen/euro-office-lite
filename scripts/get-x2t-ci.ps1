$ErrorActionPreference = "Stop"

$targetDir = Join-Path $PSScriptRoot "..\src-tauri\binaries"
$repo = $env:GITHUB_REPOSITORY
$zipName = "x2t-binaries.zip"
$tempZip = Join-Path $env:TEMP $zipName

if (Test-Path (Join-Path $targetDir "x2t-x86_64-pc-windows-msvc.exe")) {
    Write-Host "x2t already present, skipping download"
    exit 0
}

Write-Host "Downloading x2t binaries from 'dependencies' release..."
gh release download dependencies --repo $repo --pattern $zipName --output $tempZip

if (-not (Test-Path $targetDir)) {
    New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
}

Write-Host "Extracting to $targetDir..."
Expand-Archive -Path $tempZip -DestinationPath $targetDir -Force

if (-not (Test-Path (Join-Path $targetDir "x2t-x86_64-pc-windows-msvc.exe"))) {
    $x2t = Join-Path $targetDir "x2t.exe"
    if (Test-Path $x2t) {
        Rename-Item $x2t "x2t-x86_64-pc-windows-msvc.exe" -Force
        Write-Host "Renamed x2t.exe -> x2t-x86_64-pc-windows-msvc.exe"
    } else {
        Write-Error "x2t binary not found after extraction"
        exit 1
    }
}

$count = (Get-ChildItem $targetDir -Recurse -File).Count
Write-Host "x2t binaries ready: $count files in $targetDir"
