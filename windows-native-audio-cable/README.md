# Vox native Windows audio cable

This optional component gives Vox a real Windows playback-to-recording bridge without installing a third-party virtual-cable product.

- **Vox Cable Input** is the Windows playback endpoint into which Vox renders its mixed microphone and generated audio.
- **Vox Cable Output** is the Windows recording endpoint selected by chat, meeting, recording, or streaming applications.

The source is generated reproducibly from Microsoft's official SysVAD sample at pinned commit `26a27df80772dbcfd69e6449b671d5c29eb5aedc`. The only upstream dependency is Microsoft's WIL submodule at the revision pinned by that repository. No third-party audio driver or service is used.

## What the adaptation changes

Microsoft SysVAD normally saves render data and synthesizes capture data. The Vox overlay replaces that demonstration behavior with a 100 ms nonpaged jitter ring:

```text
Vox / Windows app
       |
       v
Vox Cable Input (48 kHz, stereo, signed 16-bit PCM)
       |
       v
bounded kernel ring -- overflow drops oldest / underrun emits silence
       |
       v
Vox Cable Output (48 kHz, stereo, signed 16-bit PCM)
       |
       v
chat, recorder, browser, or streaming application
```

Windows Audio Engine performs application-side sample-rate and channel conversion. Both driver endpoints expose one identical format, preventing mismatched raw frames inside the bridge. Shared-mode applications are mixed by Windows before reaching the single driver render stream.

## Build

Requirements are all Microsoft-native:

1. Git for obtaining Microsoft's source.
2. Visual Studio with Desktop C++ tooling.
3. A matching Windows SDK and Windows Driver Kit (WDK), including WDK Tools.

From ordinary PowerShell:

```powershell
cd windows-native-audio-cable
.\build.ps1
```

Run this from a Windows-local clone or copy of the Vox repository (for example,
under `C:\Users\<you>\source\vox`), not from a WSL shell or UNC-mounted WSL
path. WDK, Visual Studio, MSBuild, and driver installation are native Windows
operations.

The script clones only Microsoft's driver-sample source, checks out the pinned
revision, applies the reviewed Vox overlay, builds the official solution with
`SignMode=Off`, and copies the unsigned package to
`dist\windows-native-audio-cable`. Building never creates a certificate or
touches a certificate store.

Use `build.ps1 -Refresh` only when you intentionally want to discard and regenerate the private `.work` tree. It never touches the Vox source tree outside this component.

## Package-specific test signing

Do not disable driver-signature enforcement. Do not give a normal user a
driver-signing key. This component instead uses a one-operation machine signer:

1. `sign-package.ps1` must run in **PowerShell 7.1+ as Administrator**.
2. It creates a self-signed code-signing certificate under
   `Cert:\LocalMachine\My`. Its private key is non-exportable and its ACL grants
   access only to `SYSTEM` and the local `Administrators` group.
3. It signs only the package's `.cat` catalog. That catalog contains hashes of
   the INF, SYS, DLL, and other package files; changing any covered file
   invalidates the package signature.
4. It exports only the public `.cer` and trusts that public certificate under
   `LocalMachine\Root` and `LocalMachine\TrustedPublisher`.
5. In a `finally` block it removes the `LocalMachine\My` certificate with
   `-DeleteKey`. The private key is destroyed even if signing fails. No PFX is
   written anywhere, and no user certificate store is used.

After a build, sign the catalog from an elevated PowerShell 7 terminal:

```powershell
pwsh
.\sign-package.ps1
```

The final message confirms that `Cert:\LocalMachine\My\<thumbprint>` no longer
exists. The remaining public certificate can verify this exact catalog but
cannot sign anything.

### Isolated Windows test boot

A locally self-signed kernel driver still requires Windows Test Mode. Test Mode
does **not** turn signature enforcement off: Windows continues to require a
digital signature, but accepts a test signature. To keep that policy out of the
ordinary Windows boot, create a separate, non-default boot-loader entry:

```powershell
# Elevated PowerShell; run once
.\create-test-boot-entry.ps1

# Select that entry for the next boot only
.\select-test-boot-once.ps1
Restart-Computer
```

The creation script copies the current boot entry, applies `testsigning on` to
the copy only, and leaves the default entry unchanged. `select-test-boot-once`
uses BCDEdit's one-time boot sequence; after the test session, the next restart
returns to the ordinary entry automatically.

Secure Boot may reject creation of a TESTSIGNING entry. The script then deletes
the partial copy and changes nothing else. If Secure Boot must remain enabled
and Test Mode is unacceptable, there is no local self-signing bypass: submit
the package for Microsoft attestation/WHQL signing.

Once Windows is running the **Vox Driver Test** entry, install from an elevated
PowerShell:

```powershell
.\install-signed.ps1
```

Memory Integrity/HVCI may remain enabled because the package is test-signed;
an unsigned image is never loaded. To remove the extra boot entry later, boot
the ordinary Windows entry and run:

```powershell
.\remove-test-boot-entry.ps1
```

Do not install SysVAD's sample APO or extension INF; Vox needs only
`ComponentizedAudioSample.inf`.

To remove the device later:

```powershell
.\uninstall.ps1
```

The uninstall script deliberately leaves the separately managed test boot
entry and public verification certificate unchanged.

## Connect Vox

After Windows exposes both endpoints, rebuild the existing Vox utilities normally and run:

```powershell
.\vox-mic-forwarder.exe --init-config
```

Choose:

- your physical microphone as the input;
- **Vox Cable Input** as the forwarder's output.

Then select **Vox Cable Output** as the microphone in the destination application. Stereo Mix is neither required nor used: it is a capture/loopback endpoint, not a writable virtual microphone sink.

## Maintenance boundary

`patches\vox-native-cable.patch` is intentionally pinned to one audited Microsoft commit. If Microsoft changes SysVAD, update the pinned commit and review the patch deliberately; do not silently float the driver source to `main`.
