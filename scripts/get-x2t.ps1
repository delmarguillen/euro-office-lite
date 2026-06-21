param(
    [string]$Repo = "delmarguillen/euro-office-lite"
)

$ErrorActionPreference = "Stop"
$targetDir = Join-Path $PSScriptRoot "..\src-tauri\binaries"
$zipName = "x2t-binaries.zip"
$tempZip = Join-Path $env:TEMP $zipName

if (Test-Path (Join-Path $targetDir "x2t-*.exe")) {
    Write-Host "x2t already exists in $targetDir"
    exit 0
}

Write-Host "Downloading x2t binaries from '$Repo' dependencies release..."
gh release download dependencies --repo $Repo --pattern $zipName --output $tempZip --clobber

if (-not (Test-Path $targetDir)) {
    New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
}

Write-Host "Extracting to $targetDir..."
Expand-Archive -Path $tempZip -DestinationPath $targetDir -Force
Remove-Item $tempZip -Force

$arch = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq "Arm64") { "aarch64" } else { "x86_64" }
$triple = "$arch-pc-windows-msvc"

$x2tTarget = Join-Path $targetDir "x2t-$triple.exe"
if (-not (Test-Path $x2tTarget)) {
    $x2tX64 = Join-Path $targetDir "x2t-x86_64-pc-windows-msvc.exe"
    $x2tPlain = Join-Path $targetDir "x2t.exe"
    if (Test-Path $x2tX64) {
        Rename-Item $x2tX64 "x2t-$triple.exe" -Force
    } elseif (Test-Path $x2tPlain) {
        Rename-Item $x2tPlain "x2t-$triple.exe" -Force
    } else {
        Write-Error "x2t binary not found after extraction"
        exit 1
    }
}

$count = (Get-ChildItem $targetDir -Recurse -File).Count
Write-Host "x2t ready: $count files in $targetDir (target: $triple)"
