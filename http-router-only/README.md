# Vox HTTP router

`http-router-only` is Vox's optional backend-only Cargo member. It preserves the shared local CrisperWhisper and LongCat behavior while removing every desktop concern: no tray, hotkeys, clipboard, audio capture, GUI, model process control, or application lifecycle.

The gateway composes three separately operated private services:

```text
CrisperWhisper :8172 -> speech-to-text (intended or literal/verbatim)
Local translate :8176 -> inbound auto-to-English or outbound English-to-selected
LongCat :8230         -> multilingual voice-cloned speech with sentence-aware requests
Vox router :8182      -> system loopback, virtual mic hoist, default playback
```

This makes the same pipeline usable by Even Realities, native JavaScript, mobile apps, or any LAN client without embedding the Windows/Linux Vox application.

## Contract

- Backends are started and stopped only from their own terminal sessions.
- `vox-http` never calls backend `/load` or `/unload` routes and never starts a process.
- Configured backend URLs must resolve syntactically to loopback, RFC1918 IPv4, IPv6 ULA/link-local, or a `.local` hostname. Public hosts are rejected at startup.
- There is no cloud fallback and no telemetry integration.
- LongCat reference audio (`.wav`, `.mp3`, or `.m4a`) and verbatim UTF-8 `.txt` transcripts remain server-local. Clients select only a profile name.
- TTS requests may override the configured seed without mutating server state. This is the HTTP equivalent of Vox's tray seed controls.

## Build and start

Rust is native here; there is deliberately no Python venv.

```bash
cd ~/multimedia/vox/http-router-only
./setup
./starthttp.sh
# first run creates target/release/config.toml; edit it there as needed
```

`Ctrl+C` stops only this gateway. It does not unload or stop CrisperWhisper, the translation service, or LongCat.

Configuration is portable and build-local: unless `VOX_HTTP_CONFIG` explicitly
points elsewhere, the gateway creates and reads `config.toml` beside the
running binary (`target/release/config.toml` for the normal build). It does not
use a platform settings directory, temporary directory, or caller working
directory.

The member shares `../crates/vox-local-core` with desktop Vox. Crisper request
parsing, LongCat sentence selection, backend-failure retreat, and WAV merging
therefore cannot drift between the two programs. Axum routes, cached subtitle
polling, private-network validation, translation proxying, and router proxying
remain deliberately local to this member.

## Routes

| Route | Purpose |
|---|---|
| `GET /health` | Read-only aggregate health for the three backends |
| `GET /v1/routes` | Language defaults, public voice names, default seed, and TTS limits |
| `POST /v1/translate` | Translate text using a named route or explicit source/target |
| `POST /v1/transcribe` | Multipart audio to intended/literal text, optionally translated |
| `POST /v1/speak` | Text (optionally translated) to a merged WAV |
| `POST /v1/transcribe-and-speak` | Multipart audio through STT, optional translation, and TTS |
| `POST /v1/hoist` | Multipart WAV/MP3/M4A to the configured microphone router (VB-CABLE on Windows) |
| `POST /v1/playback` | Multipart WAV/MP3/M4A to Windows' current default playback device |
| `POST /v1/audio/transcribe` | Bounded microphone or system window through STT and optional translation |
| `POST /v1/audio/dub` | Microphone or system audio through STT, translation, LongCat, then playback/mic/WAV |
| `POST /v1/audio/clear` | Explicitly discard one source's router buffer |
| `GET/POST/DELETE /v1/audio/stream` | Cached start/status/stop worker for either source |
| `POST /v1/system-audio/transcribe` | Bounded latest system-audio window through STT and optional translation |
| `POST /v1/system-audio/dub` | System audio through STT, translation, LongCat, then playback/mic/WAV |
| `POST /v1/system-audio/clear` | Explicitly discard the router loopback buffer |
| `POST /v1/system-audio/stream` | Start/replace one background caption worker |
| `GET /v1/system-audio/stream` | Immediate cached status; preserves prior text while the next chunk runs |
| `DELETE /v1/system-audio/stream` | Stop the worker while retaining its final cached snapshot |

Inference routes require `Authorization: Bearer ...` only when `server.api_key` is configured.

When `longcat.concatenate_chunks=true`, `characters_per_request` is a target,
not a hard split point. Vox walks backward to the preceding sentence ending;
if none exists it walks forward to the next ending, and it never cuts an
unpunctuated sentence. A backend failure retreats one additional complete
sentence and retries. The successful WAVs are concatenated in order. With
concatenation disabled, the entire text is sent as one request and returned as
one WAV.

### Translate

```bash
curl http://alien.local:8180/v1/translate \
  -H 'Content-Type: application/json' \
  -d '{"text":"buenos días","route":"inbound"}'
```

`inbound` defaults to detected language -> English. `outbound` defaults to English -> the configured selected language. Every translation-capable endpoint accepts optional `source_language` and `target_language` overrides, so `auto`/selected source and English/selected target are available at every composition layer:

```bash
curl http://alien.local:8180/v1/translate \
  -H 'Content-Type: application/json' \
  -d '{"text":"buongiorno","route":"inbound","source_language":"Italian","target_language":"English"}'
```

### Transcribe

```bash
curl http://alien.local:8180/v1/transcribe \
  -F file=@recording.m4a \
  -F mode=intended \
  -F translate=true \
  -F route=inbound \
  -F source_language=auto \
  -F target_language=English
```

Use `mode=literal` for word-faithful transcription; the gateway maps it to CrisperWhisper's canonical `verbatim` operation.

### Speak

```bash
curl http://alien.local:8180/v1/speak \
  -H 'Content-Type: application/json' \
  -d '{"text":"Good morning","translate":true,"route":"outbound","source_language":"English","target_language":"Spanish","voice":"Ana","seed":1025}' \
  -o speech.wav
```

### Transcribe and speak

```bash
curl http://alien.local:8180/v1/transcribe-and-speak \
  -F file=@recording.wav \
  -F mode=intended \
  -F translate=true \
  -F route=outbound \
  -F source_language=English \
  -F target_language=Spanish \
  -F voice=Ana \
  -F seed=1025 \
  -o reply.wav
```

Every TTS response returns `X-Vox-Seed`. The combined route also returns `X-Vox-Transcript-B64` and `X-Vox-Output-Text-B64`, so a thin client can recover both UTF-8 strings without a second response format. Clients can implement increment/decrement controls locally by reading the default from `/v1/routes`, changing it, and sending `seed` with the next request; the gateway remains stateless.

### Hoist, subtitles, and live dubbing

```bash
curl http://alien.local:8180/v1/hoist -F file=@line.m4a

curl http://alien.local:8180/v1/system-audio/transcribe \
  -H 'Content-Type: application/json' \
  -d '{"seconds":1.5,"mode":"intended","translate":true,"route":"inbound"}'

curl http://alien.local:8180/v1/audio/transcribe \
  -H 'Content-Type: application/json' \
  -d '{"source":"microphone","seconds":1.5,"language":"detect","translate":false}'

curl http://alien.local:8180/v1/system-audio/dub \
  -H 'Content-Type: application/json' \
  -d '{"seconds":1.5,"translate":true,"route":"inbound","voice":"Ana","output":"playback"}'
```

The generic audio routes accept `source=system` or `source=microphone`; the
older `/v1/system-audio/*` names remain compatibility aliases. `output` may
be `playback`, `mic`, or `wav`. Desktop Vox and `vox-http` may
run simultaneously: neither binds the other's port, and both are ordinary
clients of the one router on `8182`. Desktop and HTTP subtitle consumers have
independent cursors rather than competing for a destructive queue. The router clears system-loopback audio
after default-device playback so a dub cannot feed back into itself.

For high-frequency lightweight clients, start the inference worker once and
poll its cached state independently:

```bash
curl -X POST http://alien.local:8180/v1/system-audio/stream \
  -H 'Content-Type: application/json' \
  -d '{"seconds":1.5,"translate":true,"route":"inbound"}'
curl http://alien.local:8180/v1/system-audio/stream
curl -X DELETE http://alien.local:8180/v1/system-audio/stream
```

Every GET is only a lock-protected memory read. It never starts inference,
drains audio, or blanks the previous caption. `processing` shows that the next
chunk is underway and `revision` increments only when a new complete caption
replaces the cached one, so 60/120/240 Hz pollers remain cheap and deterministic.

See [ARCHITECTURE.md](ARCHITECTURE.md) for boundaries and the seven supported flows.
