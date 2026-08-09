[CmdletBinding()]
param(
    [string]$RouterExe,
    [switch]$OpenSoundSettings
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($RouterExe)) {
    $RouterExe = Join-Path $RepoRoot 'target\release\vox-mic-forwarder.exe'
}
$RouterExe = [IO.Path]::GetFullPath($RouterExe)

if (-not (Test-Path -LiteralPath $RouterExe -PathType Leaf)) {
    $cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
    if (-not $cargo) {
        $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    }
    if (-not $cargo) {
        throw 'cargo is not on PATH. Build vox-mic-forwarder first or pass -RouterExe.'
    }

    Write-Host 'Building the optional Vox microphone router...'
    Push-Location $RepoRoot
    try {
        & $cargo.Source build --release --features mic-forwarder --bin vox-mic-forwarder
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
}

Write-Host 'Checking the VB-CABLE endpoints through Vox CPAL/WASAPI discovery...'
& $RouterExe --verify-cable
if ($LASTEXITCODE -ne 0) {
    Write-Host ''
    Write-Host 'VB-CABLE is not installed or Windows has not exposed it since installation.' -ForegroundColor Yellow
    Write-Host 'Download VBCABLE_Driver_Pack45.zip from the official page, extract it locally,'
    Write-Host 'run VBCABLE_Setup_x64.exe as Administrator, and reboot Windows:'
    Write-Host 'https://vb-audio.com/Cable/'
    throw 'VB-CABLE endpoint verification failed.'
}

Write-Host ''
Write-Host 'Starting the numbered router configuration wizard.'
Write-Host 'Choose the real microphone(s) as inputs; never choose CABLE Output as an input.'
Write-Host 'The wizard will recommend CABLE Input as the router output.'
& $RouterExe --init-config
if ($LASTEXITCODE -ne 0) {
    throw "Router configuration failed with exit code $LASTEXITCODE."
}

Write-Host ''
Write-Host 'VB-CABLE routing is configured.' -ForegroundColor Green
Write-Host 'Set CABLE Output (VB-Audio Virtual Cable) as the microphone in Windows or the target application.'
Write-Host 'Do not change the Windows playback default; leave speakers/headphones as the playback device.'
Write-Host "Start the router with: `"$RouterExe`""

if ($OpenSoundSettings) {
    Start-Process 'ms-settings:sound'
}
