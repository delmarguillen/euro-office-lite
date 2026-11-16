# status.ps1 [INSTALLER...]: what is installed, read from the uninstall entry
# and from the exe. The exe carries no commit id, and an NSIS installer cannot
# be opened to hash the exe it holds: each INSTALLER given is shown with its
# own sha256 and version, to compare with the installed version.
param(
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Installers
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')

function Show-Package {
    $version = Get-InstalledVersion
    if (-not $version) { $version = 'not installed' }
    Write-Output ('package: ' + $AppProduct + ' ' + $version)
}

function Show-Binary {
    $sum = Get-InstalledSha256
    if (-not $sum) { $sum = 'missing' }
    Write-Output ('binary: ' + (Get-AppExe) + ' sha256 ' + $sum)
}

function Show-Installer([string]$installer) {
    $version = Get-FileProductVersion $installer
    $verdict = 'differs from'
    if ($version -and ($version -eq (Get-InstalledVersion))) { $verdict = 'is' }
    Write-Output ('installer: ' + (Split-Path -Leaf $installer) + ' sha256 ' + (Get-Sha256 $installer) + ' version ' + $version + ' ' + $verdict + ' the installed version')
}

function Show-Processes {
    $running = (@(Get-AppProcessIds) -join ' ')
    if (-not $running) { $running = 'none' }
    Write-Output ('running: ' + $running)
}

function Show-UserState {
    Write-Output ('recover entries: ' + @(Get-RecoverEntries).Count)
    Write-Output ('recent-files.json: ' + (Get-Sha256 (Join-Path (Get-ConfigDir) 'recent-files.json')))
}

Show-Package
Show-Binary
foreach ($installer in $Installers) {
    Show-Installer $installer
}
Show-Processes
Show-UserState
