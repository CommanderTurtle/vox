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

$prerequisiteInstaller = Join-Path $PSScriptRoot 'install-wdk-prerequisites.ps1'
if (-not (Test-Path -LiteralPath $prerequisiteInstaller)) {
    throw 'The native Windows driver prerequisite installer is missing.'
}
& $prerequisiteInstaller -ToolsOnly

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

$toolsRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Tools'
$devcon = Get-ChildItem -Path $toolsRoot -Filter 'devcon.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\devcon\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $devcon) { throw 'Microsoft DevCon was not found in the WDK Tools directory.' }
$devgen = Get-ChildItem -Path $toolsRoot -Filter 'devgen.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\devgen\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $devgen) { throw 'Microsoft DevGen was not found in the Windows 11 WDK Tools directory.' }

$hardwareId = 'Root\Sysvad_ComponentizedAudioSample'
$requiresRestart = $false
$rootDevices = @(Get-CimInstance -ClassName Win32_PnPEntity -ErrorAction Stop |
    Where-Object { @($_.HardwareID) -contains $hardwareId })
if ($rootDevices.Count -gt 1) {
    throw "Found $($rootDevices.Count) existing Vox root devices. Run uninstall.ps1 once before reinstalling so Windows exposes only one cable pair."
}
if ($rootDevices.Count -eq 0) {
    & $devgen.FullName /add /bus ROOT /hardwareid $hardwareId
    if ($LASTEXITCODE -ne 0) {
        throw "DevGen could not create the persistent Vox root device (exit $LASTEXITCODE)."
    }
    & pnputil.exe /add-driver $inf.FullName /install
    $result = $LASTEXITCODE
    if ($result -notin @(0, 3010)) {
        throw "PnPUtil installation failed with exit code $result."
    }
    $requiresRestart = $result -eq 3010
}
else {
    & $devcon.FullName update $inf.FullName $hardwareId
    $result = $LASTEXITCODE
    if ($result -notin @(0, 1)) {
        throw "DevCon update failed with exit code $result."
    }
    $requiresRestart = $result -eq 1
}

$deadline = [DateTime]::UtcNow.AddSeconds(10)
do {
    $endpoints = @(Get-PnpDevice -Class AudioEndpoint -PresentOnly -ErrorAction SilentlyContinue)
    $render = @($endpoints | Where-Object { $_.FriendlyName -like '*Vox Cable Input*' })
    $capture = @($endpoints | Where-Object { $_.FriendlyName -like '*Vox Cable Output*' })
    if ($render.Count -eq 1 -and $capture.Count -eq 1) { break }
    Start-Sleep -Milliseconds 200
} while ([DateTime]::UtcNow -lt $deadline)
if ($render.Count -ne 1 -or $capture.Count -ne 1) {
    if ($requiresRestart) {
        Write-Warning 'Windows accepted the attested driver but requires a restart before the cable endpoints can be verified.'
        exit 3010
    }
    throw 'The attested package was staged, but Windows did not enumerate exactly one Vox playback and recording endpoint.'
}

if ($requiresRestart) {
    Write-Warning 'Windows installed the driver but reported that a reboot is required to finish. No reboot was initiated.'
}
else {
    Write-Host "Installed and enumerated $($render[0].FriendlyName) and $($capture[0].FriendlyName) without changing boot policy or certificate trust."
}
Write-Host 'Run vox-mic-forwarder.exe --verify-cable to confirm the router sees the same WASAPI endpoints and negotiated formats.'
if ($requiresRestart) { exit 3010 }
