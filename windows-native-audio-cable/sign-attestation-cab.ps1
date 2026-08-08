[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9A-Fa-f ]{40,}$')]
    [string]$CertificateThumbprint,
    [ValidateSet('CurrentUser', 'LocalMachine')]
    [string]$CertificateOwner = 'CurrentUser',
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)

$ErrorActionPreference = 'Stop'
$isAdministrator = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdministrator) {
    $pwsh = (Get-Process -Id $PID).Path
    $quotedScript = '"' + $PSCommandPath.Replace('"', '\"') + '"'
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File $quotedScript -CertificateThumbprint `"$CertificateThumbprint`" -CertificateOwner $CertificateOwner -TimestampUrl `"$TimestampUrl`""
    $process = Start-Process -FilePath $pwsh -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    exit $process.ExitCode
}

$thumbprint = $CertificateThumbprint -replace '\s', ''
$cab = Join-Path $PSScriptRoot 'dist\attestation\VoxNativeAudioCable.cab'
if (-not (Test-Path -LiteralPath $cab)) {
    throw 'No attestation CAB exists. Run prepare-attestation.ps1 first.'
}

$storePath = "Cert:\$CertificateOwner\My\$thumbprint"
$certificate = Get-Item -LiteralPath $storePath -ErrorAction SilentlyContinue
if (-not $certificate) {
    throw "The EV certificate is not visible to this elevated administrator at $storePath."
}
if (-not $certificate.HasPrivateKey) {
    throw 'The selected EV certificate has no usable private key or hardware-token provider.'
}

$signtool = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin') -Filter 'signtool.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $signtool) { throw 'Microsoft SignTool was not found in the Windows SDK/WDK.' }

$storeArgs = @('/s', 'My')
if ($CertificateOwner -eq 'LocalMachine') { $storeArgs += '/sm' }
& $signtool.FullName sign /v /fd SHA256 /td SHA256 /tr $TimestampUrl @storeArgs /sha1 $thumbprint $cab
if ($LASTEXITCODE -ne 0) { throw "EV signing failed with exit code $LASTEXITCODE." }

& $signtool.FullName verify /v /pa $cab
if ($LASTEXITCODE -ne 0) { throw 'The EV-signed submission CAB did not verify.' }

Write-Host "EV-signed HDC submission CAB: $cab"
Write-Host 'No certificate or trust authority was installed, exported, or deleted. Submit this CAB in Partner Center.'
