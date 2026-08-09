[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$CertificatePath,

    [Parameter(Mandatory)]
    [string]$PfxPath,

    [Security.SecureString]$PfxPassword,

    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    [ValidateSet('x64', 'ARM64')]
    [string]$Platform = 'x64',

    [switch]$Refresh
)

$ErrorActionPreference = 'Stop'
$CodeSigningEku = '1.3.6.1.5.5.7.3.3'
$HardwareId = 'Root\Sysvad_ComponentizedAudioSample'

function Find-WdkTool {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [ValidateSet('bin', 'Tools')]
        [string]$Area
    )

    $root = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\$Area"
    $tool = Get-ChildItem -Path $root -Filter $Name -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\x64\\' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $tool) {
        throw "$Name was not found in the Windows Driver Kit $Area directory."
    }
    $tool.FullName
}

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

$identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
if ($identity -ne 'NT SERVICE\TrustedInstaller') {
    throw "Run this script from the TrustedInstaller PowerShell described in the repository README. Current identity: $identity"
}

$resolvedCertificate = Resolve-ExistingFile -Path $CertificatePath -Label 'Public certificate'
$resolvedPfx = Resolve-ExistingFile -Path $PfxPath -Label 'PFX'
if (-not $PfxPassword) {
    $PfxPassword = Read-Host 'PFX password' -AsSecureString
}

$rootCertificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($resolvedCertificate)
& certutil.exe -addstore root $resolvedCertificate
if ($LASTEXITCODE -ne 0) {
    throw 'CertUtil did not import the public certificate into Cert:\LocalMachine\Root.'
}

$imported = @(Import-PfxCertificate `
    -FilePath $resolvedPfx `
    -CertStoreLocation 'Cert:\LocalMachine\My' `
    -Password $PfxPassword)

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

& (Join-Path $PSScriptRoot 'build.ps1') `
    -Configuration $Configuration `
    -Platform $Platform `
    -Refresh:$Refresh

$package = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable'
$catalogs = @(Get-ChildItem -LiteralPath $package -Filter '*.cat' -File)
if ($catalogs.Count -ne 1) {
    throw "Expected exactly one catalog in $package; found $($catalogs.Count)."
}

$signTool = Find-WdkTool -Name 'signtool.exe' -Area 'bin'
& $signTool sign /fd SHA256 /sm /s My /sha1 $signer.Thumbprint $catalogs[0].FullName
if ($LASTEXITCODE -ne 0) {
    throw "SignTool failed with exit code $LASTEXITCODE."
}

$inf = Join-Path $package 'ComponentizedAudioSample.inf'
if (-not (Test-Path -LiteralPath $inf -PathType Leaf)) {
    throw "The signed package is missing its INF: $inf"
}

$devgen = Find-WdkTool -Name 'devgen.exe' -Area 'Tools'
$devcon = Find-WdkTool -Name 'devcon.exe' -Area 'Tools'
$devices = @(Get-CimInstance -ClassName Win32_PnPEntity -ErrorAction Stop |
    Where-Object { @($_.HardwareID) -contains $HardwareId })
if ($devices.Count -gt 1) {
    throw "Found $($devices.Count) matching root devices. Run uninstall.ps1 before reinstalling."
}

$restartRequired = $false
if ($devices.Count -eq 0) {
    & $devgen /add /bus ROOT /hardwareid $HardwareId
    if ($LASTEXITCODE -ne 0) {
        throw "DevGen failed with exit code $LASTEXITCODE."
    }

    & pnputil.exe /add-driver $inf /install
    if ($LASTEXITCODE -notin @(0, 3010)) {
        throw "PnPUtil failed with exit code $LASTEXITCODE."
    }
    $restartRequired = $LASTEXITCODE -eq 3010
}
else {
    & $devcon update $inf $HardwareId
    if ($LASTEXITCODE -notin @(0, 1)) {
        throw "DevCon failed with exit code $LASTEXITCODE."
    }
    $restartRequired = $LASTEXITCODE -eq 1
}

Write-Host "Certificate: Cert:\LocalMachine\My\$($signer.Thumbprint)"
Write-Host "Private key: $machineKey"
Write-Host "Signed catalog: $($catalogs[0].FullName)"
Write-Host 'Installed endpoints: Vox Cable Input -> Vox Cable Output'
if ($restartRequired) {
    Write-Warning 'Windows reports that a restart is required. No restart was initiated.'
    exit 3010
}
