//! vox — Global voice input/output booster for CLI AI agents.
//!
//! Runs as a system-tray application. See SPEC.md for the full specification.

mod app;
mod asr;
mod audio;
mod config;
mod inject;
mod settings;
mod tts;
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam::channel;

use app::hotkey::{HotkeyBinding, HotkeyEvent, start_hotkey_listener};
use asr::AsrManager;
use asr::whisper_local::WhisperLocalEngine;
use asr::mimo_asr::MimoAsrEngine;
use asr::openai_asr::OpenaiAsrEngine;
use asr::aliyun_asr::AliyunAsrEngine;
use audio::capture::AudioCapture;
use audio::utils::pcm_to_wav;
use config::ConfigManager;
use inject::{InjectMode, inject_text};
use app::state::AppState;
use tray::{TrayEvent, TrayCommand, spawn_tray, update_tray_state};
use tts::TtsManager;
use tts::TtsInputMode;
use tts::mimo_tts::MimoTtsEngine;
use tts::playback::play_wav_async;

/// Shared application state.
struct AppCtx {
    config_mgr: ConfigManager,
    asr_mgr: AsrManager,
    tts_mgr: TtsManager,
    tts_input_mode: tts::TtsInputMode,
    inject_mode: InjectMode,
    /// Sender for commands to the tray thread.
    tray_cmd_tx: channel::Sender<TrayCommand>,
    /// Current recording capture handle (None when idle).
    capture: Option<AudioCapture>,
    /// Path to config file (for settings window).
    config_path: std::path::PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("vox v{} starting...", env!("CARGO_PKG_VERSION"));

    // ── Load configuration ────────────────────────────────────────────────
    let config_mgr = ConfigManager::load_or_create()?;
    {
        let cfg = config_mgr.read();
        log::info!(
            "Config loaded from {:?} | engine={} | inject_mode={}",
            config_mgr.path(),
            cfg.asr.primary_engine,
            cfg.inject.mode,
        );
    }

    // ── Parse hotkey bindings ────────────────────────────────────────────
    let (record_binding, engine_switch_binding, inject_switch_binding, tts_binding) = {
        let cfg = config_mgr.read();
        let record = HotkeyBinding::parse(&cfg.hotkey.record_toggle)
            .expect("Invalid record_toggle hotkey config");
        let engine = HotkeyBinding::parse(&cfg.hotkey.engine_switch)
            .expect("Invalid engine_switch hotkey config");
        let inject = HotkeyBinding::parse(&cfg.hotkey.inject_mode_switch)
            .expect("Invalid inject_mode_switch hotkey config");
        let tts_str = if cfg.hotkey.tts_trigger.is_empty() {
            "Alt+T"
        } else {
            &cfg.hotkey.tts_trigger
        };
        let tts = HotkeyBinding::parse(tts_str)
            .expect("Invalid tts_trigger hotkey config");
        (record, engine, inject, tts)
    };

    // ── Initialize ASR manager ───────────────────────────────────────────
    let (primary_engine, fallback_engines) = {
        let cfg = config_mgr.read();
        (cfg.asr.primary_engine.clone(), cfg.asr.fallback_engines.clone())
    };

    let mut asr_mgr = AsrManager::new(primary_engine.clone(), fallback_engines);

    // Register whisper-local
    let _ = WhisperLocalEngine::new("");
    if let Ok(engine) = WhisperLocalEngine::new("") {
        asr_mgr.register(Box::new(engine));
    } else {
        log::warn!("whisper-local engine not available");
    }

    // Register Mimo ASR engine
    {
        let cfg = config_mgr.read();
        let mimo_cfg = &cfg.asr.mimo;
        if !mimo_cfg.api_key.is_empty() {
            log::info!("Registering Mimo ASR engine: {} at {}", mimo_cfg.model, mimo_cfg.base_url);
            let engine = MimoAsrEngine::new(&mimo_cfg.base_url, &mimo_cfg.api_key, &mimo_cfg.model);
            asr_mgr.register(Box::new(engine));
        } else {
            log::warn!("Mimo ASR engine skipped: no api_key configured");
        }
    }

    // Register OpenAI Whisper API engine
    {
        let cfg = config_mgr.read();
        let openai_cfg = &cfg.asr.openai;
        if !openai_cfg.api_key.is_empty() {
            log::info!("Registering OpenAI Whisper API engine");
            let engine = OpenaiAsrEngine::new(
                "https://api.openai.com/v1",
                &openai_cfg.api_key,
                &openai_cfg.model,
            );
            asr_mgr.register(Box::new(engine));
        } else {
            log::info!("OpenAI engine skipped: no api_key configured");
        }
    }

    // Register Aliyun ASR engine
    {
        let cfg = config_mgr.read();
        let aliyun_cfg = &cfg.asr.aliyun;
        if !aliyun_cfg.appkey.is_empty() && !aliyun_cfg.token.is_empty() {
            log::info!("Registering Aliyun ASR engine");
            let engine = AliyunAsrEngine::new(&aliyun_cfg.appkey, &aliyun_cfg.token);
            asr_mgr.register(Box::new(engine));
        } else {
            log::info!("Aliyun engine skipped: no appkey/token configured");
        }
    }

    // ── Initialize TTS manager ───────────────────────────────────────────
    let mut tts_mgr = TtsManager::new("mimo-tts".to_string());

    // Register Mimo TTS engine
    {
        let cfg = config_mgr.read();
        let mimo_cfg = &cfg.asr.mimo; // share base URL and API key with ASR
        let tts_cfg = &cfg.tts.mimo;
        if !mimo_cfg.api_key.is_empty() {
            log::info!("Registering Mimo TTS engine: {} at {}", tts_cfg.model, mimo_cfg.base_url);
            let engine = MimoTtsEngine::new(&mimo_cfg.base_url, &mimo_cfg.api_key, &tts_cfg.model);
            tts_mgr.register(Box::new(engine));
        } else {
            log::warn!("Mimo TTS engine skipped: no api_key configured");
        }
    }

    let inject_mode = InjectMode::from_config(&config_mgr.read());
    let tts_input_mode = TtsInputMode::from_config(&config_mgr.read());

    // ── Build system tray (dedicated thread with message pump) ───────────
    let (tray_event_sender, tray_event_receiver) = channel::unbounded::<TrayEvent>();
    let engines = asr_mgr.engine_names();
    let tts_engines = tts_mgr.engine_names();

    let (tray_cmd_tx, _tray_handle) = spawn_tray(&engines, &tts_engines, tray_event_sender.clone());

    // ── Start global hotkey listener ──────────────────────────────────────
    let (hotkey_sender, hotkey_receiver) = channel::unbounded::<HotkeyEvent>();
    let stop_flag = Arc::new(AtomicBool::new(false));

    start_hotkey_listener(
        record_binding,
        engine_switch_binding,
        inject_switch_binding,
        tts_binding,
        hotkey_sender.clone(),
        stop_flag.clone(),
    );

    log::info!("vox initialized. Waiting for events...");

    // ── Application context ──────────────────────────────────────────────
    let mut ctx = AppCtx {
        config_path: config_mgr.path().clone(),
        config_mgr,
        asr_mgr,
        tts_mgr,
        tts_input_mode,
        inject_mode,
        tray_cmd_tx,
        capture: None,
    };

    // ── Main event loop (NO Windows message pump needed here) ────────────
    loop {
        crossbeam::select! {
            recv(tray_event_receiver) -> msg => {
                if !handle_tray_event(&mut ctx, msg) {
                    stop_flag.store(true, Ordering::Relaxed);
                    // Signal tray thread to shut down
                    let _ = ctx.tray_cmd_tx.send(TrayCommand::Shutdown);
                    break;
                }
            }
            recv(hotkey_receiver) -> msg => {
                if let Ok(event) = msg {
                    handle_hotkey_event(&mut ctx, event);
                }
            }
        }
    }

    log::info!("vox shutdown complete.");
    Ok(())
}

/// Handle a tray event. Returns false if the app should quit.
fn handle_tray_event(ctx: &mut AppCtx, msg: Result<TrayEvent, crossbeam::channel::RecvError>) -> bool {
    match msg {
        Ok(TrayEvent::Quit) => {
            log::info!("Quit requested.");
            return false;
        }
        Ok(TrayEvent::OpenSettings) => {
            log::info!("Opening settings window...");
            let path = ctx.config_path.clone();
            settings::spawn_settings_window(&path);
        }
        Ok(TrayEvent::SetEngine(name)) => {
            log::info!("Switching engine to: {}", name);
            if let Err(e) = ctx.asr_mgr.set_active(&name) {
                log::error!("Failed to switch engine: {}", e);
            }
            let mut cfg = ctx.config_mgr.write();
            cfg.asr.primary_engine = name;
            drop(cfg);
            let _ = ctx.config_mgr.save();
        }
        Ok(TrayEvent::SetInjectMode(mode)) => {
            log::info!("Switching inject mode to: {}", mode);
            ctx.inject_mode = match mode.as_str() {
                "clipboard" => InjectMode::Clipboard,
                _ => InjectMode::Keyboard,
            };
            let mut cfg = ctx.config_mgr.write();
            cfg.inject.mode = mode;
            drop(cfg);
            let _ = ctx.config_mgr.save();
        }
        Ok(TrayEvent::SetTtsEngine(name)) => {
            log::info!("Switching TTS engine to: {}", name);
            if let Err(e) = ctx.tts_mgr.set_active(&name) {
                log::error!("Failed to switch TTS engine: {}", e);
            }
            let mut cfg = ctx.config_mgr.write();
            cfg.tts.primary_engine = name;
            drop(cfg);
            let _ = ctx.config_mgr.save();
        }
        Ok(TrayEvent::SetTtsInputMode(mode)) => {
            log::info!("Switching TTS input mode to: {}", mode);
            ctx.tts_input_mode = match mode.as_str() {
                "clipboard" => TtsInputMode::Clipboard,
                _ => TtsInputMode::Selection,
            };
            let mut cfg = ctx.config_mgr.write();
            cfg.tts.input_mode = mode;
            drop(cfg);
            let _ = ctx.config_mgr.save();
        }
        Ok(TrayEvent::ToggleRecording) => {
            toggle_recording(ctx);
        }
        Err(_) => return false,
    }
    true
}

/// Handle a hotkey event.
fn handle_hotkey_event(ctx: &mut AppCtx, event: HotkeyEvent) {
    match event {
        HotkeyEvent::RecordTogglePressed => {
            log::info!("[Hotkey] Record toggle pressed");
            toggle_recording(ctx);
        }
        HotkeyEvent::RecordToggleReleased => {}
        HotkeyEvent::EngineSwitch => {
            if let Some(name) = ctx.asr_mgr.cycle_engine() {
                log::info!("[Hotkey] Switched engine to: {}", name);
                let mut cfg = ctx.config_mgr.write();
                cfg.asr.primary_engine = name.to_string();
                drop(cfg);
                let _ = ctx.config_mgr.save();
            }
        }
        HotkeyEvent::InjectModeSwitch => {
            ctx.inject_mode = ctx.inject_mode.toggle();
            log::info!("[Hotkey] Switched inject mode to: {}", ctx.inject_mode.as_str());
            let mut cfg = ctx.config_mgr.write();
            cfg.inject.mode = ctx.inject_mode.as_str().to_string();
            drop(cfg);
            let _ = ctx.config_mgr.save();
        }
        HotkeyEvent::TtsTrigger => {
            log::info!("[Hotkey] TTS trigger");
            trigger_tts(ctx);
        }
    }
}

/// Trigger TTS: read text and speak it.
fn trigger_tts(ctx: &mut AppCtx) {
    let mode = ctx.tts_input_mode;
    log::info!("TTS input mode: {:?}", mode);

    // Get text according to selected mode
    let text = match mode {
        TtsInputMode::Selection => {
            match inject::text_reader::read_selected_text() {
                Ok(t) if !t.is_empty() => {
                    log::info!("Got {} chars from selection", t.len());
                    t
                }
                Ok(_) => {
                    log::error!("No text selected. Select text first or switch to Clipboard mode.");
                    return;
                }
                Err(e) => {
                    log::error!("Failed to read selection: {}", e);
                    return;
                }
            }
        }
        TtsInputMode::Clipboard => {
            match inject::text_reader::read_clipboard_text() {
                Ok(t) if !t.is_empty() => {
                    log::info!("Got {} chars from clipboard", t.len());
                    t
                }
                Ok(_) => {
                    log::error!("Clipboard is empty.");
                    return;
                }
                Err(e) => {
                    log::error!("Failed to read clipboard: {}", e);
                    return;
                }
            }
        }
    };

    log::info!("TTS synthesizing with engine '{}': {} chars", ctx.tts_mgr.active_engine(), text.len());

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to create runtime: {}", e);
            return;
        }
    };

    let result = rt.block_on(ctx.tts_mgr.synthesize(&text));
    match result {
        Ok(audio) => {
            log::info!("TTS synthesized {} bytes of audio", audio.len());
            // Play audio asynchronously
            play_wav_async(audio);
        }
        Err(e) => {
            log::error!("TTS synthesis failed: {}", e);
        }
    }
}

/// Toggle recording on/off.
fn toggle_recording(ctx: &mut AppCtx) {
    if ctx.capture.is_some() {
        // Stop recording
        log::info!("Stopping recording...");
        let capture = ctx.capture.take().unwrap();
        let samples = capture.stop();
        log::info!("Captured {} samples ({:.1}s)", samples.len(), samples.len() as f64 / 16000.0);

        update_tray_state(&ctx.tray_cmd_tx, AppState::Transcribing);

        // Create a runtime for the async ASR call
        let rt = match tokio::runtime::Runtime::new() {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to create tokio runtime: {}", e);
                update_tray_state(&ctx.tray_cmd_tx, AppState::Idle);
                return;
            }
        };

        // 1. Encode PCM to WAV
        let wav_bytes = match pcm_to_wav(&samples, 16000) {
            Ok(b) => b,
            Err(e) => {
                log::error!("WAV encoding failed: {}", e);
                update_tray_state(&ctx.tray_cmd_tx, AppState::Idle);
                return;
            }
        };

        // 2. Run ASR
        log::info!("Running ASR with engine '{}'...", ctx.asr_mgr.active_engine());
        let result = rt.block_on(ctx.asr_mgr.transcribe(&wav_bytes));
        let text = match result {
            Ok(t) => {
                log::info!("ASR result: {} chars", t.len());
                t
            }
            Err(e) => {
                log::error!("ASR failed: {}", e);
                update_tray_state(&ctx.tray_cmd_tx, AppState::Idle);
                return;
            }
        };

        // 3. Inject text
        log::info!("Injecting text ({} chars) via {:?}", text.len(), ctx.inject_mode);
        if let Err(e) = inject_text(&text, ctx.inject_mode) {
            log::error!("Text injection failed: {}", e);
        }

        update_tray_state(&ctx.tray_cmd_tx, AppState::Idle);
    } else {
        // Start recording
        log::info!("Starting recording...");
        match AudioCapture::start() {
            Ok(capture) => {
                update_tray_state(&ctx.tray_cmd_tx, AppState::Recording);
                ctx.capture = Some(capture);
                log::info!("Recording started.");
            }
            Err(e) => {
                log::error!("Failed to start recording: {}", e);
            }
        }
    }
}
