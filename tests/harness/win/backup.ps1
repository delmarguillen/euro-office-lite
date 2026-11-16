# backup.ps1 DIR: saves the app's recovery folder, recent files and window
# state into DIR\state, with the sha256 of every file for restore.ps1, and
# checks the copy against the original.
param(
    [Parameter(Position = 0)][string]$Dir
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')

function Copy-Entries([string]$from, [string]$to, [string[]]$entries) {
    foreach ($entry in $entries) {
        Copy-Item -LiteralPath (Join-Path $from $entry) -Destination (Join-Path $to $entry) -Recurse -Force
    }
}

function Invoke-Backup([string]$dir) {
    if (-not $dir) { Stop-Harness 'usage: backup.ps1 DIR' }
    $config = Get-ConfigDir
    if (-not (Test-Path -LiteralPath $config -PathType Container)) { Stop-Harness ('no ' + $config) }
    if (@(Get-AppProcessIds).Count -gt 0) { Stop-Harness 'the app is running: close it first' }
    $dir = Resolve-FullPath $dir
    $state = Join-Path $dir 'state'
    # A backup made here before would mix with this one.
    if (Test-Path -LiteralPath $state) { Remove-Item -LiteralPath $state -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $state | Out-Null
    $entries = @(Get-ExistingEntries $config)
    Copy-Entries $config $state $entries
    $snapshot = @(Get-Snapshot $config $entries)
    $copied = @(Get-Snapshot $state $entries)
    if (($snapshot -join '|') -ne ($copied -join '|')) { Stop-Harness ('the copy in ' + $state + ' differs from ' + $config) }
    Write-Lines (Join-Path $dir 'entries.txt') $entries
    Write-Lines (Join-Path $dir 'snapshot.txt') $snapshot
    Write-Output ('saved ' + $snapshot.Count + ' paths to ' + $dir)
}

Invoke-Backup $Dir
