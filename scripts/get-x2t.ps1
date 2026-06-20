param(
    [string]$Version = "v9.4.0"
)

$ErrorActionPreference = "Stop"
$targetDir = Join-Path $PSScriptRoot "..\src-tauri\binaries"
$installerUrl = "https://github.com/ONLYOFFICE/DesktopEditors/releases/download/$Version/DesktopEditors_x64.exe"
$tempInstaller = Join-Path $env:TEMP "DesktopEditors_x64.exe"

if (Test-Path (Join-Path $targetDir "x2t-*.exe")) {
    Write-Host "x2t already exists in $targetDir"
    exit 0
}

Write-Host "Downloading ONLYOFFICE Desktop Editors $Version..."
if (-not (Test-Path $tempInstaller)) {
    Invoke-WebRequest -Uri $installerUrl -OutFile $tempInstaller -UseBasicParsing
}

Write-Host "Installing silently (will require admin)..."
Start-Process -FilePath $tempInstaller -ArgumentList "/VERYSILENT", "/NORESTART", "/SUPPRESSMSGBOXES" -Wait -Verb RunAs

$converterDir = Join-Path $env:ProgramFiles "ONLYOFFICE\DesktopEditors\converter"
if (-not (Test-Path $converterDir)) {
    Write-Error "ONLYOFFICE converter not found at $converterDir"
    exit 1
}

if (-not (Test-Path $targetDir)) {
    New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
}

Write-Host "Copying x2t and dependencies..."
Get-ChildItem $converterDir -File | ForEach-Object {
    Copy-Item $_.FullName -Destination $targetDir -Force
}

$arch = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq "Arm64") { "aarch64" } else { "x86_64" }
$triple = "$arch-pc-windows-msvc"
Rename-Item (Join-Path $targetDir "x2t.exe") "x2t-$triple.exe" -Force

Write-Host "x2t installed to $targetDir (target: $triple)"
Write-Host "You can now uninstall ONLYOFFICE Desktop Editors if desired."
