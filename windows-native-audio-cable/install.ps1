[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$CertificateThumbprint
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

function Resolve-MachineKeyPath {
    param(
        [Parameter(Mandatory)]
        [string]$Thumbprint
    )

    $storeDetails = @(& certutil.exe -store My $Thumbprint 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "CertUtil could not inspect Cert:\LocalMachine\My\$Thumbprint."
    }
    $containerLine = $storeDetails |
        Where-Object { $_ -match '^\s*Unique container name:\s*(.+?)\s*$' } |
        Select-Object -First 1
    if (-not $containerLine -or $containerLine -notmatch '^\s*Unique container name:\s*(.+?)\s*$') {
        throw 'CertUtil did not report a unique RSA MachineKeys container name.'
    }
    $path = Join-Path $env:ProgramData "Microsoft\Crypto\RSA\MachineKeys\$($Matches[1])"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "The prepared private-key container was not found: $path"
    }
    $path
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
$trustedInstallerSid = [Security.Principal.NTAccount]::new(
    'NT SERVICE', 'TrustedInstaller').Translate([Security.Principal.SecurityIdentifier])
$isTrustedInstallerIdentity = $identity.User -eq $trustedInstallerSid
$hasTrustedInstallerGroup = $principal.IsInRole($trustedInstallerSid)
if (-not $isTrustedInstallerIdentity -and -not $hasTrustedInstallerGroup) {
    throw "The active Windows token does not contain an enabled TrustedInstaller SID. Current identity: $($identity.Name)"
}
Write-Host "Signing token: $($identity.Name) (TrustedInstaller SID enabled)"

$normalizedThumbprint = ($CertificateThumbprint -replace '\s', '').ToUpperInvariant()
$certificatePath = "Cert:\LocalMachine\My\$normalizedThumbprint"
if (-not (Test-Path -LiteralPath $certificatePath)) {
    throw "The prepared LocalMachine signing certificate was not found: $certificatePath"
}
$signer = Get-Item -LiteralPath $certificatePath
if (-not $signer.HasPrivateKey -or $signer.EnhancedKeyUsageList -notmatch 'Code Signing') {
    throw 'The prepared certificate does not have both a private key and the Code Signing EKU.'
}

$machineKey = Resolve-MachineKeyPath -Thumbprint $signer.Thumbprint

$package = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable'
if (-not (Test-Path -LiteralPath $package -PathType Container)) {
    throw 'The unsigned driver package does not exist. Run build.ps1 from an ordinary Windows developer PowerShell first.'
}
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
