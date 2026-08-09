[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$IsAdministrator = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $IsAdministrator) {
    throw 'Open PowerShell as Administrator before running this script.'
}

$DevCon = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Tools') -Filter 'devcon.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $DevCon) { throw 'Microsoft DevCon was not found in the WDK Tools directory.' }

& $DevCon.FullName remove 'Root\Sysvad_ComponentizedAudioSample'
$result = $LASTEXITCODE
if ($result -notin @(0, 1)) { throw "DevCon removal failed with exit code $result." }
Write-Host 'Removed all Vox native cable root devices. The test-signing boot policy was not changed.'
if ($result -eq 1) {
    Write-Warning 'Windows requires a restart to finish removing the driver device. No reboot was initiated.'
    exit 3010
}
