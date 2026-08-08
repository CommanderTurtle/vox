[CmdletBinding()]
param(
    [switch]$Refresh
)

$ErrorActionPreference = 'Stop'
$isAdministrator = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdministrator) {
    $pwsh = (Get-Process -Id $PID).Path
    $quotedScript = '"' + $PSCommandPath.Replace('"', '\"') + '"'
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File $quotedScript"
    if ($Refresh) { $arguments += ' -Refresh' }
    $process = Start-Process -FilePath $pwsh -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    exit $process.ExitCode
}

if ($PSVersionTable.PSVersion -lt [Version]'7.1') {
    throw 'Run this entry point with PowerShell 7.1+ (pwsh). It is required for guaranteed private-key deletion.'
}

Write-Host 'Building the pinned Microsoft SysVAD adaptation...'
& (Join-Path $PSScriptRoot 'build.ps1') -Refresh:$Refresh

Write-Host 'Enabling signed-driver Test Mode on the current Windows boot entry...'
$bcdOutput = @(& bcdedit.exe /set '{current}' testsigning on 2>&1)
if ($LASTEXITCODE -ne 0) {
    throw "Windows rejected TESTSIGNING before any signer was created. Secure Boot firmware policy may be blocking it.`n$($bcdOutput -join [Environment]::NewLine)"
}

Write-Host 'Creating or reusing the one-operation Vox signer...'
& (Join-Path $PSScriptRoot 'sign-package.ps1')

Write-Host 'Installing the test-signed root audio device...'
& (Join-Path $PSScriptRoot 'install-signed.ps1')

Write-Host ''
Write-Host 'Vox native audio cable development installation finished.'
Write-Host 'A reboot is recommended so Windows enters signed-driver Test Mode and starts the audio endpoints cleanly.'
Write-Host 'The installer did not reboot the computer and did not add any account to an administrator or signing group.'
Write-Host 'All Vox private signing keys have already been destroyed; one public verification certificate remains.'
