[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Open PowerShell as Administrator before running this script.'
}

$package = Join-Path $PSScriptRoot 'dist\windows-native-audio-cable'
if (-not (Test-Path -LiteralPath $package)) {
    throw 'No signed package exists. Run build.ps1 and sign-package.ps1 first.'
}

$catalogs = @(Get-ChildItem -LiteralPath $package -Filter '*.cat' -File)
if ($catalogs.Count -ne 1) {
    throw "Expected exactly one package catalog in $package; found $($catalogs.Count)."
}

$signtool = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin') -Filter 'signtool.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $signtool) { throw 'Microsoft SignTool was not found in the WDK.' }

& $signtool.FullName verify /v /pa $catalogs[0].FullName
if ($LASTEXITCODE -ne 0) {
    throw 'The package catalog is not signed by a currently trusted certificate. Run sign-package.ps1 first.'
}

$devcon = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Tools') -Filter 'devcon.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\devcon\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $devcon) { throw 'Microsoft DevCon was not found in the WDK Tools directory.' }

$inf = Join-Path $package 'ComponentizedAudioSample.inf'
if (-not (Test-Path -LiteralPath $inf)) { throw "Missing driver INF: $inf" }

& $devcon.FullName install $inf 'Root\Sysvad_ComponentizedAudioSample'
if ($LASTEXITCODE -ne 0) {
    throw "DevCon installation failed with exit code $LASTEXITCODE. Boot the dedicated Vox Driver Test entry before retrying."
}

Write-Host 'Installed Vox Cable Input (playback) and Vox Cable Output (recording).'
