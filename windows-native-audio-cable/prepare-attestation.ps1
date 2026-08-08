[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [ValidateSet('x64', 'ARM64')]
    [string]$Platform = 'x64',
    [switch]$Refresh
)

$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build.ps1') -Configuration $Configuration -Platform $Platform -Refresh:$Refresh

$package = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable'
$attestation = Join-Path $PSScriptRoot 'dist\attestation'
$driverFolder = Join-Path $attestation 'VoxCable'
$cab = Join-Path $attestation 'VoxNativeAudioCable.cab'
$ddf = Join-Path $attestation 'VoxNativeAudioCable.ddf'

if (Test-Path -LiteralPath $attestation) {
    $resolved = [IO.Path]::GetFullPath($attestation)
    $expectedParent = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'dist'))
    if (-not $resolved.StartsWith($expectedParent, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to replace unexpected path: $resolved"
    }
    Remove-Item -LiteralPath $attestation -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $driverFolder | Out-Null

Get-ChildItem -LiteralPath $package -File |
    Where-Object { $_.Extension -ne '.cer' } |
    Copy-Item -Destination $driverFolder -Force

$sourceRoot = Join-Path $PSScriptRoot '.work\windows-driver-samples\audio\sysvad'
$binaryNames = Get-ChildItem -LiteralPath $driverFolder -File |
    Where-Object { $_.Extension -in @('.sys', '.dll', '.exe') } |
    ForEach-Object { $_.BaseName } |
    Sort-Object -Unique

foreach ($name in $binaryNames) {
    $pdb = Get-ChildItem -LiteralPath $sourceRoot -Filter "$name.pdb" -File -Recurse -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTimeUtc -Descending |
        Select-Object -First 1
    if ($pdb) {
        Copy-Item -LiteralPath $pdb.FullName -Destination $driverFolder -Force
    }
    else {
        Write-Warning "No matching PDB was found for $name. Microsoft requests symbols for crash analysis."
    }
}

$files = @(Get-ChildItem -LiteralPath $driverFolder -File | Sort-Object Name)
if (-not ($files | Where-Object Extension -eq '.inf')) { throw 'The submission contains no INF.' }
if (-not ($files | Where-Object Extension -eq '.sys')) { throw 'The submission contains no SYS binary.' }
if (-not ($files | Where-Object Extension -eq '.cat')) { throw 'The submission contains no catalog.' }

$lines = @(
    '; Vox Native Audio Cable Hardware Dev Center submission',
    '.OPTION EXPLICIT',
    '.Set CabinetFileCountThreshold=0',
    '.Set FolderFileCountThreshold=0',
    '.Set FolderSizeThreshold=0',
    '.Set MaxCabinetSize=0',
    '.Set MaxDiskFileCount=0',
    '.Set MaxDiskSize=0',
    '.Set CompressionType=MSZIP',
    '.Set Cabinet=on',
    '.Set Compress=on',
    '.Set DiskDirectoryTemplate=.',
    '.Set CabinetNameTemplate=VoxNativeAudioCable.cab',
    '.Set DestinationDir=VoxCable'
)
$lines += $files | ForEach-Object { '"' + $_.FullName + '"' }
Set-Content -LiteralPath $ddf -Value $lines -Encoding ascii

Push-Location $attestation
try {
    & makecab.exe /F $ddf
    if ($LASTEXITCODE -ne 0) { throw "MakeCab failed with exit code $LASTEXITCODE." }
}
finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $cab)) {
    throw "MakeCab completed but did not create $cab."
}

Write-Host "Unsigned HDC submission CAB: $cab"
Write-Host 'Next: run sign-attestation-cab.ps1 with the thumbprint of the EV certificate owned by the administrator/hardware token.'
