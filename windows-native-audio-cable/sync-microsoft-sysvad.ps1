[CmdletBinding()]
param(
    [switch]$Refresh
)

$ErrorActionPreference = 'Stop'
$SysvadCommit = '26a27df80772dbcfd69e6449b671d5c29eb5aedc'
$MicrosoftRepository = 'https://github.com/microsoft/Windows-driver-samples.git'
$Root = $PSScriptRoot
$WorkRoot = Join-Path $Root '.work'
$SourceRoot = Join-Path $WorkRoot 'windows-driver-samples'
$PatchPath = Join-Path $Root 'patches\vox-native-cable.patch'

function Invoke-Git {
    param([string[]]$GitArguments)
    & git @GitArguments
    if ($LASTEXITCODE -ne 0) {
        throw "git failed: git $($GitArguments -join ' ')"
    }
}

if ($Refresh -and (Test-Path -LiteralPath $SourceRoot)) {
    $resolvedWork = [IO.Path]::GetFullPath($WorkRoot)
    $resolvedSource = [IO.Path]::GetFullPath($SourceRoot)
    if (-not $resolvedSource.StartsWith($resolvedWork, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to refresh an unexpected path: $resolvedSource"
    }
    Remove-Item -LiteralPath $SourceRoot -Recurse -Force
}

if (Test-Path -LiteralPath (Join-Path $SourceRoot 'audio\sysvad\EndpointsCommon\voxcable.cpp')) {
    Write-Host "Vox native cable source is already prepared at $SourceRoot"
    exit 0
}

New-Item -ItemType Directory -Force -Path $WorkRoot | Out-Null
if (-not (Test-Path -LiteralPath (Join-Path $SourceRoot '.git'))) {
    Invoke-Git @('clone', '--filter=blob:none', '--no-checkout', $MicrosoftRepository, $SourceRoot)
}

Invoke-Git @('-C', $SourceRoot, 'fetch', '--depth', '1', 'origin', $SysvadCommit)
Invoke-Git @('-C', $SourceRoot, 'sparse-checkout', 'init', '--cone')
Invoke-Git @('-C', $SourceRoot, 'sparse-checkout', 'set', 'audio/sysvad')
Invoke-Git @('-C', $SourceRoot, 'checkout', '--detach', $SysvadCommit)
Invoke-Git @('-C', $SourceRoot, 'submodule', 'update', '--init', '--depth', '1', 'wil')

Invoke-Git @('-C', $SourceRoot, 'apply', '--check', $PatchPath)
Invoke-Git @('-C', $SourceRoot, 'apply', $PatchPath)

# Keep the official Microsoft base INF, but expose only the two Vox endpoints
# and replace sample-facing names. The source file is UTF-16LE.
$InxPath = Join-Path $SourceRoot 'audio\sysvad\TabletAudioSample\ComponentizedAudioSample.inx'
$Encoding = [Text.Encoding]::Unicode
$Inx = [IO.File]::ReadAllText($InxPath, $Encoding)

$InterfaceSection = @'
[SYSVAD_SA.NT.Interfaces]
AddInterface=%KSCATEGORY_AUDIO%, %KSNAME_WaveSpeaker%, SYSVAD.I.WaveSpeaker
AddInterface=%KSCATEGORY_RENDER%, %KSNAME_WaveSpeaker%, SYSVAD.I.WaveSpeaker
AddInterface=%KSCATEGORY_REALTIME%, %KSNAME_WaveSpeaker%, SYSVAD.I.WaveSpeaker
AddInterface=%KSCATEGORY_AUDIO%, %KSNAME_TopologySpeaker%, SYSVAD.I.TopologySpeaker
AddInterface=%KSCATEGORY_TOPOLOGY%, %KSNAME_TopologySpeaker%, SYSVAD.I.TopologySpeaker

AddInterface=%KSCATEGORY_AUDIO%, %KSNAME_WaveMicIn%, SYSVAD.I.WaveMicIn
AddInterface=%KSCATEGORY_REALTIME%, %KSNAME_WaveMicIn%, SYSVAD.I.WaveMicIn
AddInterface=%KSCATEGORY_CAPTURE%, %KSNAME_WaveMicIn%, SYSVAD.I.WaveMicIn
AddInterface=%KSCATEGORY_AUDIO%, %KSNAME_TopologyMicIn%, SYSVAD.I.TopologyMicIn
AddInterface=%KSCATEGORY_TOPOLOGY%, %KSNAME_TopologyMicIn%, SYSVAD.I.TopologyMicIn

'@

$SectionPattern = '(?ms)^\[SYSVAD_SA\.NT\.Interfaces\]\r?\n.*?(?=^\[SYSVAD_SA\.NT\.Services\])'
if (-not [regex]::IsMatch($Inx, $SectionPattern)) {
    throw 'The pinned Microsoft INF interface section was not found.'
}
$Inx = [regex]::Replace($Inx, $SectionPattern, $InterfaceSection)

$Replacements = [ordered]@{
    'ProviderName = "TODO-Set-Provider"' = 'ProviderName = "Vox"'
    'MfgName      = "TODO-Set-Manufacturer"' = 'MfgName      = "Vox"'
    'MsCopyRight  = "TODO-Set-Copyright"' = 'MsCopyRight  = "Microsoft sample, adapted by Vox"'
    'SYSVAD_SA.DeviceDesc="Virtual Audio Device (WDM) - Tablet Sample"' = 'SYSVAD_SA.DeviceDesc="Vox Native Audio Cable"'
    'SYSVAD_ComponentizedAudioSample.SvcDesc="Virtual Audio Device (WDM) - Tablet Sample Driver"' = 'SYSVAD_ComponentizedAudioSample.SvcDesc="Vox Native Audio Cable Driver"'
    'SYSVAD.WaveSpeaker.szPname="SYSVAD Wave Speaker"' = 'SYSVAD.WaveSpeaker.szPname="Vox Cable Input"'
    'SYSVAD.TopologySpeaker.szPname="SYSVAD Topology Speaker"' = 'SYSVAD.TopologySpeaker.szPname="Vox Cable Input"'
    'SYSVAD.WaveMicIn.szPname="SYSVAD Wave Microphone Headphone"' = 'SYSVAD.WaveMicIn.szPname="Vox Cable Output"'
    'SYSVAD.TopologyMicIn.szPname="SYSVAD Topology Microphone Headphone"' = 'SYSVAD.TopologyMicIn.szPname="Vox Cable Output"'
    'MicInCustomName= "External Microphone Headphone"' = 'MicInCustomName= "Vox Cable Output"'
}

foreach ($Entry in $Replacements.GetEnumerator()) {
    if (-not $Inx.Contains($Entry.Key)) {
        throw "The pinned Microsoft INF changed unexpectedly: missing '$($Entry.Key)'"
    }
    $Inx = $Inx.Replace($Entry.Key, $Entry.Value)
}

[IO.File]::WriteAllText($InxPath, $Inx, $Encoding)
Write-Host "Prepared Microsoft SysVAD $SysvadCommit with the Vox native cable overlay."
