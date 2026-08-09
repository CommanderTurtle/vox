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

For non-desktop consumers, the same repository contains the optional
`http-router-only` Cargo member. It exposes the composed private API without
tray, hotkey, clipboard, capture, GUI, or device dependencies. Desktop Vox and
the HTTP member share `crates/vox-local-core`, so Crisper request parsing and
LongCat sentence-aware concatenation cannot drift between them.

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

To expose those already-running backends to LAN applications without starting
desktop Vox:

```bash
cd ~/multimedia/vox/http-router-only
./setup
./starthttp.sh
```

The gateway never starts, loads, unloads, or stops a model. Its portable
configuration is created beside its binary at
`http-router-only/target/release/config.toml`; see
[`http-router-only/README.md`](../http-router-only/README.md) for the complete
route contract.

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
concatenate_chunks = true
characters_per_request = 240

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

The tray always exposes `LongCat Voice Pair`. Every saved pair is a basic
checkmarked item; when no pairs exist, it points the user to Settings. The
active pair is also switchable with the configurable `tts_voice_switch`
hotkey (`Alt+Shift+T` by default). `LongCat Seed (<current>)` provides direct
`− 1` and `+ 1` actions. `tts_seed_increment` (`Alt+Shift+S`) advances the
persisted seed, while `tts_seed_decrement` (`Alt+Shift+A`) lowers it. Each
change rebuilds only the TTS adapters and is
available to the next synthesis without restarting Vox. The old
`prompt_audio_path` + inline `prompt_text` fields remain a fallback only when
no named profiles exist.

When `concatenate_chunks` is enabled, `characters_per_request` selects a
literal character position. Vox scans backward from that position to the
previous `.`, `?`, `!`, ellipsis, or equivalent full-width terminator, sends
that complete sentence group once, and continues through the final sentence.
It never cuts in the middle of a sentence. If LongCat actually rejects a
multi-sentence request, Vox moves its final sentence back to the remaining
text and retries the shorter prefix; successful WAVs are never regenerated.
Every successful PCM WAV is rebuilt into one valid result in order.

When `concatenate_chunks` is disabled, none of this splitting or retry logic
runs: the complete text is sent exactly once and Vox returns the single WAV
from that request directly.

## Synthesized-audio destinations

All TTS-producing flows—text, translated text, speech-to-TTS, and translated
speech-to-TTS—share one output selection:

```toml
[tts.output]
# playback | clipboard_wav | mic_forwarder
mode = "playback"
mic_forwarder_url = "http://127.0.0.1:8182"
```

- `playback` preserves the original speaker behavior.
- `clipboard_wav` writes `vox-output.wav` beside `vox.exe` and publishes it
  to the Windows clipboard as a pasteable file (`CF_HDROP`), not path text.
- `mic_forwarder` posts a normalized WAV to the native router.

## Native microphone router

The router is an opt-in Cargo target; the normal release command still builds
only `vox.exe`:

```powershell
cargo build --release --features mic-forwarder --bin vox-mic-forwarder
.\target\release\vox-mic-forwarder.exe --list-devices
.\target\release\vox-mic-forwarder.exe --verify-cable
.\target\release\vox-mic-forwarder.exe --init-config
```

`--verify-cable` fails unless the same CPAL/WASAPI enumerator used by the router
sees both VB-CABLE endpoints, then prints their negotiated formats.
`--init-config` is an interactive, numbered device wizard. It saves exact
device names into `mic-forwarder.toml` beside the router executable and does
not require hand-written TOML. It recommends **CABLE Input (VB-Audio Virtual
Cable)** as the router output, rejects **CABLE Output (VB-Audio Virtual
Cable)** as a physical input, and separately asks for the real playback device
used by the system-audio subtitle tap.

If `--list-devices` shows only ordinary speakers/headphones, Windows has no
virtual microphone path available yet. The Rust router cannot create a Windows
audio driver from user space. Download and install the current VB-CABLE package
from the official VB-Audio page, reboot Windows, then run the repository's
post-install helper. Selecting headphones or speakers is useful only for
monitoring/testing and does not create a selectable microphone. Stereo Mix is
also not a substitute: it is a capture/loopback endpoint rather than a writable
playback sink.

```powershell
.\windows-vb-cable\setup.ps1 -OpenSoundSettings
```

The generated file is equivalent to:

```toml
bind = "127.0.0.1:8182"
output_device = "CABLE Input (VB-Audio Virtual Cable)"
system_audio_enabled = true
system_audio_device = "default"
system_audio_sample_rate = 16000
injected_gain = 1.0
queue_seconds = 300

[[inputs]]
name = "default"
gain = 1.0

[[inputs]]
name = "Second microphone name"
gain = 1.0
```

The router captures both inputs continuously, mixes them with WAV, MP3, or
M4A media posted to
`POST /v1/forward`, and renders the result to `output_device`. For a virtual
microphone, this must be the playback side of an installed virtual cable;
choose that cable's capture side as the Windows/application microphone. The
configured pair is **CABLE Input** and **CABLE Output**, respectively. The
physical mic remains an ordinary, live input to the mix.

Running the router normally opens its small native control window. Paste or
**Browse…** to any `.wav`, `.mp3`, or `.m4a`, then choose **Hoist to
microphone**. Use `--headless` when only the local HTTP boundary is wanted.
`POST /v1/playback` sends media to the current Windows default playback
device. The router opens every configured endpoint in its native Windows
shared-mode format; it does no codec recompression. It downmixes the microphone
bus and uses stateful linear interpolation only when endpoint rates differ.
The physical microphone's driver flags and hardware DSP are not copied to the
virtual driver; those remain owned by Windows. Consequently the Hands-Free
profile Windows exposes is the practical quality ceiling for AirPods and other
Bluetooth microphones. Vox preserves those captured samples without a second
codec stage, but cannot recreate bandwidth absent from the Bluetooth source.
Restart it after changing Windows' default input/output devices.

The independent WASAPI loopback tap captures `system_audio_device` for
captions and never enters the routed microphone mix. A second tap captures the
physical microphone mix before generated audio is injected. Health and device
data are available at `/health` and `/v1/devices`. Both taps maintain bounded
history with a cursor per consumer, so any number of desktop and HTTP caption
clients can coexist without draining one another's audio windows.

## Independent live caption lanes

Start `vox-mic-forwarder`, CrisperWhisper, and optionally the local translator.
Then use the tray's **Live Captions** submenu:

- **Physical microphone mix**: original-language or translated captions.
- **System playback**: original-language or translated captions.
- **System playback → translated captions + LongCat dub**: translated text is spoken
  to the Windows default output. The router clears loopback audio captured
  during playback so the dub cannot recursively transcribe itself.

Every native borderless overlay has a visible drag bar and close button, stays
always on top, and also closes with Escape. Its rolling-window and per-source
language settings live in executable-local `config.toml`:

```toml
[subtitles]
router_url = "http://127.0.0.1:8182"
chunk_seconds = 1.5
font_size = 30.0
max_lines = 3
microphone_language = "detect"
system_language = "detect"
target_language = "English"
```

The router keeps only the newest bounded audio window for each inference pass,
preventing slow ASR/translation from accumulating a stale queue. The overlay
detects the native display cadence on launch, enables Windows' 1 ms timer
resolution, and uses DWM-flushed, GPU-backed, vsynced presentation (4.17 ms on
a 240 Hz panel). It remains DPI-aware through native egui/winit and does not
use WinUI animation. The router,
CrisperWhisper model, translator, and LongCat service remain warm between
windows. Legitimate multiline translations are preserved while narrow model
presentation wrappers and Markdown fences are discarded.

Choose a fixed `microphone_language` / `system_language` whenever it is known.
That is the direct, one-pass Crisper path. `detect` is deliberately optional
and resolves every audio window independently; it never changes either saved
setting or latches the previous result.

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
max_tokens = 512
source_language = "auto"
target_language = "English"
active_route = "inbound"
system_prompt = "Translate faithfully into the requested target language. Return only the translation. Preserve names, numbers, formatting, profanity, and tone."

[translate.inbound]
source_language = "auto"
target_language = "English"

[translate.outbound]
source_language = "English"
target_language = "Spanish"
```

The source fields remain in TOML for compatibility, but the local EraX service
does not require them. EraX detects its own input language and only needs the
selected target. The language token produced by the optional audio MITM belongs
exclusively to the final full Crisper pass.

The tray selects the active target route and offers common outbound languages;
Settings also permits any free-form language name. Vox exposes the complete
desktop flow matrix without duplicating its battle-tested capture/playback
paths:

| Route | Input | Pipeline | Output |
|---:|---|---|---|
| 1 | microphone/system sound | Crisper STT | text |
| 2 | microphone/system sound | Crisper STT → EraX target | text |
| 3 | microphone/system sound | Crisper STT → LongCat | audio |
| 4 | microphone/system sound | Crisper STT → EraX target → LongCat | audio |
| 5 | selected/clipboard text | EraX target | text |
| 6 | selected/clipboard text | EraX target → LongCat | audio |
| 7 | selected/clipboard text | LongCat | audio |

Dedicated hotkeys select raw versus translated TTS. The normal record and TTS
hotkeys keep the existing `translate.asr` and `translate.tts` switches for
users who prefer translation to be their default path.

All seven compositions are first-class desktop actions. In particular,
`Alt+Ctrl+R` is the dedicated speech → active translation route → injected
text action; it does not depend on the ordinary record hotkey's `translate.asr`
preference. Translation is always visible in the tray, including on first run,
where it can be enabled and its active route/target selected. Settings opens on
the dedicated Translation & flows page instead of burying the backend below
the ASR and TTS forms.

When `model` is blank, Vox selects the first model advertised by the
configured `/models` route and caches that choice for the process lifetime.
The bounded completion length keeps speech translations brief. Source speech
is delimited and explicitly treated as untrusted data.
If translation fails, Vox logs the failure and safely continues with the
original transcript/text rather than losing the user's input.

### Standalone translation runtime

`~/multimedia/translate` loads the local EraX Translator V1.0 Q6_K GGUF with
llama.cpp, the XLM-R classifier on CPU, and the INT4 EraX-VL runtime on CPU.
Ordinary translation calls use the tiny dedicated translator first. The VL
runtime is used by the optional spoken-language MITM and as a narrow fallback
when the dedicated translator returns an unchanged or wrong-language result.

For `detect` audio only, Crisper produces all supported language rows in one
low-token decoder batch. XLM-R cheaply narrows that inventory. EraX-VL ranks
complete, grammatical rows using punctuation/coherence evidence. An acoustic
prior above 0.90 resolves obvious cases immediately; otherwise only the small
finalist set is translated and compared. The selected ISO token drives one
full-quality Crisper pass. None of this runs for a fixed-language route.

The service does not launch ComfyUI, an agent, a diffusion model, or a cloud
client. It has its own process and GPU lifecycle and consumes none of Agents
A1's vLLM sequences. The model is eager-loaded by default; `/load` and
`/unload` provide explicit operator lifecycle control.

## Focused CLI paths

These commands exercise the same adapters without opening the tray UI:

```powershell
cargo run -- transcribe .\sample.wav
cargo run -- translate "buenos días"
cargo run -- tts "A short line for the configured LongCat voice." .\voice.wav
```

The model-backed acceptance script exercises all seven compositions and writes
four inspectable WAVs plus `report.json`:

```bash
python scripts/route_acceptance.py \
  --audio /path/to/spoken-language.wav \
  --expected-language zh \
  --target English \
  --reference-audio ~/multimedia/longcat/assets/prompt.wav \
  --reference-text '小偷却一点也不气馁，继续在抽屉里翻找。'
```

## Current boundary

This integration covers Vox itself: capture, CrisperWhisper STT, optional
translation, LongCat TTS, playback, injection, configuration, and CLI access.
AgentOS, Wan Streamer Replication, and the Even Realities application are
intentionally deferred. They can consume these stable local service seams
later without binding the desktop client to any one frontend or agent stack.
