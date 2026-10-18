$ErrorActionPreference = "Stop"

$targetDir = Join-Path $PSScriptRoot "..\src-tauri\binaries"
$repo = $env:GITHUB_REPOSITORY
$zipName = "x2t-binaries.zip"
$tempZip = Join-Path $env:TEMP $zipName

if (Test-Path (Join-Path $targetDir "x2t-x86_64-pc-windows-msvc.exe")) {
    Write-Host "x2t already present, skipping download"
    exit 0
}

# sha256 of x2t-binaries.zip in the 'dependencies' release. A replaced asset
# must fail the build until its new digest is pinned here.
$expectedSha256 = '8d27e54dfa39cb60672753969fb5b76833f274a3aa9140e0e6cbbae1b67f64b9'

# Retry only the download (transient API/network errors); a checksum
# mismatch below is never retried.
Write-Host "Downloading x2t binaries from 'dependencies' release..."
$attempt = 1
$delay = 10
while ($true) {
    gh release download dependencies --repo $repo --pattern $zipName --output $tempZip --clobber
    if ($LASTEXITCODE -eq 0) { break }
    if ($attempt -ge 3) { throw "download of $zipName failed after $attempt attempts" }
    Write-Host "Download attempt $attempt failed; retrying in ${delay}s"
    Start-Sleep -Seconds $delay
    $attempt++
    $delay *= 2
}

$actualSha256 = (Get-FileHash $tempZip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
    Remove-Item $tempZip -Force
    throw "sha256 mismatch for ${zipName}: expected $expectedSha256, actual $actualSha256"
}
Write-Host "sha256 verified for ${zipName}: $actualSha256"

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
