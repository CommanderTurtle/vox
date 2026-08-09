[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $pwsh = (Get-Process -Id $PID).Path
    $quotedScript = '"' + $PSCommandPath.Replace('"', '\"') + '"'
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File $quotedScript"
    $process = Start-Process -FilePath $pwsh -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    exit $process.ExitCode
}

$prerequisiteInstaller = Join-Path $PSScriptRoot 'install-wdk-prerequisites.ps1'
if (-not (Test-Path -LiteralPath $prerequisiteInstaller)) {
    throw 'The native Windows driver prerequisite installer is missing.'
}
& $prerequisiteInstaller -ToolsOnly

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

$devgen = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Tools') -Filter 'devgen.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\devgen\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $devgen) { throw 'Microsoft DevGen was not found in the Windows 11 WDK Tools directory.' }
$devcon = Get-ChildItem -Path (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Tools') -Filter 'devcon.exe' -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\devcon\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $devcon) { throw 'Microsoft DevCon was not found in the WDK Tools directory.' }

$inf = Join-Path $package 'ComponentizedAudioSample.inf'
if (-not (Test-Path -LiteralPath $inf)) { throw "Missing driver INF: $inf" }

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
    & pnputil.exe /add-driver $inf /install
    $result = $LASTEXITCODE
    if ($result -notin @(0, 3010)) {
        throw "PnPUtil installation failed with exit code $result. Confirm that Test Signing is active in the running kernel before retrying."
    }
    $requiresRestart = $result -eq 3010
}
else {
    # PnPUtil intentionally will not force a same-version package over the
    # installed driver. DevCon update is safe here because the one existing
    # root devnode has already been identified; unlike `devcon install`, it
    # cannot create a duplicate cable.
    & $devcon.FullName update $inf $hardwareId
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
        Write-Warning 'Windows accepted the Vox driver but requires a restart before the cable endpoints can be verified. Restart once, then run install-development.ps1 again.'
        exit 3010
    }
    $device = @(Get-CimInstance -ClassName Win32_PnPEntity -ErrorAction SilentlyContinue |
        Where-Object { @($_.HardwareID) -contains $hardwareId } |
        Select-Object -First 1)
    $status = if ($device) { "$($device[0].Status): $($device[0].ConfigManagerErrorCode)" } else { 'root device missing' }
    throw "The package was staged, but Windows did not enumerate exactly one Vox playback and recording endpoint (device status $status)."
}

Write-Host "Installed and enumerated $($render[0].FriendlyName) and $($capture[0].FriendlyName)."
if ($requiresRestart) {
    Write-Warning 'Windows reports that a restart is still required to finish the driver update. No reboot was initiated.'
}
Write-Host 'Run vox-mic-forwarder.exe --verify-cable to confirm the router sees the same WASAPI endpoints and negotiated formats.'
if ($requiresRestart) { exit 3010 }
