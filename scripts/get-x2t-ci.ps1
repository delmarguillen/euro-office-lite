$ErrorActionPreference = "Stop"

$targetDir = Join-Path $PSScriptRoot "..\src-tauri\binaries"
$repo = $env:GITHUB_REPOSITORY
$zipName = "x2t-binaries-windows-x64-10.0.0.169.zip"
$tempZip = Join-Path $env:TEMP $zipName

if (Test-Path (Join-Path $targetDir "x2t.exe")) {
    Write-Host "x2t already present, skipping download"
    exit 0
}

# sha256 of the zip in the 'dependencies' release. A replaced asset must fail
# the build until its new digest is pinned here.
$expectedSha256 = 'PENDING_SHA256_WINDOWS_X64_10_0_0_169'

# A PENDING_ placeholder marks a zip whose digest is not pinned yet.
if ($expectedSha256.StartsWith('PENDING_')) {
    throw "sha256 for $zipName is still the placeholder $expectedSha256; pin the published digest in get-x2t-ci.ps1"
}

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

$x2t = Join-Path $targetDir "x2t.exe"
if (-not (Test-Path $x2t)) {
    Write-Error "x2t binary not found after extraction"
    exit 1
}
# The installer ships binaries\x2t.exe. The build workflows still rename this
# target-named copy for the arm64 installer, and nothing else uses it.
Copy-Item $x2t (Join-Path $targetDir "x2t-x86_64-pc-windows-msvc.exe") -Force

$count = (Get-ChildItem $targetDir -Recurse -File).Count
Write-Host "x2t binaries ready: $count files in $targetDir"
