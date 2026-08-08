[CmdletBinding()]
param(
    [string]$PackageDirectory = ''
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($PackageDirectory)) {
    $PackageDirectory = Join-Path $PSScriptRoot 'dist\attested'
}
else {
    $PackageDirectory = [IO.Path]::GetFullPath($PackageDirectory)
}

$isAdministrator = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdministrator) {
    $pwsh = (Get-Process -Id $PID).Path
    $quotedScript = '"' + $PSCommandPath.Replace('"', '\"') + '"'
    $quotedPackage = '"' + $PackageDirectory.Replace('"', '\"') + '"'
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File $quotedScript -PackageDirectory $quotedPackage"
    $process = Start-Process -FilePath $pwsh -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    exit $process.ExitCode
}

if (-not (Test-Path -LiteralPath $PackageDirectory)) {
    throw "Extract the Microsoft-signed dashboard download beneath: $PackageDirectory"
}

$infs = @(Get-ChildItem -LiteralPath $PackageDirectory -Filter 'ComponentizedAudioSample.inf' -File -Recurse)
if ($infs.Count -ne 1) {
    throw "Expected exactly one ComponentizedAudioSample.inf beneath $PackageDirectory; found $($infs.Count)."
}
$inf = $infs[0]
$catalogs = @(Get-ChildItem -LiteralPath $inf.DirectoryName -Filter '*.cat' -File)
if ($catalogs.Count -ne 1) {
    throw "Expected exactly one Microsoft-generated catalog beside $($inf.FullName); found $($catalogs.Count)."
}

$signtool = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin') -Filter 'signtool.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $signtool) { throw 'Microsoft SignTool was not found in the Windows SDK/WDK.' }

& $signtool.FullName verify /v /kp $catalogs[0].FullName
if ($LASTEXITCODE -ne 0) {
    throw 'The catalog does not satisfy the active Windows kernel-signing policy. Refusing installation.'
}
foreach ($driver in Get-ChildItem -LiteralPath $inf.DirectoryName -Filter '*.sys' -File) {
    & $signtool.FullName verify /v /kp $driver.FullName
    if ($LASTEXITCODE -ne 0) {
        throw "Microsoft kernel signature verification failed: $($driver.FullName)"
    }
}

$devcon = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Tools') -Filter 'devcon.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\devcon\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $devcon) { throw 'Microsoft DevCon was not found in the WDK Tools directory.' }

& $devcon.FullName install $inf.FullName 'Root\Sysvad_ComponentizedAudioSample'
$result = $LASTEXITCODE
if ($result -notin @(0, 3010)) {
    throw "Driver installation failed with exit code $result."
}
if ($result -eq 3010) {
    Write-Warning 'Windows installed the driver but reported that a reboot is required to finish. No reboot was initiated.'
}
else {
    Write-Host 'Installed the Microsoft-attested Vox audio cable without changing boot policy or certificate trust.'
}
