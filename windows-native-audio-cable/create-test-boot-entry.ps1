[CmdletBinding()]
param(
    [string]$Description = 'Vox Driver Test'
)

$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Open PowerShell as Administrator before running this script.'
}

$stateDirectory = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable'
$stateFile = Join-Path $stateDirectory 'test-boot-entry.txt'
if (Test-Path -LiteralPath $stateFile) {
    throw "A saved Vox test entry already exists: $(Get-Content -Raw -LiteralPath $stateFile)"
}

$copyOutput = @(& bcdedit.exe /copy '{current}' /d $Description 2>&1)
if ($LASTEXITCODE -ne 0) {
    throw "BCDEdit could not copy the current boot entry:`n$($copyOutput -join [Environment]::NewLine)"
}

$match = [regex]::Match(($copyOutput -join ' '), '\{[0-9a-fA-F-]{36}\}')
if (-not $match.Success) {
    throw "BCDEdit created an entry but its GUID could not be parsed:`n$($copyOutput -join [Environment]::NewLine)"
}
$entry = $match.Value

try {
    & bcdedit.exe /set $entry testsigning on
    if ($LASTEXITCODE -ne 0) {
        throw 'Windows rejected TESTSIGNING for the copied boot entry. Secure Boot policy may be preventing local test-signed drivers.'
    }

    New-Item -ItemType Directory -Force -Path $stateDirectory | Out-Null
    Set-Content -LiteralPath $stateFile -Value $entry -Encoding ascii -NoNewline
}
catch {
    & bcdedit.exe /delete $entry /cleanup | Out-Null
    throw
}

Write-Host "Created non-default test entry: $entry ($Description)"
Write-Host 'The ordinary boot entry was not modified. Run select-test-boot-once.ps1 before the one reboot used for driver installation.'
