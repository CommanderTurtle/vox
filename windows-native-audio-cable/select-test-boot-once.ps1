[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Open PowerShell as Administrator before running this script.'
}

$stateFile = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable\test-boot-entry.txt'
if (-not (Test-Path -LiteralPath $stateFile)) {
    throw 'No saved Vox test boot entry exists. Run create-test-boot-entry.ps1 first.'
}
$entry = (Get-Content -Raw -LiteralPath $stateFile).Trim()
if ($entry -notmatch '^\{[0-9a-fA-F-]{36}\}$') {
    throw "Invalid saved boot-entry identifier: $entry"
}

& bcdedit.exe /bootsequence $entry
if ($LASTEXITCODE -ne 0) {
    throw "BCDEdit could not select $entry for the next boot."
}

Write-Host "The next reboot will use $entry once. Windows will return to the ordinary default entry on the following boot."
Write-Host 'Restart when ready; this script does not reboot the computer.'
