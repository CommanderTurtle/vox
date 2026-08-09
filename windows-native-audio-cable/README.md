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

If the matching SDK, WDK build/tools, or Visual Studio DriverKit component is
missing, `build.ps1` runs `install-wdk-prerequisites.ps1`. That helper requests
UAC once, verifies the Authenticode signatures on Microsoft's official
installers, installs the machine-wide components, returns to the ordinary
PowerShell process, and resumes the build. It resolves Visual Studio and the
Windows Kits by their machine-wide installation paths; neither the standard
user nor the administrator account needs a custom developer `PATH`.

Run this from a Windows-local clone or copy of the Vox repository (for example,
under `C:\Users\<you>\source\vox`), not from a WSL shell or UNC-mounted WSL
path. WDK, Visual Studio, MSBuild, and driver installation are native Windows
operations.

The script clones only Microsoft's driver-sample source, checks out the pinned
revision, applies the reviewed Vox overlay, builds the official solution with
the native x64 MSBuild/WDK validation host and `SignMode=Off`, and copies the unsigned package to
`dist\windows-native-audio-cable`. Building never creates a certificate or
touches a certificate store.

Use `build.ps1 -Refresh` only when you intentionally want to discard and regenerate the private `.work` tree. It never touches the Vox source tree outside this component.

## Preferred: Microsoft-attested package

The WireGuard-style lifecycle is a pre-signed driver plus a UAC-elevated
installer. UAC does not sign or attest a driver; it only authorizes installation
of a package that already satisfies kernel policy. Windows Hello and TPM device
attestation likewise prove a user/device identity, not driver-publisher trust.

For the normal Secure Boot path with no Test Mode and normally no reboot:

1. Register for Microsoft's Hardware Developer Program and associate the
   required EV code-signing certificate with that account.
2. Build the Hardware Dev Center CAB:

   ```powershell
   .\prepare-attestation.ps1
   ```

   This rebuilds with `SignMode=Off`, gathers the INF, binaries, catalog, and
   matching PDB symbols, and creates
   `dist\attestation\VoxNativeAudioCable.cab`. Files live below the required
   `VoxCable` subfolder rather than at the CAB root.
3. Sign the submission CAB with the EV certificate owned by the administrator
   account or its hardware token:

   ```powershell
   .\sign-attestation-cab.ps1 -CertificateThumbprint '<EV thumbprint>'
   ```

   When launched by a standard user, the script requests UAC credentials and
   runs only the signing operation under the administrator account. It creates,
   imports, exports, and deletes no certificate. The EV key remains governed by
   its existing hardware-token/HSM provider.
4. Submit that CAB in Partner Center. Microsoft replaces the catalog with a
   Microsoft SHA-2 catalog and appends Microsoft signatures to the binaries.
5. Extract the returned signed package beneath `dist\attested`, then run from
   the normal account:

   ```powershell
   .\install-attested.ps1
   ```

   The installer is designed to be launched by an ordinary, non-administrator
   user. It requests UAC once and, when necessary, installs and verifies only
   the official Microsoft SDK/WDK installation tools before it continues; an
   end user does not need Visual Studio. It then verifies the catalog and every SYS against Windows
   kernel policy and creates the root audio device with the WDK's DevCon. It
   never changes a certificate store or boot policy. It does not request a
   reboot; if Windows exceptionally returns `3010`, it reports that fact
   instead of restarting automatically.

Microsoft's current attestation workflow is for test audiences and requires an
EV certificate plus Hardware Dashboard registration. For public retail
distribution, use WHCP/HLK certification. Neither an OEM UEFI CA, a Windows
Hello key, nor a successful device-health attestation event substitutes for
that publisher enrollment.

## Offline fallback: package-specific test signing

Do not disable driver-signature enforcement. Do not give a normal user a
driver-signing key. This component instead uses a one-operation machine signer:

### One-command developer installation

From a normal PowerShell 7 terminal, run:

```powershell
.\install-development.ps1
```

The script requests UAC elevation, so an ordinary non-administrator account is
prompted for the separate administrator account's credentials. It then:

1. queries the running kernel's Code Integrity flags, checks the current BCD
   entry, and enables `TESTSIGNING` only when it is absent;
2. if Test Signing is not active yet, stops before building and asks for one
   restart; rerunning the same command after that restart performs the only
   build;
3. builds through the installed Visual Studio/WDK;
4. signs the package with the ephemeral machine signer described below;
5. creates exactly one persistent root devnode (or updates the existing one),
   installs the driver, and verifies that Windows enumerated both endpoints;
6. destroys every Vox private signing key before returning.

If a previous run was interrupted after creating its signer, the next run
reuses that signer instead of creating a duplicate. After a successful run,
old Vox public certificates are removed and exactly one current public
verification certificate remains. The script never changes local-group
membership, never exports a PFX, and never reboots automatically.

The script verifies that BCDEdit actually stored the setting before it creates
the signer. It also reports the independent UEFI Secure Boot state. It never
builds a package that cannot load in the current kernel: if Test Mode was not
already active, it exits after configuring the boot entry and the post-restart
run builds, signs, installs, and verifies the endpoints.
Test Mode stays enabled because Windows must continue accepting the
self-signed driver on later boots. If you eventually disable it with
`bcdedit /set testsigning off`, this development driver will no longer load.
Secure Boot can reject the TESTSIGNING setting; when that happens, the script
stops before creating a signer.

### Signer lifecycle

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

The following lower-level sequence remains available when each phase needs to
be controlled separately:

```powershell
pwsh
.\sign-package.ps1
```

The final message confirms that `Cert:\LocalMachine\My\<thumbprint>` no longer
exists. The remaining public certificate can verify this exact catalog but
cannot sign anything.

### Optional isolated Windows test boot

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

Once Windows is running the **Vox Driver Test** entry, install from an ordinary
PowerShell. The script requests UAC and installs any missing official Microsoft
driver prerequisites before continuing:

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
.\vox-mic-forwarder.exe --verify-cable
.\vox-mic-forwarder.exe --init-config
```

`--verify-cable` uses the exact CPAL/WASAPI enumerator used at runtime and
prints the playback and recording formats Windows negotiated. The setup wizard
then recommends **Vox Cable Input** automatically, refuses to select **Vox
Cable Output** as its own physical input, and separately selects the real
speaker/headphone endpoint used for system-audio subtitles.

Choose:

- your physical microphone as the input;
- **Vox Cable Input** as the forwarder's output.

Then select **Vox Cable Output** as the microphone in the destination application. Stereo Mix is neither required nor used: it is a capture/loopback endpoint, not a writable virtual microphone sink.

The physical microphone and virtual cable are separate devices, so driver
flags are not copied from one to the other. Vox captures the physical endpoint
in Windows' native shared-mode format, converts samples to a mono microphone
bus with stateful linear interpolation only when rates differ, and writes that
bus to the 48 kHz cable without codec compression. Windows still owns any
hardware DSP and Bluetooth profile selection. In particular, an AirPods mic is
limited by the Hands-Free profile Windows exposes; Vox neither lowers that
source quality further through a codec nor claims to restore bandwidth that
Bluetooth did not provide.

## Maintenance boundary

`patches\vox-native-cable.patch` is intentionally pinned to one audited Microsoft commit. If Microsoft changes SysVAD, update the pinned commit and review the patch deliberately; do not silently float the driver source to `main`.
