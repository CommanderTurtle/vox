# Legacy native Vox cable quick start

This is the continuation point for Vox's first-party, open-source alternative to VB-CABLE. At commit `6e27451`, the repository contains the native Windows audio cable, its signing scripts, and the working Vox microphone router.

The driver exposes:

- `Vox Cable Input` as the playback endpoint.
- `Vox Cable Output` as the recording endpoint selected by other applications.

This is a development-driver workflow. It works with Secure Boot disabled and Windows test-signing enabled; it is not a production driver-distribution path.

## 1. Clone the exact driver revision twice

Use a remote containing commit `6e27451de01eb778b2402dc70604ce6758ea0003`.

From an Administrator-backed WSL session:

```bash
git clone https://github.com/CommanderTurtle/vox.git ~/multimedia/vox-native-cable
cd ~/multimedia/vox-native-cable
git checkout --detach 6e27451de01eb778b2402dc70604ce6758ea0003
```

From ordinary Windows PowerShell:

```powershell
git clone https://github.com/CommanderTurtle/vox.git C:\source\vox-native-cable
Set-Location C:\source\vox-native-cable
git checkout --detach 6e27451de01eb778b2402dc70604ce6758ea0003
```

## 2. Refresh and build the driver in ordinary PowerShell

Run `-Refresh` first. It fetches the pinned Microsoft SysVAD source, applies the Vox patch, discovers the Visual Studio/WDK chain, and stages the driver package.

```powershell
Set-Location C:\source\vox-native-cable
.\windows-native-audio-cable\build.ps1 -Refresh
```

## 3. Prepare the signing identity from Administrator-backed WSL

```bash
cd ~/multimedia/vox-native-cable
./prepare-windows-driver-signing.sh
cat keys/windows-thumbprint.txt
```

The script creates or reuses the ignored `keys/` bundle, imports the certificate and private key into the Windows machine store, and grants the TrustedInstaller service identity access to that exact private-key container. Save the printed thumbprint for installation.

## 4. Enable the Windows development-driver environment

After disabling Secure Boot in firmware, run from elevated Windows PowerShell and reboot:

```powershell
bcdedit.exe -set TESTSIGNING ON
Restart-Computer
```

Microsoft references: [test-signing boot configuration](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/the-testsigning-boot-configuration-option) and [test-signing driver packages](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/test-signing-driver-packages).

## 5. Elevate to TrustedInstaller

Resource 1: https://www.youtube.com/watch?v=Vj1uh89v-Sc&t=1556 (25:56)
Resource 2: https://www.reddit.com/r/CurseForge/comments/tvg1c6/comment/n1bbttp/
Resource 3: https://www.virustotal.com/gui/file/467639dfc6a2d61ed7bc4d61549abf60ac23cc89fd6d6da1c68c2c7befdcb604/detection

# Code, in elevated (run-as-administrator) powershell:

# Step 1
Add-MpPreference -ExclusionProcess "powershell.exe"
# Step 2
Install-Module -Name NtObjectManager -RequiredVersion 1.1.32
# Step 3
Remove-MpPreference -ExclusionProcess "powershell.exe"

# Required
Import-Module NtObjectManager

# Step 4:
Restart-Service TrustedInstaller
$p = Get-NtProcess -Name TrustedInstaller.exe
$p
$th = $p.GetFirstThread()

$current = Get-NtThread -Current -PseudoHandle
$imp = $current.ImpersonateThread($th)
$imp_token = Get-NtToken -Impersonation
$imp_token

# Check at any time:
$imp_token.Groups

---------------------------------other cool stuff:

# (option) Launch Command-Prompt as NT-Authority as your user:
$p = Get-NtProcess -Name TrustedInstaller.exe
$proc = New-Win32Process cmd.exe -CreationFlags NewConsole -ParentProcess $p

# (option) Test Check:
$proc.Process.User

```

Before installing, confirm that the shell is either TrustedInstaller or SYSTEM with the TrustedInstaller SID enabled:

```powershell
$imp_token.Groups
```

Historical sources: [`windows-native-audio-cable/README.md`](https://github.com/CommanderTurtle/vox/blob/6e27451de01eb778b2402dc70604ce6758ea0003/windows-native-audio-cable/README.md) and [`prepare-windows-driver-signing.sh`](https://github.com/CommanderTurtle/vox/blob/6e27451de01eb778b2402dc70604ce6758ea0003/prepare-windows-driver-signing.sh).

## 6. Sign and install from the TrustedInstaller shell

Paste the thumbprint printed by the WSL preparation script:

```powershell
Set-Location C:\source\vox-native-cable
$Thumbprint = '<paste keys/windows-thumbprint.txt here>'

.\windows-native-audio-cable\install.ps1 `
  -CertificateThumbprint $Thumbprint
```

Reboot if the installer returns Windows restart code `3010`.

## 7. Build and initialize the router

Back in ordinary Windows PowerShell:

```powershell
Set-Location C:\source\vox-native-cable
cargo build --release --features mic-forwarder --bin vox-mic-forwarder

.\target\release\vox-mic-forwarder.exe --verify-cable
.\target\release\vox-mic-forwarder.exe --init-config
```

Select physical microphones as router inputs and `Vox Cable Input` as its output. Applications receive the combined physical-microphone and Vox-generated audio through `Vox Cable Output`.

Trusted driver is installed. If secure boot is enabled, there are a few optional steps:

1. Enter Custom-Mode on BIOS to enable editing PK/KEK/DB/DBX
2. Be aware that this will prevent windows TPM/TPM-WMI from trusting writing to db's in security updates
2. Understand this only works with secure boot disabled only elsewise

## Verify -- expected: `Status: Started`, with Code 52 (untrusted signer) absent

```powershell
pnputil /enum-devices /instanceid 'ROOT\DEVGEN\{5590C1E3-7FF3-C843-A4E6-01184D6F0B5F}'
```
