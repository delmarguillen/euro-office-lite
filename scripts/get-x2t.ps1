param(
    [string]$Repo = "delmarguillen/euro-office-lite"
)

$ErrorActionPreference = "Stop"
$targetDir = Join-Path $PSScriptRoot "..\src-tauri\binaries"
$zipName = "x2t-binaries-windows-x64-9.4.0.129.zip"
$tempZip = Join-Path $env:TEMP $zipName
# Same pin as scripts/get-x2t-ci.ps1.
$expectedSha256 = '072dd361b7a9fbaa7c1da197c4fbfce3d6a52b63610ac70fb0cb21ec66f9d10c'

if (Test-Path (Join-Path $targetDir "x2t.exe")) {
    Write-Host "x2t already exists in $targetDir"
    exit 0
}

Write-Host "Downloading x2t binaries from '$Repo' dependencies release..."
gh release download dependencies --repo $Repo --pattern $zipName --output $tempZip --clobber
if ($LASTEXITCODE -ne 0) { throw "download of $zipName failed" }

$actualSha256 = (Get-FileHash $tempZip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
    Remove-Item $tempZip -Force
    throw "sha256 mismatch for ${zipName}: expected $expectedSha256, actual $actualSha256"
}

if (-not (Test-Path $targetDir)) {
    New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
}

Write-Host "Extracting to $targetDir..."
Expand-Archive -Path $tempZip -DestinationPath $targetDir -Force
Remove-Item $tempZip -Force

# The app runs binaries\x2t.exe on every architecture (x64 under emulation on
# arm64), so it keeps the name it has in the zip.
if (-not (Test-Path (Join-Path $targetDir "x2t.exe"))) {
    Write-Error "x2t binary not found after extraction"
    exit 1
}

$count = (Get-ChildItem $targetDir -Recurse -File).Count
Write-Host "x2t ready: $count files in $targetDir"
