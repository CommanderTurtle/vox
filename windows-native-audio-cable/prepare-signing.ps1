[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$CertificatePath,

    [Parameter(Mandatory)]
    [string]$PfxPath,

    [Parameter(Mandatory)]
    [string]$PasswordPath,

    [Parameter(Mandatory)]
    [string]$ThumbprintOutputPath
)

$ErrorActionPreference = 'Stop'
$CodeSigningEku = '1.3.6.1.5.5.7.3.3'

function Resolve-ExistingFile {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Label
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label was not found: $Path"
    }
    (Resolve-Path -LiteralPath $Path).Path
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Certificate preparation requires an elevated Windows Administrator token. Current identity: $($identity.Name)"
}

$resolvedCertificate = Resolve-ExistingFile -Path $CertificatePath -Label 'Public certificate'
$resolvedPfx = Resolve-ExistingFile -Path $PfxPath -Label 'PFX'
$resolvedPassword = Resolve-ExistingFile -Path $PasswordPath -Label 'PFX password file'
$rootCertificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($resolvedCertificate)

& certutil.exe -addstore root $resolvedCertificate
if ($LASTEXITCODE -ne 0) {
    throw 'CertUtil did not import the public certificate into Cert:\LocalMachine\Root.'
}

$plainPassword = (Get-Content -LiteralPath $resolvedPassword -Raw).Trim()
$securePassword = ConvertTo-SecureString -String $plainPassword -AsPlainText -Force
$imported = @(Import-PfxCertificate `
    -FilePath $resolvedPfx `
    -CertStoreLocation 'Cert:\LocalMachine\My' `
    -Password $securePassword)
$plainPassword = $null

$signer = $imported |
    Where-Object {
        $_.HasPrivateKey -and
        @($_.EnhancedKeyUsageList.ObjectId.Value) -contains $CodeSigningEku
    } |
    Select-Object -First 1
if (-not $signer) {
    throw 'The PFX did not import a certificate with a private key and the Code Signing EKU.'
}
if ($rootCertificate.Thumbprint -ne $signer.Thumbprint) {
    throw 'The public certificate and the private-key certificate in the PFX do not match.'
}

$rsa = [Security.Cryptography.X509Certificates.RSACertificateExtensions]::GetRSAPrivateKey($signer)
if (-not ($rsa -is [Security.Cryptography.RSACryptoServiceProvider])) {
    if ($rsa) { $rsa.Dispose() }
    throw 'The imported RSA private key is not stored in the PDF-specified MachineKeys container.'
}
$keyName = $rsa.CspKeyContainerInfo.UniqueKeyContainerName
$rsa.Dispose()

$machineKey = Join-Path $env:ProgramData "Microsoft\Crypto\RSA\MachineKeys\$keyName"
if (-not (Test-Path -LiteralPath $machineKey -PathType Leaf)) {
    throw "The imported private-key container was not found: $machineKey"
}

& icacls.exe $machineKey /grant 'NT SERVICE\TrustedInstaller:F'
if ($LASTEXITCODE -ne 0) {
    throw "TrustedInstaller access could not be granted to $machineKey."
}

[IO.File]::WriteAllText($ThumbprintOutputPath, $signer.Thumbprint + [Environment]::NewLine)
Write-Host "Prepared certificate: Cert:\LocalMachine\My\$($signer.Thumbprint)"
Write-Host "TrustedInstaller key access: $machineKey"
Write-Host "Thumbprint file: $ThumbprintOutputPath"
