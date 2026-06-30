# vox

Voice I/O companion for CLI AI agents — system-tray app providing global ASR (speech-to-text) and TTS (text-to-speech) via pluggable cloud/local engines.

## Project

- **Stack**: Rust 2021 edition, tokio async, crossbeam channels, tray-icon, cpal (audio), rdev (hotkeys), egui (settings UI)
- **Entry**: `src/main.rs` — initializes config, engines, tray, hotkeys, then enters crossbeam::select! event loop
- **Config**: TOML at platform config dir (`com.vox/vox/`); `defaults.toml` embedded via `include_str!`
- **Default engines**: local ASR (`whisper-cpp` HTTP server, with `openai`-compatible fallback) + free **Edge TTS** (no API key). Mimo/Aliyun cloud engines register only when keys are configured.

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
├── app/         — state.rs (AppState: Idle/Recording/Transcribing)
│                  hotkey.rs (HotkeyBinding parser, rdev listener thread)
├── asr/         — mod.rs (AsrEngine trait + AsrManager + fallback chain)
│                  whisper_cpp.rs (local whisper.cpp HTTP server, /inference)
│                  mimo_asr.rs (Mimo chat completions API, input_audio data URL)
│                  openai_asr.rs (OpenAI-compatible /audio/transcriptions, multipart; base_url configurable for localhost)
│                  aliyun_asr.rs (nls-gateway REST, raw PCM body)
│                  whisper_local.rs (whisper.cpp FFI, feature-gated)
├── audio/       — capture.rs (cpal microphone, 16kHz mono i16 PCM, bg thread)
│                  utils.rs (WAV encode, RMS level, duration helpers)
├── inject/      — mod.rs (InjectMode + inject_text dispatcher)
│                  keyboard.rs (enigo keyboard simulation)
│                  clipboard.rs (arboard + enigo Ctrl+V, save/restore)
│                  text_reader.rs (read selected text via Ctrl+C simulation)
├── tts/         — mod.rs (TtsEngine trait + TtsManager + TtsInputMode enum)
│                  edge_tts.rs (free Microsoft Edge TTS over WebSocket, no API key; requests WAV)
│                  mimo_tts.rs (Mimo chat completions, base64 PCM → WAV)
│                  playback.rs (temp WAV → system player: powershell/afplay/aplay)
├── config/      — mod.rs (ConfigManager: load/save TOML, RwLock, serde(default) for backwards compat)
├── tray/        — mod.rs (dedicated thread with Windows message pump, tray-icon + menu)
├── settings/    — mod.rs (egui/eframe window, spawned in own thread)
├── scripts/     — build-whisper / install-service / install-autostart / package
```

### Key patterns

- **Engine plugin**: `AsrEngine` / `TtsEngine` traits (`#[async_trait]`, `Send + Sync`), registered into `AsrManager`/`TtsManager` at startup
- **Cross-thread**: app uses `crossbeam::channel` (unbounded) for main event loop; audio capture runs on dedicated thread with `AtomicBool` stop flag
- **Tray thread**: dedicated thread + `PeekMessageW` loop on Windows (necessary for right-click to work)
- **Menu IDs**: tray menu items use string IDs (`"quit"`, `"tts_mimo-tts"`, `"tts_input_selection"`) → parsed in event listener

## Conventions

- **Naming**: types `PascalCase`; functions/vars `snake_case`; modules named after their `mod.rs` parent
- **Error handling**: `thiserror` for domain errors (`AsrError`, `InjectError`, `TtsError`); `Box<dyn Error>` only at top-level main fallback
- **Async**: `#[async_trait]` for engine traits; `tokio::runtime::Runtime::new().block_on()` for one-shot calls from sync context (main loop is sync)
- **Testing**: inline `#[cfg(test)] mod tests` with `#[test]`; `mockall` for trait mocking; `tempfile` for config roundtrip tests
- **Dead code**: intentionally preserved API surfaces marked `#[allow(dead_code)]` (e.g. level meters, sample_rate getter, unused engine variants)
- **Config backwards compat**: new config fields use `#[serde(default)]` so old files don't break
- **Feature gates**: `whisper-local` feature (requires libclang to build); `cloud-asr` feature is default

## Notes

*(Add project-specific notes here as they arise.)*
