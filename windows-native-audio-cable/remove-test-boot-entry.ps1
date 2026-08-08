[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Open PowerShell as Administrator before running this script.'
}

$stateFile = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable\test-boot-entry.txt'
if (-not (Test-Path -LiteralPath $stateFile)) {
    throw 'No saved Vox test boot entry exists.'
}
$entry = (Get-Content -Raw -LiteralPath $stateFile).Trim()
if ($entry -notmatch '^\{[0-9a-fA-F-]{36}\}$') {
    throw "Invalid saved boot-entry identifier: $entry"
}

& bcdedit.exe /delete $entry /cleanup
if ($LASTEXITCODE -ne 0) {
    throw "BCDEdit could not delete $entry."
}
Remove-Item -LiteralPath $stateFile -Force
Write-Host "Removed the Vox test boot entry: $entry"
