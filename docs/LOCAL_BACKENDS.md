# Local Backends: Windows Vox + WSL Models

This fork keeps desktop interaction and model inference separate:

```text
Windows Vox
  microphone / global hotkeys / tray / cursor injection / playback
        |
        +-- HTTP --> CrisperWhisper 2.0 in WSL (intended or literal transcript)
        +-- HTTP --> standalone local translation service (optional)
        +-- HTTP --> LongCat in WSL (voice-cloned WAV)
```

Vox is the cross-platform client. Windows is the preferred daily target for
global Alt hotkeys, native mouse/cursor behavior, the tray, and playback. WSL
hosts the heavyweight model processes. Neither `~/multimedia/whisper` nor
`~/multimedia/longcat` is started, stopped, loaded, or unloaded by Vox.

## What you actually run

Start the existing services in separate WSL shells:

```bash
cd ~/multimedia/whisper
./starthttp.sh
```

Leave each command running in its own shell (or the workstation's existing
raw-terminal job runner). `starthttp.sh` activates that folder's private UV
environment and launches the project's existing WebUI/API server; it does not
replace or patch the WebUI. Run the project's setup script only when the folder has no
`.venv` or its declared dependencies intentionally changed.

```bash
cd ~/multimedia/longcat
./starthttp.sh
```

Prepare the translator once, then start it in its own WSL shell:

```bash
cd ~/multimedia/translate
./setupwithuv.sh
./starthttp.sh
```

Their current defaults are:

| Service | URL | Useful status route |
|---|---|---|
| CrisperWhisper | `http://alien.local:8172` | `/api/health` |
| LongCat | `http://alien.local:8230` | `/api/status` |
| Local translator | `http://alien.local:8176` | `/health` |

From the Windows Vox machine, verify the private-LAN route before launching
Vox:

```powershell
Invoke-RestMethod http://alien.local:8172/api/health
Invoke-RestMethod http://alien.local:8230/api/status
Invoke-RestMethod http://alien.local:8176/health
```

These checks do not change model state. All three services load eagerly by
default when their operator starts them, and each exposes explicit load/unload
routes. Their HTTP servers may remain running while GPU-heavy models are released:

```powershell
Invoke-RestMethod -Method Post http://alien.local:8172/api/unload
Invoke-RestMethod -Method Post http://alien.local:8230/api/unload
Invoke-RestMethod -Method Post http://alien.local:8176/unload
```

All three checked-in runtime environments set Hugging Face/Transformers
offline mode plus `HF_HUB_DISABLE_TELEMETRY=1` and `DO_NOT_TRACK=1`. No Vox
adapter has a cloud fallback. Binding to `0.0.0.0` makes the services visible
to the private LAN, so the Windows firewall/router should admit these ports
only from that trusted network.

Use `127.0.0.1` when Windows mirrored networking exposes WSL on localhost.
Use `alien.local` (or the host's private address) when running Vox from a
different LAN device. No adapter has a cloud fallback.

Build and run the desktop client from native Windows PowerShell:

```powershell
git clone https://github.com/CommanderTurtle/vox.git
Set-Location vox
cargo build --release
.\target\release\vox.exe
```

Open **Settings** from the Vox tray and save the following choices. Settings
apply live; Vox rebuilds its lightweight HTTP clients without restarting the
model servers. As in upstream Vox, edited global-hotkey strings are parsed on
the next Vox launch; tray/profile/route changes are immediate.

## CrisperWhisper speech input

Choose `crisper-whisper` as the primary ASR engine and set:

```toml
[asr]
primary_engine = "crisper-whisper"

[asr.crisper]
base_url = "http://alien.local:8172"
mode = "intended" # or "literal"; also switchable from the tray
language = "en"
chunk_duration = 30.0
stride = 26.0
context_words = 12
max_new_tokens = 256
hotwords = ""
```

Vox sends either `operation=intended` (cleaned/non-literal) or
`operation=verbatim` (word-faithful/literal) according to the tray/Settings selection.
The standalone service and WebUI remain unchanged.

## LongCat voice output

Choose `longcat` as the TTS engine and configure one or more proven voice
pairs in Settings. Each named pair is an audio file plus its exact UTF-8
`.txt` transcript:

```toml
[tts]
primary_engine = "longcat"
input_mode = "selection"

[tts.longcat]
base_url = "http://alien.local:8230"
active_voice_profile = "Ana"
steps = 16
guidance_strength = 4.0
guidance_method = "apg"
seed = 1024
duration_scale = 1.0
max_chunk_seconds = 20.0

[[tts.longcat.voice_profiles]]
name = "Ana"
audio_path = 'C:\voices\ana.m4a'
transcript_path = 'C:\voices\ana.txt'

[[tts.longcat.voice_profiles]]
name = "Studio voice"
audio_path = 'C:\voices\studio.wav'
transcript_path = 'C:\voices\studio.txt'
```

These paths are read by the Vox client, so a Windows Vox process uses Windows
paths. WAV, MP3, and M4A reference audio are accepted. Vox reads the selected
transcript at synthesis time and uploads that reference pair to the local
LongCat service for each segment; the service's established WebUI and lazy
load/unload behavior remain unchanged.

The active pair is switchable from the tray or the configurable
`tts_voice_switch` hotkey. The old `prompt_audio_path` + inline `prompt_text`
fields remain a fallback only when no named profiles exist.

Long text is split on sentence boundaries. Oversized sentences fall back to
word boundaries and CJK text falls back to character boundaries. Vox ports
LongCat's own duration estimate and adds a reference-conditioning margin, so
no request is budgeted above twenty seconds. Multiple PCM WAV responses are
decoded and rebuilt into one valid WAV before native playback.

## Local translation

Translation can independently affect ASR output before cursor injection and
TTS input before synthesis:

```toml
[translate]
enabled = true
asr = true
tts = false
base_url = "http://alien.local:8176/v1"
api_key = ""
model = ""
max_tokens = 256
source_language = "auto"
target_language = "English"
active_route = "inbound"
system_prompt = "You are an expert cross-lingual translator. Identify the source language and translate it naturally into the requested target language without losing meaning or nuance. Return only the translation: no labels, commentary, or explanation. Preserve names, numbers, formatting, profanity, and tone."

[translate.inbound]
source_language = "auto"
target_language = "English"

[translate.outbound]
source_language = "English"
target_language = "Spanish"
```

The tray selects the active direction and offers common outbound languages;
Settings also permits any free-form language name. Vox exposes the complete
desktop flow matrix without duplicating its battle-tested capture/playback
paths:

| Input | Optional stage | Output |
|---|---|---|
| Speech (STT) | none or active translation route | injected text |
| Selected/clipboard text | active translation route | injected text |
| Speech (STT) | none or active translation route | TTS |
| Selected/clipboard text | none or active translation route | TTS |

Dedicated hotkeys select raw versus translated TTS. The normal record and TTS
hotkeys keep the existing `translate.asr` and `translate.tts` switches for
users who prefer translation to be their default path.

When `model` is blank, Vox selects the first model advertised by the
configured `/models` route and caches that choice for the process lifetime.
The bounded completion length keeps short speech translations brief. Source
speech is delimited and explicitly treated as untrusted data. The literal
`(translate)` cue is kept immediately before the source text because it
stabilizes the Qwen checkpoint on mixed-script and noisy Unicode input.
If translation fails, Vox logs the failure and safely continues with the
original transcript/text rather than losing the user's input.

### Standalone translation runtime

`~/multimedia/translate` loads the existing
`qwen3_vl_4b_nvfp4_full.safetensors` directly. The checkpoint is a Comfy
`comfy_quant` Qwen3-VL model, and Comfy's Krea2 wrapper retains its tied output
projection and supplies text generation. The service duplicates the proven
`CLIPLoader(type=krea2) -> Generate Text` path: PyTorch attention, default
template enabled, thinking disabled, and the screenshot's fixed-seed sampling
settings (`0.7`, top-k `64`, top-p `0.95`, min-p `0.05`, repetition `1.05`).

It imports a sparse checkout of Comfy's inference core as a library. It does
not launch the Comfy application, workflow engine, queue, web server, frontend,
diffusion model, or VAE. Translation therefore has its own process and GPU
lifecycle and consumes none of Agents A1's vLLM sequences. The model is
eager-loaded by default; `/load` and `/unload` provide explicit operator lifecycle control.

## Focused CLI paths

These commands exercise the same adapters without opening the tray UI:

```powershell
cargo run -- transcribe .\sample.wav
cargo run -- translate "buenos días"
cargo run -- tts "A short line for the configured LongCat voice." .\voice.wav
```

## Current boundary

This integration covers Vox itself: capture, CrisperWhisper STT, optional
translation, LongCat TTS, playback, injection, configuration, and CLI access.
AgentOS, Wan Streamer Replication, and the Even Realities application are
intentionally deferred. They can consume these stable local service seams
later without binding the desktop client to any one frontend or agent stack.
