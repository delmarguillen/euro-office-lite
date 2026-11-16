# restore.ps1 DIR: puts back what backup.ps1 DIR saved (an entry that did not
# exist then is removed) and checks the result against its snapshot.
# Exit status: 0 restored and equal, 1 restored but different, 2 could not run.
param(
    [Parameter(Position = 0)][string]$Dir
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')

function Remove-Entries([string]$config) {
    foreach ($entry in $StateEntries) {
        $path = Join-Path $config $entry
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force }
    }
}

function Restore-Entries([string]$state, [string]$config, [string[]]$entries) {
    New-Item -ItemType Directory -Force -Path $config | Out-Null
    Remove-Entries $config
    foreach ($entry in $entries) {
        Copy-Item -LiteralPath (Join-Path $state $entry) -Destination (Join-Path $config $entry) -Recurse -Force
    }
}

function Invoke-Restore([string]$dir) {
    if (-not $dir) { Stop-Harness 'usage: restore.ps1 DIR' }
    $dir = Resolve-FullPath $dir
    $snapshotFile = Join-Path $dir 'snapshot.txt'
    if (-not (Test-Path -LiteralPath $snapshotFile -PathType Leaf)) { Stop-Harness ('no backup in ' + $dir) }
    if (@(Get-AppProcessIds).Count -gt 0) { Stop-Harness 'the app is running: close it first' }
    $config = Get-ConfigDir
    $entries = @(Read-Lines (Join-Path $dir 'entries.txt'))
    Restore-Entries (Join-Path $dir 'state') $config $entries
    $before = @(Read-Lines $snapshotFile)
    $after = @(Get-Snapshot $config $entries)
    Write-Lines (Join-Path $dir 'snapshot-after.txt') $after
    if (($before -join '|') -eq ($after -join '|')) {
        Write-Output ('restored ' + $before.Count + ' paths from ' + $dir + ', sha256 match')
        return
    }
    Write-Log 'restored state differs from the backup:'
    foreach ($line in @(Compare-Object $before $after)) {
        Write-Log ($line.SideIndicator + ' ' + $line.InputObject)
    }
    exit 1
}

Invoke-Restore $Dir
