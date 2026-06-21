$ErrorActionPreference = "Stop"

$binDir = Join-Path $PSScriptRoot "..\src-tauri\binaries"
$outZip = Join-Path $PSScriptRoot "..\x2t-binaries.zip"

if (-not (Test-Path $binDir)) {
    Write-Error "Binaries directory not found: $binDir"
    exit 1
}

$x2tArm = Join-Path $binDir "x2t-aarch64-pc-windows-msvc.exe"
$x2tX64 = Join-Path $binDir "x2t-x86_64-pc-windows-msvc.exe"
$x2tPlain = Join-Path $binDir "x2t.exe"
$renamed = $false

if ((Test-Path $x2tArm) -and -not (Test-Path $x2tX64)) {
    Copy-Item $x2tArm $x2tX64
    Write-Host "Created x64 copy from ARM64 binary (x2t is x64 under emulation)"
    $renamed = $true
}

if (Test-Path $outZip) { Remove-Item $outZip -Force }

Write-Host "Packaging binaries..."
$tempStaging = Join-Path $env:TEMP "x2t-package-staging"
if (Test-Path $tempStaging) { Remove-Item $tempStaging -Recurse -Force }
New-Item -ItemType Directory -Path $tempStaging -Force | Out-Null

Get-ChildItem $binDir -Exclude "*.log", "x2t-aarch64-pc-windows-msvc.exe" | ForEach-Object {
    if ($_.PSIsContainer) {
        Copy-Item $_.FullName -Destination (Join-Path $tempStaging $_.Name) -Recurse
    } else {
        Copy-Item $_.FullName -Destination $tempStaging
    }
}

Compress-Archive -Path "$tempStaging\*" -DestinationPath $outZip -Force
Remove-Item $tempStaging -Recurse -Force

if ($renamed) { Remove-Item $x2tX64 -Force }

$sizeMB = [math]::Round((Get-Item $outZip).Length / 1MB, 2)
Write-Host "Created $outZip ($sizeMB MB)"
Write-Host ""
Write-Host "Next steps:"
Write-Host "  1. Create GitHub repo and push main"
Write-Host "  2. Create 'dependencies' release and upload the zip:"
Write-Host "     gh release create dependencies x2t-binaries.zip --title 'Build Dependencies' --notes 'x2t converter binaries for CI' --repo <owner>/<repo>"
Write-Host "  3. Push a tag to trigger CI:"
Write-Host "     git tag v0.1.0; git push origin v0.1.0"
