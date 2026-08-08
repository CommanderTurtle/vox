[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [ValidateSet('x64', 'ARM64')]
    [string]$Platform = 'x64',
    [switch]$Refresh
)

$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'sync-microsoft-sysvad.ps1') -Refresh:$Refresh

$SourceRoot = Join-Path $PSScriptRoot '.work\windows-driver-samples'
$Solution = Join-Path $SourceRoot 'audio\sysvad\sysvad.sln'
$VsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $VsWhere)) {
    throw 'Visual Studio Installer (vswhere.exe) was not found.'
}

$MSBuild = & $VsWhere -latest -products * -requires Microsoft.Component.MSBuild -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
if (-not $MSBuild) {
    throw 'MSBuild was not found in the installed Visual Studio instance.'
}

$WdkTargets = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\build') -Filter 'WindowsDriver.Common.targets' -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $WdkTargets) {
    throw 'Windows Driver Kit build targets were not found. Install the Microsoft WDK matching your Windows SDK, then rerun this script.'
}

& $MSBuild $Solution /m /t:Build "/p:Configuration=$Configuration" "/p:Platform=$Platform" '/p:SignMode=Off'
if ($LASTEXITCODE -ne 0) {
    throw "SysVAD build failed with exit code $LASTEXITCODE."
}

$Package = Get-ChildItem -Path (Join-Path $SourceRoot 'audio\sysvad') -Directory -Recurse |
    Where-Object { $_.Name -eq 'package' -and (Test-Path -LiteralPath (Join-Path $_.FullName 'ComponentizedAudioSample.inf')) } |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if (-not $Package) {
    throw 'The build completed, but its unsigned driver package could not be located.'
}

$Dist = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable'
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
Copy-Item -Path (Join-Path $Package.FullName '*') -Destination $Dist -Recurse -Force

Write-Host "Vox native cable package: $Dist"
Write-Host 'The package is unsigned. Run sign-package.ps1 from an elevated PowerShell 7.1+ before installation.'
