# Vox quick start

Vox uses the maintained VB-CABLE Windows driver as one narrow transport. Vox
itself remains the mixer/router: it combines the real microphone with generated
audio and writes that mix to VB-CABLE. No custom driver, certificate, test boot,
TrustedInstaller workflow, or firmware change is required.

## 1. Start the private model services

Run each service in its own WSL terminal:

```bash
cd ~/multimedia/whisper && ./starthttp.sh   # CrisperWhisper: 8172
cd ~/multimedia/translate && ./starthttp.sh # EraX/XLM/EraX-VL: 8176
cd ~/multimedia/longcat && ./starthttp.sh   # LongCat: 8230
```

The model services are local-only implementations with no cloud fallback. The
Windows Vox app connects to the WSL/LAN addresses configured in its settings.

## 2. Build the two Windows programs

From ordinary Windows PowerShell:

```powershell
Set-Location C:\path\to\vox
cargo build --release --features mic-forwarder --bins
```

This produces:

- `target\release\vox.exe` — tray app, hotkeys, captions, STT/translation/TTS.
- `target\release\vox-mic-forwarder.exe` — physical-mic/generated-audio mixer.

## 3. Install and wire VB-CABLE

Install VB-CABLE from its [official download](https://vb-audio.com/Cable), then
run:

```powershell
.\windows-vb-cable\setup.ps1 -OpenSoundSettings
```

In the numbered wizard select:

1. **Router input:** the real microphone, such as `Headset (AirPods)`.
2. **Router output:** `CABLE Input (VB-Audio Virtual Cable)`.
3. In Discord, a browser, or another receiving app, choose
   `CABLE Output (VB-Audio Virtual Cable)` as that app's microphone.

Leave the normal Windows playback default on headphones/speakers. System audio
is not part of the microphone mix unless a separately selected caption lane
captures it.

## 4. Run

```powershell
.\target\release\vox-mic-forwarder.exe
.\target\release\vox.exe
```

The router window shows physical-input and injected-audio activity. Vox can
play TTS to speakers, publish a pasteable WAV, or hoist it into the router.

Open **Settings → Routes** to edit the seven ready-made workflows. Every row
has its own input, Crisper language, optional translation target, output, and
hotkey; changes apply immediately after Save. The same routes also appear
under the tray's **Programmable Routes** menu. Caption rows are concurrent,
so German→English system captions can stay open beside native English system
captions and microphone captions.

## Language behavior

- Choose a known Crisper language for the fastest path: one full transcription.
- Choose `detect` only when the language is genuinely unknown. Each utterance
  runs the optional parallel Crisper → XLM/EraX-VL MITM, then one full Crisper
  pass with the selected token. The result never becomes sticky state.
- EraX translation is independently multilingual. It requires a target
  language, not the Crisper source token.

See [Local Backends](docs/LOCAL_BACKENDS.md) for the complete seven-route
matrix, live-caption lanes, service lifecycle, and acceptance command.
