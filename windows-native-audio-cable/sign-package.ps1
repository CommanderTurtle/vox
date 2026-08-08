[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Assert-Administrator {
    $principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Open PowerShell 7 as Administrator before running this script.'
    }
}

function Find-SignTool {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $tool = Get-ChildItem -LiteralPath $kits -Filter 'signtool.exe' -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $tool) {
        throw 'Microsoft SignTool was not found. Install the WDK Tools feature first.'
    }
    return $tool.FullName
}

Assert-Administrator
if ($PSVersionTable.PSVersion -lt [Version]'7.1') {
    throw 'This script requires elevated PowerShell 7.1+ (pwsh) so Remove-Item -DeleteKey can destroy the temporary private key.'
}

$package = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable'
if (-not (Test-Path -LiteralPath $package)) {
    throw 'No driver package exists. Run build.ps1 first.'
}

$catalogs = @(Get-ChildItem -LiteralPath $package -Filter '*.cat' -File)
if ($catalogs.Count -ne 1) {
    throw "Expected exactly one package catalog in $package; found $($catalogs.Count)."
}
$catalog = $catalogs[0].FullName
$signtool = Find-SignTool

$administrators = [Security.Principal.SecurityIdentifier]::new(
    [Security.Principal.WellKnownSidType]::BuiltinAdministratorsSid, $null)
$system = [Security.Principal.SecurityIdentifier]::new(
    [Security.Principal.WellKnownSidType]::LocalSystemSid, $null)
$privateKeyAcl = [Security.AccessControl.FileSecurity]::new()
$privateKeyAcl.SetAccessRuleProtection($true, $false)
$privateKeyAcl.SetOwner($administrators)
foreach ($sid in @($administrators, $system)) {
    $rule = [Security.AccessControl.FileSystemAccessRule]::new(
        $sid,
        [Security.AccessControl.FileSystemRights]::FullControl,
        [Security.AccessControl.AccessControlType]::Allow)
    [void]$privateKeyAcl.AddAccessRule($rule)
}

$label = Get-Date -Format 'yyyyMMdd-HHmmss'
$subject = 'CN=Vox Native Audio Cable Development'
$ownedSubject = {
    param([Security.Cryptography.X509Certificates.X509Certificate2]$candidate)
    $candidate.Subject -eq $subject -or $candidate.Subject -like 'CN=Vox Native Audio Cable Package *'
}
$cert = $null
$thumbprint = $null
$trustInstalled = $false
$completed = $false

try {
    $interruptedSigners = @(Get-ChildItem -LiteralPath 'Cert:\LocalMachine\My' |
        Where-Object { (& $ownedSubject $_) -and $_.HasPrivateKey } |
        Sort-Object NotBefore -Descending)

    if ($interruptedSigners.Count -gt 0) {
        $cert = $interruptedSigners[0]
        Write-Host "Reusing interrupted Vox signer: $($cert.Thumbprint)"
        foreach ($duplicate in $interruptedSigners | Select-Object -Skip 1) {
            Remove-Item -LiteralPath "Cert:\LocalMachine\My\$($duplicate.Thumbprint)" -DeleteKey -Force
        }
    }
    else {
        $cert = New-SelfSignedCertificate `
            -Type CodeSigningCert `
            -Subject $subject `
            -FriendlyName "Vox package-only signer $label" `
            -KeyFriendlyName "Vox ephemeral package key $label" `
            -CertStoreLocation 'Cert:\LocalMachine\My' `
            -Provider 'Microsoft Software Key Storage Provider' `
            -KeyAlgorithm RSA `
            -KeyLength 3072 `
            -HashAlgorithm SHA256 `
            -KeyExportPolicy NonExportable `
            -SecurityDescriptor $privateKeyAcl `
            -NotAfter (Get-Date).AddYears(10)
    }

    $thumbprint = $cert.Thumbprint
    if (-not $cert.HasPrivateKey) {
        throw 'The machine certificate was created without an accessible private key.'
    }

    & $signtool sign /v /fd SHA256 /sm /s My /sha1 $thumbprint $catalog
    if ($LASTEXITCODE -ne 0) {
        throw "SignTool failed with exit code $LASTEXITCODE."
    }

    $publicCertificate = Join-Path $package 'vox-native-audio-cable-test.cer'
    Export-Certificate -Cert $cert -FilePath $publicCertificate -Type CERT -Force | Out-Null
    Import-Certificate -FilePath $publicCertificate -CertStoreLocation 'Cert:\LocalMachine\Root' | Out-Null
    Import-Certificate -FilePath $publicCertificate -CertStoreLocation 'Cert:\LocalMachine\TrustedPublisher' | Out-Null
    $trustInstalled = $true

    & $signtool verify /v /pa $catalog
    if ($LASTEXITCODE -ne 0) {
        throw "The signed catalog did not verify successfully (exit $LASTEXITCODE)."
    }

    foreach ($store in @('Root', 'TrustedPublisher')) {
        Get-ChildItem -LiteralPath "Cert:\LocalMachine\$store" |
            Where-Object { (& $ownedSubject $_) -and $_.Thumbprint -ne $thumbprint } |
            Remove-Item -Force
    }

    $completed = $true
}
finally {
    Get-ChildItem -LiteralPath 'Cert:\LocalMachine\My' |
        Where-Object { (& $ownedSubject $_) -and $_.HasPrivateKey } |
        ForEach-Object {
            Remove-Item -LiteralPath "Cert:\LocalMachine\My\$($_.Thumbprint)" -DeleteKey -Force
        }

    if (-not $completed -and $trustInstalled -and $thumbprint) {
        foreach ($store in @('Root', 'TrustedPublisher')) {
            $publicPath = "Cert:\LocalMachine\$store\$thumbprint"
            if (Test-Path -LiteralPath $publicPath) {
                Remove-Item -LiteralPath $publicPath -Force
            }
        }
    }
}

if (Get-ChildItem -LiteralPath 'Cert:\LocalMachine\My' | Where-Object { (& $ownedSubject $_) -and $_.HasPrivateKey }) {
    throw 'The signed catalog exists, but complete Vox private-key cleanup could not be confirmed.'
}

Write-Host "Signed only: $catalog"
Write-Host "Public test certificate: $package\vox-native-audio-cable-test.cer"
Write-Host "Thumbprint: $thumbprint"
Write-Host 'The non-exportable machine private key has been destroyed. Only the public trust certificate remains.'
