[CmdletBinding()]
param(
    [switch]$ToolsOnly
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$kitVersion = '10.0.28000.0'
$vsConfigurationUri = 'https://raw.githubusercontent.com/microsoft/Windows-driver-samples/main/_wdk_utils/winget/configs/wdk-desktop.vsconfig'
$sdkInstallerUri = 'https://download.microsoft.com/download/06fc99ac-527e-451e-a536-8866695a2e7e/KIT_BUNDLE_WINDOWSSDK_MEDIACREATION/winsdksetup.exe'
$wdkInstallerUri = 'https://download.microsoft.com/download/eb29e759-d7ad-4541-b60c-e9774ad8c593/KIT_BUNDLE_WDK_MEDIACREATION/wdksetup.exe'

function Test-Administrator {
    ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Test-WdkReady {
    param([switch]$ToolsOnly)

    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
    $required = @(
        (Join-Path $kits "Tools\$kitVersion\x64\devcon.exe"),
        (Join-Path $kits "Tools\$kitVersion\x64\devgen.exe"),
        (Join-Path $kits "bin\$kitVersion\x64\signtool.exe")
    )
    if (-not $ToolsOnly) {
        $required += @(
            (Join-Path $kits "Include\$kitVersion"),
            (Join-Path $kits "build\$kitVersion\WindowsDriver.Common.targets")
        )
    }
    if (@($required | Where-Object { -not (Test-Path -LiteralPath $_) }).Count -gt 0) {
        return $false
    }

    if ($ToolsOnly) { return $true }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { return $false }
    $driverVs = & $vswhere -latest -products * -requires Component.Microsoft.Windows.DriverKit -property installationPath
    -not [string]::IsNullOrWhiteSpace(($driverVs | Select-Object -First 1))
}

function Install-MicrosoftKit {
    param(
        [Parameter(Mandatory)] [string]$Name,
        [Parameter(Mandatory)] [string]$Uri,
        [Parameter(Mandatory)] [string]$Destination
    )

    if (-not (Test-Path -LiteralPath $Destination)) {
        Write-Host "Downloading the official Microsoft $Name installer..."
        Invoke-WebRequest -Uri $Uri -OutFile $Destination -UseBasicParsing
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $Destination
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notlike '*Microsoft Corporation*') {
        throw "$Name installer signature is not a valid Microsoft Corporation signature. Refusing execution."
    }

    Write-Host "Installing or reconciling $Name..."
    $process = Start-Process -FilePath $Destination -ArgumentList @('/features', '+', '/quiet', '/norestart') -Wait -PassThru
    $result = $process.ExitCode
    if ($result -notin @(0, 3010)) {
        throw "$Name installation failed with exit code $result."
    }
    return $result -eq 3010
}

if (Test-WdkReady -ToolsOnly:$ToolsOnly) {
    $readyDescription = if ($ToolsOnly) { 'Windows driver installation tools' } else { 'Windows driver build environment' }
    Write-Host "$readyDescription $kitVersion is already ready."
    return
}

if (-not (Test-Administrator)) {
    $hostPath = (Get-Process -Id $PID).Path
    $quotedScript = '"' + $PSCommandPath.Replace('"', '\"') + '"'
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File $quotedScript"
    if ($ToolsOnly) { $arguments += ' -ToolsOnly' }
    $process = Start-Process -FilePath $hostPath -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "Elevated WDK prerequisite installation failed with exit code $($process.ExitCode)."
    }
    return
}

$logDirectory = Join-Path $env:ProgramData 'Vox'
New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
$logPath = Join-Path $logDirectory 'wdk-install.log'
Start-Transcript -Path $logPath -Force | Out-Null
try {
    if (Test-WdkReady -ToolsOnly:$ToolsOnly) {
        $readyDescription = if ($ToolsOnly) { 'Windows driver installation tools' } else { 'Windows driver build environment' }
        Write-Host "$readyDescription $kitVersion is already ready."
        return
    }

    $installerDirectory = Join-Path $env:ProgramData 'Vox\installers'
    New-Item -ItemType Directory -Path $installerDirectory -Force | Out-Null
    $restartRequired = Install-MicrosoftKit -Name 'Windows SDK 10.0.28000.2526' -Uri $sdkInstallerUri -Destination (Join-Path $installerDirectory 'winsdksetup-28000.2526.exe')
    $restartRequired = (Install-MicrosoftKit -Name 'Windows Driver Kit 10.0.28000.2526' -Uri $wdkInstallerUri -Destination (Join-Path $installerDirectory 'wdksetup-28000.2526.exe')) -or $restartRequired

    if (-not $ToolsOnly) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        $setup = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\setup.exe'
        if (-not (Test-Path -LiteralPath $vswhere) -or -not (Test-Path -LiteralPath $setup)) {
            throw 'Visual Studio Installer is missing after the SDK/WDK package installation.'
        }
        $vsPath = & $vswhere -latest -products * -property installationPath | Select-Object -First 1
        if ([string]::IsNullOrWhiteSpace($vsPath)) { throw 'No Visual Studio instance is available for WDK integration.' }

        $vsConfiguration = Join-Path $env:TEMP 'vox-wdk-desktop.vsconfig'
        Invoke-WebRequest -Uri $vsConfigurationUri -OutFile $vsConfiguration -UseBasicParsing
        Write-Host "Adding Microsoft's official driver-development components to $vsPath..."
        $arguments = @(
            'modify',
            '--installPath', ('"' + $vsPath + '"'),
            '--config', ('"' + $vsConfiguration + '"'),
            '--passive',
            '--norestart'
        )
        $vsProcess = Start-Process -FilePath $setup -ArgumentList $arguments -Wait -PassThru
        if ($vsProcess.ExitCode -notin @(0, 3010)) {
            throw "Visual Studio WDK component installation failed with exit code $($vsProcess.ExitCode)."
        }
        $restartRequired = ($vsProcess.ExitCode -eq 3010) -or $restartRequired
    }

    if (-not (Test-WdkReady -ToolsOnly:$ToolsOnly)) {
        $missingDescription = if ($ToolsOnly) {
            'signing or device-installation tools'
        }
        else {
            'build, signing, device-installation, or Visual Studio components'
        }
        throw "The official installer completed, but one or more WDK $kitVersion $missingDescription are still absent."
    }

    $installedDescription = if ($ToolsOnly) { 'driver installation tools' } else { 'driver build environment' }
    Write-Host "WDK $kitVersion $installedDescription is fully installed and verified."
    if ($restartRequired) {
        Write-Warning 'One installer requested a Windows restart. The WDK is present, but restart before the first driver build.'
    }
}
finally {
    Stop-Transcript | Out-Null
    Write-Host "WDK installation log: $logPath"
}
