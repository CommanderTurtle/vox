# VB-CABLE adapter

Vox does not ship or modify an audio driver. It mixes physical microphones and
generated audio in `vox-mic-forwarder`, then writes that mix to the playback
side of the separately installed VB-CABLE driver:

```text
physical microphone(s) ─┐
LongCat / WAV injection ├─ Vox mixer ─> CABLE Input ─> CABLE Output ─> applications
HTTP /v1/forward ───────┘

system playback ─> independent WASAPI loopback ─> subtitles/dubbing only
```

The system-playback tap remains isolated from the microphone bus. It is never
mixed into `CABLE Input`, so desktop audio cannot leak into the virtual
microphone unless the user explicitly selects a loopback device as a physical
input.

## One-time Windows setup

1. Download `VBCABLE_Driver_Pack45.zip` from the
   [official VB-Audio page](https://vb-audio.com/Cable/).
2. Extract the complete ZIP to a local Windows folder.
3. Run `VBCABLE_Setup_x64.exe` as Administrator and choose **Install Driver**.
4. Reboot Windows as required by the official installer.
5. From an ordinary Windows PowerShell in the Vox repository, run:

   ```powershell
   .\windows-vb-cable\setup.ps1 -OpenSoundSettings
   ```

The script builds the optional router if needed, verifies both endpoints using
the same CPAL/WASAPI enumeration used at runtime, and launches Vox's numbered
configuration wizard. It does not download, bundle, silently install, or
redistribute VB-CABLE.

Choose the real microphone—such as the AirPods headset input—as the router
input. The wizard recommends:

```text
Router output:       CABLE Input (VB-Audio Virtual Cable)
Application mic:     CABLE Output (VB-Audio Virtual Cable)
System playback:     existing speakers/headphones
Subtitle audio tap:  existing speakers/headphones
```

The exact endpoint names are stored in `mic-forwarder.toml` beside the router
executable. This prevents changing the Windows default microphone to
`CABLE Output` from accidentally turning the cable back into its own input.

## Trust and package boundary

The current official package is `VBCABLE_Driver_Pack45.zip` (October 2024,
Windows XP through Windows 11, x86/x64/Arm64). During the 2026-08-09 audit:

- Official ZIP SHA-256: `B950E39F01AF1D04EA623C8F6D8EB9B6EA5C477C637295FABF20631C85116BFB`
- `vbaudio_cable64_win10.cat`: valid Microsoft Windows Hardware Compatibility
  Publisher signature
- `VBCABLE_Setup_x64.exe` and `vbaudio_cable64_win10.sys`: valid Vincent Burel
  publisher signatures

Treat the hash as a record for Pack45, not a promise for future packages.
Always obtain updates from the official page and follow VB-Audio's license.

## Migrating from the retired Vox SysVAD experiment

The old self-signed driver is not needed. Remove its failed device/package from
an elevated Windows PowerShell with:

```powershell
.\windows-vb-cable\remove-legacy-vox-driver.ps1 -RestoreCodeIntegrityPolicy
```

`-RestoreCodeIntegrityPolicy` removes only the `UpgradedSystem` value added for
that retired experiment. The cleanup script refuses to remove a matching device
unless Windows reports its provider as `Vox`; it never changes VB-CABLE.

Reboot if PnPUtil requests it, then perform the one-time VB-CABLE setup above.
