# vox

Voice I/O companion for CLI AI agents — system-tray app providing global ASR (speech-to-text) and TTS (text-to-speech) via pluggable cloud/local engines.

## Project

- **Stack**: Rust 2021 edition, tokio async, crossbeam channels, tray-icon, cpal (audio), rdev (hotkeys), egui (settings UI)
- **Entry**: `src/main.rs` — initializes config, engines, tray, hotkeys, then enters crossbeam::select! event loop
- **Config**: TOML at platform config dir (`com.vox/vox/`); `defaults.toml` embedded via `include_str!`
- **Default engines**: local ASR (`whisper-cpp` HTTP server, with `openai`-compatible fallback) + free **Edge TTS** (no API key). Mimo/Aliyun cloud engines register only when keys are configured. **Doubao** (Volcano Engine) ASR/TTS register and become the default when `asr.doubao.api_key` is set; the key is shared between ASR + TTS and must come from the Doubao Speech console (not Ark).
- **Doubao (Volcano Engine)**: set `[asr.doubao].api_key` (Doubao Speech console key, shared with TTS) to auto-register `doubao-asr` (`volc.seedasr.sauc.duration`, WebSocket `bigmodel_nostream`) + `doubao-tts` (`seed-tts-2.0`, HTTP NDJSON streaming) and make them the default primary engines. TTS requests PCM and is wrapped to WAV via the shared `playback::pcm_to_wav`.

## Commands

| Action | Command |
|--------|---------|
| Build | `cargo build` |
| Run | `cargo run` |
| Release | `cargo build --release` |
| Test | `cargo test` |
| Debug logging | `RUST_LOG=debug cargo run` |
| Build with local whisper | `cargo build --features whisper-local` |
| CLI: transcribe file | `cargo run -- transcribe <file.wav>` |
| CLI: inject text | `cargo run -- inject <text> [--mode keyboard\|clipboard]` |

## Architecture

```
main.rs (event loop: crossbeam::select! tray events × hotkey events)
├── app/         — state.rs (AppState: Idle/Recording/Transcribing; RecordMode: PushToTalk/Toggle)
│                  hotkey.rs (HotkeyBinding parser, rdev listener; emits Pressed+Released for PTT)
├── asr/         — mod.rs (AsrEngine trait + AsrManager + fallback chain)
│                  whisper_cpp.rs (local whisper.cpp HTTP server, /inference)
│                  mimo_asr.rs (Mimo chat completions API, input_audio data URL)
│                  openai_asr.rs (OpenAI-compatible /audio/transcriptions, multipart; base_url configurable for localhost)
│                  aliyun_asr.rs (nls-gateway REST, raw PCM body)
│                  doubao_asr.rs (Volcano Doubao ASR 2.0, WebSocket binary-frame protocol, bigmodel_nostream endpoint)
│                  whisper_local.rs (whisper.cpp FFI, feature-gated)
├── audio/       — capture.rs (cpal microphone, 16kHz mono i16 PCM, bg thread)
│                  utils.rs (WAV encode, RMS level, duration helpers)
├── inject/      — mod.rs (InjectMode + inject_text dispatcher)
│                  keyboard.rs (enigo keyboard simulation)
│                  clipboard.rs (arboard + enigo Ctrl+V, save/restore)
│                  text_reader.rs (read selected text via Ctrl+C simulation)
├── tts/         — mod.rs (TtsEngine trait + TtsManager + TtsInputMode enum)
│                  edge_tts.rs (free Microsoft Edge TTS over WebSocket, no API key; requests MP3, decoded in-process by rodio)
│                  mimo_tts.rs (Mimo chat completions, base64 PCM → WAV)
│                  doubao_tts.rs (Volcano Doubao TTS 2.0, HTTP NDJSON streaming; requests PCM → WAV)
│                  playback.rs (temp WAV → system player: powershell/afplay/aplay)
├── config/      — mod.rs (ConfigManager: load/save TOML, RwLock, serde(default) for backwards compat)
├── tray/        — mod.rs (dedicated thread with Windows message pump, tray-icon + menu)
├── settings/    — mod.rs (egui/eframe window, spawned in own thread)
├── scripts/     — build-whisper / install-service / install-autostart / package
```

### Key patterns

- **Engine plugin**: `AsrEngine` / `TtsEngine` traits (`#[async_trait]`, `Send + Sync`), registered into `AsrManager`/`TtsManager` at startup
- **Cross-thread**: app uses `crossbeam::channel` (unbounded) for the main event loop (tray events × hotkey events × ASR results × settings saves); audio capture runs on a dedicated thread with an `AtomicBool` stop flag
- **Non-blocking main loop**: ASR/TTS run as tasks on a shared `tokio::runtime::Runtime` (`AppCtx.runtime`); results come back via the `asr_result_rx` channel. The main loop never `block_on`s — hotkeys/tray stay responsive
- **Engine managers are `Arc`-shared**: `AsrManager`/`TtsManager` store engines in a `Vec` (registration order preserved) and keep `active` behind a `RwLock`, so an `Arc<Manager>` can be cloned into background tasks
- **Tray thread**: dedicated thread + `PeekMessageW` loop on Windows. Menu + tooltip are rebuilt from a plain-data `MenuModel` on `TrayCommand::RefreshMenu` — main thread pushes a fresh model on any state change
- **Menu IDs**: namespaced string IDs (`asr:<name>`, `inject:<mode>`, `recordmode:<mode>`, `tts:<name>`, `ttsinput:<mode>`, `toggle`, `settings`, `quit`) → resolved in `map_event`
- **Record mode**: `RecordMode` (PushToTalk/Toggle) in `AppCtx`; the hotkey listener already emits both `RecordTogglePressed` and `RecordToggleReleased`. `handle_hotkey_event` dispatches: PTT → `start_recording` on press, `stop_recording` on release; Toggle → `toggle_recording` on press, ignore release. Switchable live via tray menu / settings (`general.record_mode`)
- **Settings decoupling**: the egui window holds a `Config` snapshot and sends the edited snapshot back via channel on save; it never touches disk/`toml`. `apply_settings` (main thread) persists + rebuilds engines + refreshes tray, all live
- **Clipboard safety**: `ClipboardSnapshot` saves/restores text *or* image around Ctrl+C/V simulation, so non-text clipboard contents aren't destroyed
- **Edge TTS**: WebSocket to `speech.platform.bing.com`; `Sec-MS-GEC` = uppercased SHA-256 of (Unix secs rounded down to 5 min → Windows file-time ticks + trusted token). Requires `MUID` cookie + `chrome-extension://…` Origin. Returns MP3 → decoded & played in-process by `rodio`
- **rustls**: `rustls::crypto::ring::default_provider().install_default()` is called once at startup (tokio-tungstenite needs a CryptoProvider)

## Conventions

- **Naming**: types `PascalCase`; functions/vars `snake_case`; modules named after their `mod.rs` parent
- **Error handling**: `thiserror` for domain errors (`AsrError`, `InjectError`, `TtsError`); `Box<dyn Error>` only at top-level main fallback
- **Async**: `#[async_trait]` for engine traits; the shared runtime spawns tasks; CLI subcommands build a one-shot `Runtime::new().block_on()`
- **Testing**: inline `#[cfg(test)] mod tests` with `#[test]`; `tempfile` for config roundtrip tests; hand-written mock engines (no `mockall`)
- **Config backwards compat**: new config fields/sections use `#[serde(default)]` so old files don't break
- **Feature gates**: `whisper-local` feature (whisper-rs FFI, requires libclang); `cloud-asr` feature is default but cloud engines register only when keys are present

## Notes

- **Default engines**: local ASR (`whisper-cpp`, fallback `openai`) + free Edge TTS — out-of-box testable with no cloud keys
- **whisper.cpp server**: start `./whisper-server -m ggml-tiny.bin --port 8080`; vox POSTs WAV to `/inference`
- **OpenAI ASR as local**: set `[asr.openai].base_url` to a localhost OpenAI-compatible server (faster-whisper, LocalAI)
