# Vox native Windows audio cable

This component builds the pinned Microsoft SysVAD adaptation already carried by
Vox. The package exposes one writable playback endpoint and one selectable
recording endpoint.

```text
Playback endpoint: Vox Cable Input
Recording endpoint: Vox Cable Output
Hardware ID: Root\Sysvad_ComponentizedAudioSample
```

The signing path is private-key gated. It uses one locally generated certificate
and grants TrustedInstaller access to that certificate's imported machine key.
The certificate represents only its own local identity.

## Requirements

```text
Windows identity for installation: NT SERVICE\TrustedInstaller
Visual Studio component: MSBuild
Windows Driver Kit components: build targets, SignTool, DevGen, DevCon
WSL commands: openssl, cp
Windows commands: certutil.exe, Import-PfxCertificate, icacls, signtool
```

## 1. Generate the certificate and private key in WSL

Run this in WSL. Replace the Windows username in the destination path.

```bash
mkdir -p ~/vox-signing
cd ~/vox-signing

openssl req -x509 -newkey rsa:3072 -sha256 -days 3650 -nodes \
  -keyout mycert.key \
  -out mycert.crt \
  -subj "/CN=Vox Native Audio Cable" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=codeSigning"

openssl pkcs12 -export \
  -out mycert.pfx \
  -inkey mycert.key \
  -in mycert.crt \
  -CSP "Microsoft Enhanced RSA and AES Cryptographic Provider" \
  -LMK

openssl x509 \
  -in mycert.crt \
  -outform DER \
  -out mycert.cer

mkdir -p /mnt/c/Users/<you>/Desktop/vox-signing
cp mycert.cer mycert.pfx /mnt/c/Users/<you>/Desktop/vox-signing/
```

The files have distinct roles.

```text
mycert.crt: public PEM certificate retained in WSL
mycert.cer: public DER certificate imported into Cert:\LocalMachine\Root
mycert.pfx: certificate and private key imported into Cert:\LocalMachine\My
mycert.key: original private key retained in WSL
```

## 2. Prepare Windows signing access from Administrator-backed WSL

WSL itself does not modify the Windows certificate stores. This command uses
WSL interop to invoke Windows CertUtil and Administrator PowerShell under the
Windows token that launched WSL.

```bash
cd ~/multimedia/vox
./prepare-windows-driver-signing.sh
cat keys/windows-thumbprint.txt
```

The preparation phase performs this exact sequence.

```text
1. certutil.exe imports the public CER into Cert:\LocalMachine\Root.
2. Import-PfxCertificate imports the certificate and private key into Cert:\LocalMachine\My.
3. The Code Signing EKU 1.3.6.1.5.5.7.3.3 is required.
4. The imported CSP key is resolved under C:\ProgramData\Microsoft\Crypto\RSA\MachineKeys.
5. icacls grants NT SERVICE\TrustedInstaller:F on that exact private-key file.
6. The prepared certificate thumbprint is written to keys/windows-thumbprint.txt.
```

The relevant Windows operations are:

```powershell
certutil.exe -addstore root C:\path\mycert.cer

Import-PfxCertificate `
  -FilePath C:\path\mycert.pfx `
  -CertStoreLocation Cert:\LocalMachine\My `
  -Password (Read-Host 'PFX password' -AsSecureString)

icacls "C:\ProgramData\Microsoft\Crypto\RSA\MachineKeys\<key-guid>" `
  /grant "NT SERVICE\TrustedInstaller":F
```

## 3. Build, sign, and install from TrustedInstaller PowerShell

The preparation phase is complete before TrustedInstaller starts. Confirm the
active identity, then provide the prepared thumbprint to the installer.

```powershell
whoami
# nt service\trustedinstaller

cd C:\path\to\vox
.\windows-native-audio-cable\install.ps1 `
  -CertificateThumbprint '<contents of keys/windows-thumbprint.txt>' `
  -Refresh
```

The TrustedInstaller phase does not import certificates or change the private
key ACL. It uses the already-prepared identity to build, sign, and install.

```powershell
Get-ChildItem Cert:\LocalMachine\My |
  Where-Object { $_.EnhancedKeyUsageList -match "Code Signing" } |
  Format-List Subject,Thumbprint

signtool sign /fd SHA256 /sm /s My /sha1 <thumbprint> C:\path\package.cat
```

## 4. Configure the Vox router

```powershell
.\target\release\vox-mic-forwarder.exe --verify-cable
.\target\release\vox-mic-forwarder.exe --init-config
```

## Uninstall the driver device

Run the uninstall command from an Administrator or TrustedInstaller PowerShell.
It removes the root device and leaves the certificate stores and private-key ACL
unchanged.

```powershell
.\windows-native-audio-cable\uninstall.ps1
```

## Internal build controls

The installer calls these internal scripts. They are not separate lifecycle
steps.

```powershell
.\windows-native-audio-cable\sync-microsoft-sysvad.ps1
.\windows-native-audio-cable\build.ps1
```

The source is pinned and patched reproducibly.

```text
Microsoft repository: https://github.com/microsoft/Windows-driver-samples.git
Pinned commit: 26a27df80772dbcfd69e6449b671d5c29eb5aedc
Patch: windows-native-audio-cable\patches\vox-native-cable.patch
Build output: windows-native-audio-cable\dist\windows-native-audio-cable
```
