[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [ValidateSet('x64', 'ARM64')]
    [string]$Platform = 'x64',
    [switch]$Refresh
)

$ErrorActionPreference = 'Stop'
$PrerequisiteInstaller = Join-Path $PSScriptRoot 'install-wdk-prerequisites.ps1'
$VsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$WdkTargets = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\build') -Filter 'WindowsDriver.Common.targets' -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
$MSBuild = $null
if (Test-Path -LiteralPath $VsWhere) {
    $MSBuild = & $VsWhere -latest -products * -requires Microsoft.Component.MSBuild -requires Component.Microsoft.Windows.DriverKit -find 'MSBuild\**\Bin\amd64\MSBuild.exe' | Select-Object -First 1
    if (-not $MSBuild) {
        $MSBuild = & $VsWhere -latest -products * -requires Microsoft.Component.MSBuild -requires Component.Microsoft.Windows.DriverKit -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
    }
}
if (-not (Test-Path -LiteralPath $VsWhere) -or -not $WdkTargets -or -not $MSBuild) {
    if (-not (Test-Path -LiteralPath $PrerequisiteInstaller)) {
        throw 'The WDK is incomplete and its automatic prerequisite installer is missing.'
    }
    Write-Host 'The complete Microsoft driver build environment is not present; installing it now...'
    & $PrerequisiteInstaller
    $MSBuild = & $VsWhere -latest -products * -requires Microsoft.Component.MSBuild -requires Component.Microsoft.Windows.DriverKit -find 'MSBuild\**\Bin\amd64\MSBuild.exe' | Select-Object -First 1
    if (-not $MSBuild) {
        $MSBuild = & $VsWhere -latest -products * -requires Microsoft.Component.MSBuild -requires Component.Microsoft.Windows.DriverKit -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
    }
}
if (-not $MSBuild) { throw 'MSBuild with the Windows Driver Kit component was not found after prerequisite installation.' }

$WdkTargets = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\build') -Filter 'WindowsDriver.Common.targets' -Recurse -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $WdkTargets) { throw 'Windows Driver Kit build targets were not found after prerequisite installation.' }

& (Join-Path $PSScriptRoot 'sync-microsoft-sysvad.ps1') -Refresh:$Refresh

$SourceRoot = Join-Path $PSScriptRoot '.work\windows-driver-samples'
$Solution = Join-Path $SourceRoot 'audio\sysvad\sysvad.sln'

& $MSBuild $Solution /m /t:Build "/p:Configuration=$Configuration" "/p:Platform=$Platform" '/p:SignMode=Off' '/p:PreferredToolArchitecture=x64'
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
if (Test-Path -LiteralPath $Dist) {
    $resolvedRoot = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    $resolvedDist = [IO.Path]::GetFullPath($Dist)
    if (-not $resolvedDist.StartsWith($resolvedRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to clean an unexpected package path: $resolvedDist"
    }
    Remove-Item -LiteralPath $resolvedDist -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
Copy-Item -Path (Join-Path $Package.FullName '*') -Destination $Dist -Recurse -Force

Write-Host "Vox native cable package: $Dist"
Write-Host 'The package is unsigned. Run sign-package.ps1 from an elevated PowerShell 7.1+ before installation.'
