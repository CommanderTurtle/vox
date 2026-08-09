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

if (-not ('Vox.NativeCodeIntegrity' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace Vox
{
    [StructLayout(LayoutKind.Sequential)]
    public struct CodeIntegrityInformation
    {
        public UInt32 Length;
        public UInt32 Options;
    }

    public static class NativeCodeIntegrity
    {
        [DllImport("ntdll.dll")]
        private static extern Int32 NtQuerySystemInformation(
            Int32 informationClass,
            ref CodeIntegrityInformation information,
            Int32 informationLength,
            out Int32 returnLength);

        public static Int32 Query(out UInt32 options)
        {
            var information = new CodeIntegrityInformation();
            information.Length = (UInt32)Marshal.SizeOf<CodeIntegrityInformation>();
            Int32 returnLength;
            Int32 status = NtQuerySystemInformation(
                103,
                ref information,
                Marshal.SizeOf<CodeIntegrityInformation>(),
                out returnLength);
            options = information.Options;
            return status;
        }
    }
}
'@
}

$codeIntegrityOptions = [uint32]0
$queryStatus = [Vox.NativeCodeIntegrity]::Query([ref]$codeIntegrityOptions)
if ($queryStatus -ne 0) {
    throw ('Windows Code Integrity status query failed with NTSTATUS 0x{0:X8}.' -f [uint32]$queryStatus)
}
$testSigningActive = ($codeIntegrityOptions -band 0x02) -ne 0

$bcdBefore = @(& bcdedit.exe /enum '{current}' 2>&1)
if ($LASTEXITCODE -ne 0) {
    throw "BCDEdit could not inspect the current boot entry.`n$($bcdBefore -join [Environment]::NewLine)"
}
$testSigningConfigured = [regex]::IsMatch(
    ($bcdBefore -join [Environment]::NewLine),
    '(?im)^\s*testsigning\s+Yes\s*$')

$secureBoot = $null
try {
    $secureBoot = Confirm-SecureBootUEFI -ErrorAction Stop
}
catch {
    Write-Verbose "Secure Boot state could not be queried: $($_.Exception.Message)"
}

Write-Host "Kernel Test Signing active now: $testSigningActive"
Write-Host "Current BCD entry configured for next boot: $testSigningConfigured"
if ($null -ne $secureBoot) {
    Write-Host "UEFI Secure Boot enabled: $secureBoot"
}

if (-not $testSigningConfigured) {
    Write-Host 'Test signing is not configured. Enabling it on the current Windows boot entry...'
    $bcdOutput = @(& bcdedit.exe /set '{current}' testsigning on 2>&1)
    if ($LASTEXITCODE -ne 0) {
        $secureBootMessage = if ($secureBoot -eq $true) {
            ' UEFI Secure Boot is enabled; Microsoft normally protects TESTSIGNING from modification in this state.'
        }
        else { '' }
        throw "Windows rejected TESTSIGNING before any signer was created.$secureBootMessage`n$($bcdOutput -join [Environment]::NewLine)"
    }

    $bcdAfter = @(& bcdedit.exe /enum '{current}' 2>&1)
    $testSigningConfigured = $LASTEXITCODE -eq 0 -and [regex]::IsMatch(
        ($bcdAfter -join [Environment]::NewLine),
        '(?im)^\s*testsigning\s+Yes\s*$')
    if (-not $testSigningConfigured) {
        throw "BCDEdit returned success, but the current boot entry does not report TESTSIGNING enabled.`n$($bcdAfter -join [Environment]::NewLine)"
    }
    Write-Host 'Test signing is now configured and will become active after reboot.'
}
elseif ($testSigningActive) {
    Write-Host 'Test signing is already configured and active; no BCD change is needed.'
}
else {
    Write-Host 'Test signing is already configured but not active in this running kernel. A reboot is required.'
}

if (-not $testSigningActive) {
    Write-Host ''
    Write-Host 'Restart Windows, select this boot entry, then run install-development.ps1 once more.'
    Write-Host 'The driver has not been built or signed yet, so the post-restart run performs the only build.'
    exit 3010
}

Write-Host 'Building the pinned Microsoft SysVAD adaptation...'
& (Join-Path $PSScriptRoot 'build.ps1') -Refresh:$Refresh

Write-Host 'Creating or reusing the one-operation Vox signer...'
& (Join-Path $PSScriptRoot 'sign-package.ps1')

Write-Host 'Installing the test-signed root audio device...'
& (Join-Path $PSScriptRoot 'install-signed.ps1')

Write-Host ''
Write-Host 'Vox native audio cable development installation finished.'
Write-Host 'Test signing was active for the installation. A reboot is optional unless Windows reports that the device requires one.'
Write-Host 'The installer did not reboot the computer and did not add any account to an administrator or signing group.'
Write-Host 'All Vox private signing keys have already been destroyed; one public verification certificate remains.'
