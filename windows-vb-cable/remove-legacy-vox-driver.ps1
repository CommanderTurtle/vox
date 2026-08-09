[CmdletBinding()]
param(
    [switch]$RestoreCodeIntegrityPolicy
)

$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal]::new(
    [Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this migration cleanup from an elevated Windows PowerShell.'
}

$HardwareId = 'Root\Sysvad_ComponentizedAudioSample'
$devices = @(Get-CimInstance -ClassName Win32_PnPEntity |
    Where-Object { @($_.HardwareID) -contains $HardwareId })
$driverPackages = [Collections.Generic.HashSet[string]]::new(
    [StringComparer]::OrdinalIgnoreCase)

foreach ($device in $devices) {
    $provider = (Get-PnpDeviceProperty -InstanceId $device.PNPDeviceID `
        -KeyName 'DEVPKEY_Device_DriverProvider' -ErrorAction Stop).Data
    if ($provider -ne 'Vox') {
        throw "Refusing to remove $($device.PNPDeviceID): provider is '$provider', not 'Vox'."
    }
    $inf = (Get-PnpDeviceProperty -InstanceId $device.PNPDeviceID `
        -KeyName 'DEVPKEY_Device_DriverInfPath' -ErrorAction Stop).Data
    if ($inf -match '^oem\d+\.inf$') {
        [void]$driverPackages.Add($inf)
    }

    Write-Host "Removing legacy device $($device.PNPDeviceID)..."
    & pnputil.exe /remove-device $device.PNPDeviceID
    if ($LASTEXITCODE -notin @(0, 3010)) {
        throw "PnPUtil device removal failed with exit code $LASTEXITCODE."
    }
}

foreach ($inf in $driverPackages) {
    Write-Host "Removing legacy driver package $inf..."
    & pnputil.exe /delete-driver $inf /uninstall /force
    if ($LASTEXITCODE -notin @(0, 3010)) {
        throw "PnPUtil package removal failed with exit code $LASTEXITCODE."
    }
}

if ($devices.Count -eq 0) {
    Write-Host 'No installed legacy Vox SysVAD device was found.'
}

if ($RestoreCodeIntegrityPolicy) {
    $policy = 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy'
    if (Get-ItemProperty -LiteralPath $policy -Name UpgradedSystem -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -LiteralPath $policy -Name UpgradedSystem
        Write-Host 'Removed the obsolete UpgradedSystem compatibility flag. Reboot Windows to apply the restored policy.'
    }
    else {
        Write-Host 'The UpgradedSystem compatibility flag is already absent.'
    }
}

Write-Host 'Legacy Vox driver cleanup complete. VB-CABLE itself was not changed.'
