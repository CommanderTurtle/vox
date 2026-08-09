//! vox — Global voice input/output booster for CLI AI agents.
//!
//! Runs as a system-tray application. See SPEC.md for the full specification.

mod app;
mod asr;
mod audio;
mod config;
mod inject;
mod settings;
mod subtitles;
mod translation;
mod tray;
mod tts;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam::channel;

use app::hotkey::{start_hotkey_listener, HotkeyBinding, HotkeyEvent};
use app::state::{AppState, RecordMode};
use asr::AsrManager;
use audio::capture::AudioCapture;
use audio::utils::{pcm_to_wav, resample_linear};
use config::ConfigManager;
use inject::{inject_text, InjectMode};
use translation::Translator;
use tray::{refresh_menu, spawn_tray, MenuModel, TrayCommand, TrayEvent};
use tts::mimo_tts::MimoTtsEngine;
use tts::TtsInputMode;
use tts::TtsManager;

/// Result of an async ASR job, posted back to the main loop.
enum AsrResult {
    /// Successfully transcribed text, ready to inject.
    Text {
        text: String,
        destination: RecordingDestination,
    },
    /// ASR failed; message is logged and the app returns to idle.
    Failed(String),
    /// Async translate/TTS action completed without text injection.
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordingDestination {
    Inject,
    TranslateAndInject,
    Speak,
    TranslateAndSpeak,
}

/// Shared application state.
struct AppCtx {
    config_mgr: ConfigManager,
    asr_mgr: Arc<AsrManager>,
    tts_mgr: Arc<TtsManager>,
    translator: Arc<Translator>,
    tts_input_mode: tts::TtsInputMode,
    inject_mode: InjectMode,
    /// How the record hotkey behaves (push-to-talk vs toggle).
    record_mode: RecordMode,
    /// Sender for commands to the tray thread.
    tray_cmd_tx: channel::Sender<TrayCommand>,
    /// Current recording capture handle (None when idle).
    capture: Option<AudioCapture>,
    recording_destination: RecordingDestination,
    /// Shared async runtime for ASR/TTS (kept alive for the app lifetime).
    runtime: tokio::runtime::Runtime,
    /// Sender for ASR results posted from background tasks.
    asr_result_tx: channel::Sender<AsrResult>,
    /// Settings window sends the edited config snapshot here on save.
    settings_save_tx: channel::Sender<config::Config>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Install the rustls crypto provider once, up front. tokio-tungstenite
    // (used by Edge TTS) needs a CryptoProvider installed before any TLS
    // handshake, and rustls no longer picks one automatically.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // ── CLI subcommands (debug/testing) ──────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 {
        match args[1].as_str() {
            "transcribe" => {
                return cmd_transcribe(&args[2..]);
            }
            "inject" => {
                return cmd_inject(&args[2..]);
            }
            "tts" => {
                return cmd_tts(&args[2..]);
            }
            "translate" => {
                return cmd_translate(&args[2..]);
            }
            "subtitles" => {
                let dub = args[2..].iter().any(|arg| arg == "--dub");
                let source = args[2..]
                    .windows(2)
                    .find(|pair| pair[0] == "--source")
                    .map(|pair| pair[1].as_str())
                    .unwrap_or("system");
                return subtitles::run(
                    source,
                    dub || args[2..].iter().any(|arg| arg == "--translate"),
                    dub,
                );
            }
            _ => {}
        }
    }

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
    let (
        record_binding,
        engine_switch_binding,
        inject_switch_binding,
        tts_binding,
        tts_voice_binding,
        tts_seed_increment_binding,
        tts_seed_decrement_binding,
        translate_text_binding,
        translate_tts_binding,
        record_translate_tts_binding,
        record_translate_text_binding,
        record_tts_binding,
        translate_route_binding,
    ) = {
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
        let tts = HotkeyBinding::parse(tts_str).expect("Invalid tts_trigger hotkey config");
        let voice = HotkeyBinding::parse(&cfg.hotkey.tts_voice_switch)
            .expect("Invalid tts_voice_switch hotkey config");
        let seed_increment = HotkeyBinding::parse(&cfg.hotkey.tts_seed_increment)
            .expect("Invalid tts_seed_increment hotkey config");
        let seed_decrement = HotkeyBinding::parse(&cfg.hotkey.tts_seed_decrement)
            .expect("Invalid tts_seed_decrement hotkey config");
        let translate_text = HotkeyBinding::parse(&cfg.hotkey.translate_text)
            .expect("Invalid translate_text hotkey config");
        let translate_tts = HotkeyBinding::parse(&cfg.hotkey.translate_tts)
            .expect("Invalid translate_tts hotkey config");
        let record_translate_tts = HotkeyBinding::parse(&cfg.hotkey.record_translate_tts)
            .expect("Invalid record_translate_tts hotkey config");
        let record_translate_text = HotkeyBinding::parse(&cfg.hotkey.record_translate_text)
            .expect("Invalid record_translate_text hotkey config");
        let record_tts =
            HotkeyBinding::parse(&cfg.hotkey.record_tts).expect("Invalid record_tts hotkey config");
        let translate_route = HotkeyBinding::parse(&cfg.hotkey.translate_route_switch)
            .expect("Invalid translate_route_switch hotkey config");
        (
            record,
            engine,
            inject,
            tts,
            voice,
            seed_increment,
            seed_decrement,
            translate_text,
            translate_tts,
            record_translate_tts,
            record_translate_text,
            record_tts,
            translate_route,
        )
    };

    // ── Initialize ASR manager ───────────────────────────────────────────
    let asr_mgr: Arc<AsrManager> = Arc::new(build_asr_manager(&config_mgr));

    // ── Initialize TTS manager ───────────────────────────────────────────
    let tts_mgr: Arc<TtsManager> = Arc::new(build_tts_manager(&config_mgr));
    let translator = Arc::new(Translator::new(&config_mgr.read().translate));

    let inject_mode = InjectMode::from_config(&config_mgr.read());
    inject::set_restore_clipboard(config_mgr.read().inject.restore_clipboard);
    inject::set_copy_only(config_mgr.read().inject.copy_only);
    let tts_input_mode = TtsInputMode::from_config(&config_mgr.read());
    let record_mode = RecordMode::from_str(&config_mgr.read().general.record_mode);

    // ── Build system tray (dedicated thread with message pump) ───────────
    let (tray_event_sender, tray_event_receiver) = channel::unbounded::<TrayEvent>();

    let (tts_voice_profiles, tts_voice_active, tts_longcat_seed) = longcat_voice_menu(&config_mgr);
    let (translate_enabled, translate_route, translate_target) = translation_menu(&config_mgr);
    let crisper_mode = config_mgr.read().asr.crisper.mode.clone();
    let initial_model = MenuModel {
        asr_engines: asr_mgr.engine_names(),
        asr_active: asr_mgr.active_engine(),
        inject_mode: inject_mode.as_str().to_string(),
        restore_clipboard: config_mgr.read().inject.restore_clipboard,
        copy_only: config_mgr.read().inject.copy_only,
        tts_engines: tts_mgr.engine_names(),
        tts_active: tts_mgr.active_engine(),
        tts_input_mode: tts_input_mode.as_str().to_string(),
        tts_output: config_mgr.read().tts.output.mode.clone(),
        tts_voice_profiles,
        tts_voice_active,
        tts_longcat_seed,
        translate_enabled,
        translate_route,
        translate_target,
        crisper_mode,
        record_mode: record_mode.as_str().to_string(),
        app_state: AppState::Idle,
    };
    let (tray_cmd_tx, _tray_handle) = spawn_tray(initial_model, tray_event_sender.clone());

    // ── Start global hotkey listener ──────────────────────────────────────
    let (hotkey_sender, hotkey_receiver) = channel::unbounded::<HotkeyEvent>();
    let stop_flag = Arc::new(AtomicBool::new(false));

    start_hotkey_listener(
        record_binding,
        engine_switch_binding,
        inject_switch_binding,
        tts_binding,
        tts_voice_binding,
        tts_seed_increment_binding,
        tts_seed_decrement_binding,
        translate_text_binding,
        translate_tts_binding,
        record_translate_tts_binding,
        record_translate_text_binding,
        record_tts_binding,
        translate_route_binding,
        hotkey_sender.clone(),
        stop_flag.clone(),
    );

    log::info!("vox initialized. Waiting for events...");

    // ── Shared async runtime + ASR result channel ───────────────────────
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (asr_result_tx, asr_result_rx) = channel::unbounded::<AsrResult>();

    // Channel for the settings window to deliver an edited config snapshot
    // on save; the main thread persists + applies it live.
    let (settings_save_tx, settings_save_rx) = channel::unbounded::<config::Config>();

    // ── Application context ──────────────────────────────────────────────
    let mut ctx = AppCtx {
        config_mgr,
        asr_mgr,
        tts_mgr,
        translator,
        tts_input_mode,
        inject_mode,
        record_mode,
        tray_cmd_tx,
        capture: None,
        recording_destination: RecordingDestination::Inject,
        runtime,
        asr_result_tx,
        settings_save_tx,
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
            recv(asr_result_rx) -> msg => {
                if let Ok(result) = msg {
                    handle_asr_result(&mut ctx, result);
                }
            }
            recv(settings_save_rx) -> msg => {
                if let Ok(new_cfg) = msg {
                    apply_settings(&mut ctx, new_cfg);
                }
            }
        }
    }

    log::info!("vox shutdown complete.");
    Ok(())
}

/// Build a snapshot of the tray menu model from the live context.
fn build_menu_model(ctx: &AppCtx, app_state: AppState) -> MenuModel {
    let (tts_voice_profiles, tts_voice_active, tts_longcat_seed) =
        longcat_voice_menu(&ctx.config_mgr);
    let (translate_enabled, translate_route, translate_target) = translation_menu(&ctx.config_mgr);
    let crisper_mode = ctx.config_mgr.read().asr.crisper.mode.clone();
    MenuModel {
        asr_engines: ctx.asr_mgr.engine_names(),
        asr_active: ctx.asr_mgr.active_engine(),
        inject_mode: ctx.inject_mode.as_str().to_string(),
        restore_clipboard: ctx.config_mgr.read().inject.restore_clipboard,
        copy_only: ctx.config_mgr.read().inject.copy_only,
        tts_engines: ctx.tts_mgr.engine_names(),
        tts_active: ctx.tts_mgr.active_engine(),
        tts_input_mode: ctx.tts_input_mode.as_str().to_string(),
        tts_output: ctx.config_mgr.read().tts.output.mode.clone(),
        tts_voice_profiles,
        tts_voice_active,
        tts_longcat_seed,
        translate_enabled,
        translate_route,
        translate_target,
        crisper_mode,
        record_mode: ctx.record_mode.as_str().to_string(),
        app_state,
    }
}

/// Push a refreshed menu + tooltip to the tray thread.
fn push_tray(ctx: &AppCtx, app_state: AppState) {
    refresh_menu(&ctx.tray_cmd_tx, build_menu_model(ctx, app_state));
}

/// Handle a tray event. Returns false if the app should quit.
fn handle_tray_event(
    ctx: &mut AppCtx,
    msg: Result<TrayEvent, crossbeam::channel::RecvError>,
) -> bool {
    match msg {
        Ok(TrayEvent::Quit) => {
            log::info!("Quit requested.");
            return false;
        }
        Ok(TrayEvent::OpenSettings) => {
            log::info!("Opening settings window...");
            let snapshot = ctx.config_mgr.read().clone();
            settings::spawn_settings_window(snapshot, ctx.settings_save_tx.clone());
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
            push_tray(ctx, AppState::Idle);
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
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::SetRestoreClipboard(restore)) => {
            {
                let mut cfg = ctx.config_mgr.write();
                cfg.inject.restore_clipboard = restore;
            }
            inject::set_restore_clipboard(restore);
            let _ = ctx.config_mgr.save();
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::SetCopyOnly(copy_only)) => {
            {
                let mut cfg = ctx.config_mgr.write();
                cfg.inject.copy_only = copy_only;
            }
            inject::set_copy_only(copy_only);
            let _ = ctx.config_mgr.save();
            push_tray(ctx, AppState::Idle);
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
            push_tray(ctx, AppState::Idle);
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
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::SetTtsOutput(mode)) => {
            log::info!("Switching TTS output to: {}", mode);
            {
                let mut cfg = ctx.config_mgr.write();
                cfg.tts.output.mode = mode;
            }
            let _ = ctx.config_mgr.save();
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::SetTtsVoiceProfile(name)) => {
            log::info!("Switching LongCat voice profile to: {}", name);
            {
                let mut cfg = ctx.config_mgr.write();
                if cfg
                    .tts
                    .longcat
                    .voice_profiles
                    .iter()
                    .any(|profile| profile.name == name)
                {
                    cfg.tts.longcat.active_voice_profile = name;
                }
            }
            let _ = ctx.config_mgr.save();
            rebuild_tts_manager(ctx);
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::AdjustLongCatSeed(delta)) => {
            let seed = {
                let mut cfg = ctx.config_mgr.write();
                cfg.tts.longcat.seed = if delta.is_negative() {
                    cfg.tts
                        .longcat
                        .seed
                        .saturating_sub(delta.unsigned_abs() as u64)
                } else {
                    cfg.tts.longcat.seed.saturating_add(delta as u64)
                };
                cfg.tts.longcat.seed
            };
            log::info!("LongCat seed: {}", seed);
            let _ = ctx.config_mgr.save();
            rebuild_tts_manager(ctx);
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::SetTranslateEnabled(enabled)) => {
            log::info!(
                "{} local translation",
                if enabled { "Enabling" } else { "Disabling" }
            );
            {
                let mut cfg = ctx.config_mgr.write();
                cfg.translate.enabled = enabled;
            }
            let _ = ctx.config_mgr.save();
            ctx.translator = Arc::new(Translator::new(&ctx.config_mgr.read().translate));
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::SetTranslateRoute(route)) => {
            log::info!("Switching translation route to: {}", route);
            {
                let mut cfg = ctx.config_mgr.write();
                cfg.translate.active_route = route;
            }
            let _ = ctx.config_mgr.save();
            ctx.translator = Arc::new(Translator::new(&ctx.config_mgr.read().translate));
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::SetTranslateTarget(language)) => {
            log::info!("Switching outbound translation target to: {}", language);
            {
                let mut cfg = ctx.config_mgr.write();
                cfg.translate.outbound.target_language = language;
            }
            let _ = ctx.config_mgr.save();
            ctx.translator = Arc::new(Translator::new(&ctx.config_mgr.read().translate));
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::SetCrisperMode(mode)) => {
            log::info!("Switching Crisper transcript mode to: {}", mode);
            {
                let mut cfg = ctx.config_mgr.write();
                cfg.asr.crisper.mode = mode;
            }
            let _ = ctx.config_mgr.save();
            ctx.asr_mgr = Arc::new(build_asr_manager(&ctx.config_mgr));
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::StartSubtitles {
            source,
            translate,
            dub,
        }) => {
            if let Err(error) = launch_subtitles(&source, translate, dub) {
                log::error!("Could not launch subtitles: {}", error);
            }
        }
        Ok(TrayEvent::SetRecordMode(mode)) => {
            log::info!("Switching record mode to: {}", mode);
            ctx.record_mode = RecordMode::from_str(&mode);
            let mut cfg = ctx.config_mgr.write();
            cfg.general.record_mode = mode;
            drop(cfg);
            let _ = ctx.config_mgr.save();
            push_tray(ctx, AppState::Idle);
        }
        Ok(TrayEvent::ToggleRecording) => {
            toggle_recording(ctx);
        }
        Err(_) => return false,
    }
    true
}

fn launch_subtitles(source: &str, translate: bool, dub: bool) -> std::io::Result<()> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.args(["subtitles", "--source", source]);
    if dub {
        command.arg("--dub");
    } else if translate {
        command.arg("--translate");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command.spawn()?;
    Ok(())
}

/// Handle a hotkey event.
fn handle_hotkey_event(ctx: &mut AppCtx, event: HotkeyEvent) {
    match event {
        HotkeyEvent::RecordTogglePressed => {
            log::info!("[Hotkey] Record toggle pressed");
            match ctx.record_mode {
                // Push-to-talk: press starts recording (only if idle).
                RecordMode::PushToTalk => start_recording(ctx),
                // Toggle: press flips start/stop.
                RecordMode::Toggle => toggle_recording(ctx),
            }
        }
        HotkeyEvent::RecordToggleReleased => {
            // Push-to-talk: release stops recording (only if recording).
            if ctx.record_mode == RecordMode::PushToTalk {
                log::info!("[Hotkey] Record toggle released (PTT stop)");
                stop_recording(ctx);
            }
        }
        HotkeyEvent::EngineSwitch => {
            if let Some(name) = ctx.asr_mgr.cycle_engine() {
                log::info!("[Hotkey] Switched engine to: {}", name);
                let mut cfg = ctx.config_mgr.write();
                cfg.asr.primary_engine = name.to_string();
                drop(cfg);
                let _ = ctx.config_mgr.save();
                push_tray(ctx, AppState::Idle);
            }
        }
        HotkeyEvent::InjectModeSwitch => {
            ctx.inject_mode = ctx.inject_mode.toggle();
            log::info!(
                "[Hotkey] Switched inject mode to: {}",
                ctx.inject_mode.as_str()
            );
            let mut cfg = ctx.config_mgr.write();
            cfg.inject.mode = ctx.inject_mode.as_str().to_string();
            drop(cfg);
            let _ = ctx.config_mgr.save();
            push_tray(ctx, AppState::Idle);
        }
        HotkeyEvent::TtsTrigger => {
            log::info!("[Hotkey] TTS trigger");
            trigger_tts(ctx);
        }
        HotkeyEvent::TtsVoiceSwitch => {
            let switched = {
                let mut cfg = ctx.config_mgr.write();
                cfg.tts.longcat.cycle_voice_profile()
            };
            if let Some(name) = switched {
                log::info!("[Hotkey] Switched LongCat voice profile to: {}", name);
                let _ = ctx.config_mgr.save();
                rebuild_tts_manager(ctx);
                push_tray(ctx, AppState::Idle);
            } else {
                log::warn!("[Hotkey] No named LongCat voice profiles configured");
            }
        }
        HotkeyEvent::TtsSeedIncrement => {
            let seed = {
                let mut cfg = ctx.config_mgr.write();
                cfg.tts.longcat.seed = cfg.tts.longcat.seed.saturating_add(1);
                cfg.tts.longcat.seed
            };
            log::info!("[Hotkey] LongCat seed: {}", seed);
            let _ = ctx.config_mgr.save();
            rebuild_tts_manager(ctx);
            push_tray(ctx, AppState::Idle);
        }
        HotkeyEvent::TtsSeedDecrement => {
            let seed = {
                let mut cfg = ctx.config_mgr.write();
                cfg.tts.longcat.seed = cfg.tts.longcat.seed.saturating_sub(1);
                cfg.tts.longcat.seed
            };
            log::info!("[Hotkey] LongCat seed: {}", seed);
            let _ = ctx.config_mgr.save();
            rebuild_tts_manager(ctx);
            push_tray(ctx, AppState::Idle);
        }
        HotkeyEvent::TranslateText => {
            log::info!("[Hotkey] Translate selected text");
            trigger_translate(ctx, false);
        }
        HotkeyEvent::TranslateTts => {
            log::info!("[Hotkey] Translate selected text and speak");
            trigger_translate(ctx, true);
        }
        HotkeyEvent::RecordTranslateTtsPressed => {
            log::info!("[Hotkey] Speech → translation → TTS pressed");
            if !ctx.translator.is_enabled() {
                log::warn!(
                    "Translation is disabled; enable Local Translation from the tray or Settings"
                );
                return;
            }
            match ctx.record_mode {
                RecordMode::PushToTalk => {
                    start_recording_for(ctx, RecordingDestination::TranslateAndSpeak)
                }
                RecordMode::Toggle => {
                    if ctx.capture.is_some() {
                        stop_recording(ctx);
                    } else {
                        start_recording_for(ctx, RecordingDestination::TranslateAndSpeak);
                    }
                }
            }
        }
        HotkeyEvent::RecordTranslateTtsReleased => {
            if ctx.record_mode == RecordMode::PushToTalk
                && ctx.recording_destination == RecordingDestination::TranslateAndSpeak
            {
                stop_recording(ctx);
            }
        }
        HotkeyEvent::RecordTranslateTextPressed => {
            log::info!("[Hotkey] Speech → translation → text pressed");
            if !ctx.translator.is_enabled() {
                log::warn!(
                    "Translation is disabled; enable Local Translation from the tray or Settings"
                );
                return;
            }
            match ctx.record_mode {
                RecordMode::PushToTalk => {
                    start_recording_for(ctx, RecordingDestination::TranslateAndInject)
                }
                RecordMode::Toggle => {
                    if ctx.capture.is_some() {
                        stop_recording(ctx);
                    } else {
                        start_recording_for(ctx, RecordingDestination::TranslateAndInject);
                    }
                }
            }
        }
        HotkeyEvent::RecordTranslateTextReleased => {
            if ctx.record_mode == RecordMode::PushToTalk
                && ctx.recording_destination == RecordingDestination::TranslateAndInject
            {
                stop_recording(ctx);
            }
        }
        HotkeyEvent::RecordTtsPressed => {
            log::info!("[Hotkey] Speech → raw TTS pressed");
            match ctx.record_mode {
                RecordMode::PushToTalk => start_recording_for(ctx, RecordingDestination::Speak),
                RecordMode::Toggle => {
                    if ctx.capture.is_some() {
                        stop_recording(ctx);
                    } else {
                        start_recording_for(ctx, RecordingDestination::Speak);
                    }
                }
            }
        }
        HotkeyEvent::RecordTtsReleased => {
            if ctx.record_mode == RecordMode::PushToTalk
                && ctx.recording_destination == RecordingDestination::Speak
            {
                stop_recording(ctx);
            }
        }
        HotkeyEvent::TranslateRouteSwitch => {
            let route = {
                let mut cfg = ctx.config_mgr.write();
                cfg.translate.cycle_route()
            };
            let _ = ctx.config_mgr.save();
            ctx.translator = Arc::new(Translator::new(&ctx.config_mgr.read().translate));
            log::info!("[Hotkey] Translation route: {}", route);
            push_tray(ctx, AppState::Idle);
        }
    }
}

/// Trigger TTS: read text and speak it.
///
/// Text acquisition happens synchronously (it simulates Ctrl+C / reads the
/// clipboard, which must run on the main thread), but synthesis + playback
/// are dispatched to the shared runtime so the event loop is not blocked.
fn trigger_tts(ctx: &mut AppCtx) {
    let Some(text) = read_tts_text(ctx.tts_input_mode) else {
        return;
    };
    speak_text(ctx, text, ctx.translator.translates_tts());
}

fn read_tts_text(mode: TtsInputMode) -> Option<String> {
    log::info!("Text input mode: {:?}", mode);
    match mode {
        TtsInputMode::Selection => match inject::text_reader::read_selected_text() {
            Ok(t) if !t.is_empty() => {
                log::info!("Got {} chars from selection", t.len());
                t
            }
            Ok(_) => {
                log::error!("No text selected. Select text first or switch to Clipboard mode.");
                return None;
            }
            Err(e) => {
                log::error!("Failed to read selection: {}", e);
                return None;
            }
        },
        TtsInputMode::Clipboard => match inject::text_reader::read_clipboard_text() {
            Ok(t) if !t.is_empty() => {
                log::info!("Got {} chars from clipboard", t.len());
                t
            }
            Ok(_) => {
                log::error!("Clipboard is empty.");
                return None;
            }
            Err(e) => {
                log::error!("Failed to read clipboard: {}", e);
                return None;
            }
        },
    }
    .into()
}

fn speak_text(ctx: &AppCtx, text: String, translate: bool) {
    log::info!(
        "TTS synthesizing with engine '{}': {} chars",
        ctx.tts_mgr.active_engine(),
        text.len()
    );

    // Synthesize on the shared runtime; play once ready. The event loop is
    // not blocked — this just spawns a task and returns.
    let tts_mgr_ref = ctx.tts_mgr.clone();
    let translator = ctx.translator.clone();
    let active = ctx.tts_mgr.active_engine();
    let fmt = ctx.tts_mgr.output_format();
    let output = ctx.config_mgr.read().tts.output.clone();
    ctx.runtime.spawn(async move {
        let text = if translate {
            match translator.translate(&text).await {
                Ok(translated) => translated,
                Err(error) => {
                    log::warn!("TTS translation failed; speaking original text: {}", error);
                    text
                }
            }
        } else {
            text
        };
        let result = tts_mgr_ref.synthesize(&text).await;
        match result {
            Ok(audio) => {
                log::info!("TTS synthesized {} bytes of audio ({:?})", audio.len(), fmt);
                if let Err(error) = tts::output::deliver(audio, fmt, output).await {
                    log::error!("TTS output failed: {}", error);
                }
            }
            Err(e) => {
                log::error!("TTS synthesis (engine '{}') failed: {}", active, e);
            }
        }
    });
}

/// Translate selected/clipboard text with the currently selected route, then
/// either inject the result or synthesize it with the active TTS voice.
fn trigger_translate(ctx: &mut AppCtx, speak: bool) {
    if !ctx.translator.is_enabled() {
        log::warn!("Translation is disabled; enable it in Vox Settings first");
        return;
    }
    let Some(text) = read_tts_text(ctx.tts_input_mode) else {
        return;
    };
    push_tray(ctx, AppState::Transcribing);
    let translator = ctx.translator.clone();
    let result_tx = ctx.asr_result_tx.clone();
    let tts_mgr = ctx.tts_mgr.clone();
    let fmt = ctx.tts_mgr.output_format();
    let output = ctx.config_mgr.read().tts.output.clone();
    ctx.runtime.spawn(async move {
        match translator.translate(&text).await {
            Ok(translated) if speak => match tts_mgr.synthesize(&translated).await {
                Ok(audio) => {
                    if let Err(error) = tts::output::deliver(audio, fmt, output).await {
                        log::error!("Translated TTS output failed: {}", error);
                    }
                    let _ = result_tx.send(AsrResult::Completed);
                }
                Err(error) => {
                    log::error!("Translated TTS failed: {}", error);
                    let _ = result_tx.send(AsrResult::Completed);
                }
            },
            Ok(translated) => {
                let _ = result_tx.send(AsrResult::Text {
                    text: translated,
                    destination: RecordingDestination::Inject,
                });
            }
            Err(error) => {
                log::error!("Translation failed: {}", error);
                let _ = result_tx.send(AsrResult::Completed);
            }
        }
    });
}

/// Flip recording state (used by Toggle mode and the tray button).
fn toggle_recording(ctx: &mut AppCtx) {
    if ctx.capture.is_some() {
        stop_recording(ctx);
    } else {
        start_recording(ctx);
    }
}

/// Start recording from the microphone.
fn start_recording(ctx: &mut AppCtx) {
    start_recording_for(ctx, RecordingDestination::Inject);
}

fn start_recording_for(ctx: &mut AppCtx, destination: RecordingDestination) {
    if ctx.capture.is_some() {
        log::warn!("start_recording called while already recording; ignoring");
        return;
    }
    log::info!("Starting recording...");
    match AudioCapture::start() {
        Ok(capture) => {
            ctx.recording_destination = destination;
            push_tray(ctx, AppState::Recording);
            ctx.capture = Some(capture);
            log::info!("Recording started.");
        }
        Err(e) => {
            log::error!("Failed to start recording: {}", e);
        }
    }
}

/// Stop recording, resample + encode to WAV, and kick off ASR.
fn stop_recording(ctx: &mut AppCtx) {
    let capture = match ctx.capture.take() {
        Some(c) => c,
        None => {
            log::warn!("stop_recording called while not recording; ignoring");
            return;
        }
    };
    let destination = ctx.recording_destination;
    log::info!("Stopping recording...");
    let device_rate = capture.sample_rate();
    let samples = capture.stop();
    log::info!(
        "Captured {} samples at {} Hz ({:.1}s)",
        samples.len(),
        device_rate,
        samples.len() as f64 / device_rate as f64
    );

    push_tray(ctx, AppState::Transcribing);

    // Resample to 16 kHz (what ASR engines expect) and encode to WAV.
    let samples_16k = resample_linear(&samples, device_rate, 16000);
    let wav_bytes = match pcm_to_wav(&samples_16k, 16000) {
        Ok(b) => b,
        Err(e) => {
            log::error!("WAV encoding failed: {}", e);
            push_tray(ctx, AppState::Idle);
            return;
        }
    };

    // Run ASR on the shared runtime without blocking the main loop.
    log::info!(
        "Running ASR with engine '{}'...",
        ctx.asr_mgr.active_engine()
    );
    let asr_ref = ctx.asr_mgr.clone();
    let translator = ctx.translator.clone();
    let asr_result_tx = ctx.asr_result_tx.clone();
    ctx.runtime.spawn(async move {
        let result = asr_ref.transcribe(&wav_bytes).await;
        match result {
            Ok(text) => {
                log::info!("ASR result: {} chars", text.len());
                let must_translate = match destination {
                    RecordingDestination::Inject => translator.translates_asr(),
                    RecordingDestination::TranslateAndInject => true,
                    RecordingDestination::Speak => false,
                    RecordingDestination::TranslateAndSpeak => true,
                };
                let text = if must_translate {
                    match translator.translate(&text).await {
                        Ok(translated) => translated,
                        Err(error) => {
                            log::warn!(
                                "ASR translation failed; injecting original transcript: {}",
                                error
                            );
                            text
                        }
                    }
                } else {
                    text
                };
                let _ = asr_result_tx.send(AsrResult::Text { text, destination });
            }
            Err(e) => {
                let _ = asr_result_tx.send(AsrResult::Failed(e.to_string()));
            }
        }
    });
}

/// Handle an ASR result posted from a background task.
fn handle_asr_result(ctx: &mut AppCtx, result: AsrResult) {
    match result {
        AsrResult::Text { text, destination } => match destination {
            RecordingDestination::Inject => {
                log::info!(
                    "Injecting text ({} chars) via {:?}",
                    text.len(),
                    ctx.inject_mode
                );
                if let Err(e) = inject_text(&text, ctx.inject_mode) {
                    log::error!("Text injection failed: {}", e);
                }
            }
            RecordingDestination::TranslateAndInject => {
                log::info!(
                    "Injecting translated text ({} chars) via {:?}",
                    text.len(),
                    ctx.inject_mode
                );
                if let Err(e) = inject_text(&text, ctx.inject_mode) {
                    log::error!("Translated text injection failed: {}", e);
                }
            }
            RecordingDestination::TranslateAndSpeak => speak_text(ctx, text, false),
            RecordingDestination::Speak => speak_text(ctx, text, false),
        },
        AsrResult::Failed(msg) => {
            log::error!("ASR failed: {}", msg);
        }
        AsrResult::Completed => {}
    }
    ctx.recording_destination = RecordingDestination::Inject;
    push_tray(ctx, AppState::Idle);
}

/// Apply an edited config snapshot delivered from the settings window:
/// persist it, rebuild engine managers, refresh runtime modes, and refresh
/// the tray menu — all live, no restart.
fn apply_settings(ctx: &mut AppCtx, new_cfg: config::Config) {
    log::info!("Applying settings from settings window...");
    {
        let mut g = ctx.config_mgr.write();
        *g = new_cfg;
    }
    if let Err(e) = ctx.config_mgr.save() {
        log::error!("Failed to persist config: {}", e);
    }

    ctx.asr_mgr = Arc::new(build_asr_manager(&ctx.config_mgr));
    ctx.tts_mgr = Arc::new(build_tts_manager(&ctx.config_mgr));
    ctx.translator = Arc::new(Translator::new(&ctx.config_mgr.read().translate));
    ctx.inject_mode = InjectMode::from_config(&ctx.config_mgr.read());
    inject::set_restore_clipboard(ctx.config_mgr.read().inject.restore_clipboard);
    inject::set_copy_only(ctx.config_mgr.read().inject.copy_only);
    ctx.tts_input_mode = TtsInputMode::from_config(&ctx.config_mgr.read());
    ctx.record_mode = RecordMode::from_str(&ctx.config_mgr.read().general.record_mode);

    push_tray(ctx, AppState::Idle);
    log::info!(
        "Settings applied | engine={} | inject={:?} | tts={} | tts_input={:?}",
        ctx.asr_mgr.active_engine(),
        ctx.inject_mode,
        ctx.tts_mgr.active_engine(),
        ctx.tts_input_mode,
    );
}

fn longcat_voice_menu(config_mgr: &ConfigManager) -> (Vec<String>, String, u64) {
    let cfg = config_mgr.read();
    let profiles = cfg
        .tts
        .longcat
        .voice_profiles
        .iter()
        .map(|profile| profile.name.clone())
        .collect();
    (
        profiles,
        cfg.tts.longcat.active_voice_name(),
        cfg.tts.longcat.seed,
    )
}

fn translation_menu(config_mgr: &ConfigManager) -> (bool, String, String) {
    let cfg = config_mgr.read();
    (
        cfg.translate.enabled,
        cfg.translate.active_route.clone(),
        cfg.translate.outbound.target_language.clone(),
    )
}

/// Rebuild only the TTS adapters after changing a voice profile while
/// preserving the selected engine.
fn rebuild_tts_manager(ctx: &mut AppCtx) {
    let active = ctx.tts_mgr.active_engine();
    let manager = build_tts_manager(&ctx.config_mgr);
    let _ = manager.set_active(&active);
    ctx.tts_mgr = Arc::new(manager);
}

// ── CLI debug subcommands ─────────────────────────────────────────────

/// `cargo run -- transcribe <audio.wav>`
fn cmd_transcribe(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let path = args.first().ok_or("Usage: vox transcribe <audio.wav>")?;
    let wav_bytes = std::fs::read(path)?;
    log::info!("Read {} bytes from {}", wav_bytes.len(), path);

    let config_mgr = ConfigManager::load_or_create()?;
    let asr_mgr = build_asr_manager(&config_mgr);

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(asr_mgr.transcribe(&wav_bytes));

    match result {
        Ok(text) => {
            println!("{}", text);
            Ok(())
        }
        Err(e) => {
            eprintln!("ASR failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// `cargo run -- inject <text> [--mode keyboard|clipboard]`
fn cmd_inject(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        return Err("Usage: vox inject <text> [--mode keyboard|clipboard]".into());
    }

    let mut text = String::new();
    let mut mode = InjectMode::Keyboard;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                mode = match args.get(i).map(|s| s.as_str()) {
                    Some("clipboard") => InjectMode::Clipboard,
                    _ => InjectMode::Keyboard,
                };
            }
            s => {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(s);
            }
        }
        i += 1;
    }

    log::info!("Injecting {} chars via {:?}", text.len(), mode);
    inject_text(&text, mode)?;
    log::info!("Injection complete.");
    Ok(())
}

/// `cargo run -- tts <text> [output]` — synthesize via the configured TTS
/// engine, write the audio file, and play it back. Debug subcommand to verify
/// TTS without the GUI.
fn cmd_tts(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        return Err("Usage: vox tts <text> [output.{wav|mp3}]".into());
    }
    let text = args[0].clone();

    let config_mgr = ConfigManager::load_or_create()?;
    let output = config_mgr.read().tts.output.clone();
    let tts_mgr = build_tts_manager(&config_mgr);
    log::info!("TTS engine: {}", tts_mgr.active_engine());

    let rt = tokio::runtime::Runtime::new()?;
    let audio = rt.block_on(tts_mgr.synthesize(&text))?;
    let fmt = tts_mgr.output_format();

    // Use the caller's path if given, else pick a suffix matching the format.
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| format!("tts_output{}", fmt.suffix()));
    std::fs::write(&path, &audio)?;
    log::info!("Wrote {} bytes ({:?}) to {}", audio.len(), fmt, path);

    log::info!("Delivering through TTS output mode '{}'...", output.mode);
    rt.block_on(tts::output::deliver(audio, fmt, output))?;
    log::info!("TTS output complete.");
    Ok(())
}

/// `vox translate <text>` — exercise only the configured local translation
/// endpoint without opening the tray application.
fn cmd_translate(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        return Err("Usage: vox translate <text>".into());
    }
    let text = args.join(" ");
    let config_mgr = ConfigManager::load_or_create()?;
    let translator = Translator::new(&config_mgr.read().translate);
    let runtime = tokio::runtime::Runtime::new()?;
    println!("{}", runtime.block_on(translator.translate(&text))?);
    Ok(())
}

fn build_asr_manager(config_mgr: &ConfigManager) -> AsrManager {
    use asr::aliyun_asr::AliyunAsrEngine;
    use asr::mimo_asr::MimoAsrEngine;
    use asr::openai_asr::OpenaiAsrEngine;
    use asr::whisper_cpp::WhisperCppEngine;
    use asr::whisper_local::WhisperLocalEngine;

    let (primary_engine, fallback_engines) = {
        let cfg = config_mgr.read();
        (
            cfg.asr.primary_engine.clone(),
            cfg.asr.fallback_engines.clone(),
        )
    };

    let mut asr_mgr = AsrManager::new(primary_engine, fallback_engines);

    // CrisperWhisper 2.0 local service. This adapter always requests the
    // intended/non-literal transcript and is safe to target across the LAN.
    {
        let cfg = config_mgr.read();
        log::info!(
            "Registering CrisperWhisper ASR engine at {} ({} mode)",
            cfg.asr.crisper.base_url,
            cfg.asr.crisper.mode
        );
        asr_mgr.register(Box::new(asr::crisper_whisper::CrisperWhisperEngine::new(
            &cfg.asr.crisper,
        )));
    }

    // whisper.cpp HTTP server — registered unconditionally; it's just an
    // HTTP client. Fails at transcribe time (and triggers fallback) if the
    // server isn't running.
    {
        let cfg = config_mgr.read();
        let wcpp_cfg = &cfg.asr.whisper_cpp;
        log::info!(
            "Registering whisper.cpp ASR engine at {}",
            wcpp_cfg.base_url
        );
        asr_mgr.register(Box::new(WhisperCppEngine::new(&wcpp_cfg.base_url)));
    }

    // Whisper local engine — only registers when a model path is configured
    // AND the `whisper-local` feature was enabled at compile time.
    {
        let cfg = config_mgr.read();
        let wl_cfg = &cfg.asr.whisper_local;
        if !wl_cfg.model_path.is_empty() {
            match WhisperLocalEngine::new(&wl_cfg.model_path) {
                Ok(engine) => asr_mgr.register(Box::new(engine)),
                Err(e) => log::warn!("whisper-local engine not available: {}", e),
            }
        } else {
            log::info!("whisper-local engine skipped: no model_path configured");
        }
    }

    // OpenAI-compatible endpoint. A blank key is valid only for an explicitly
    // local service; never register the public OpenAI default without auth.
    // That keeps a local-only profile from making a noisy cloud fallback.
    {
        let cfg = config_mgr.read();
        let openai_cfg = &cfg.asr.openai;
        let url = openai_cfg.base_url.to_ascii_lowercase();
        let local = url.starts_with("http://127.0.0.1")
            || url.starts_with("http://localhost")
            || url.starts_with("http://[::1]");
        if !openai_cfg.api_key.trim().is_empty() || local {
            log::info!(
                "Registering OpenAI-compatible ASR engine at {} (model {})",
                openai_cfg.base_url,
                openai_cfg.model
            );
            asr_mgr.register(Box::new(OpenaiAsrEngine::new(
                &openai_cfg.base_url,
                &openai_cfg.api_key,
                &openai_cfg.model,
            )));
        } else {
            log::info!("OpenAI-compatible ASR skipped: no local endpoint or API key");
        }
    }

    {
        let cfg = config_mgr.read();
        let mimo_cfg = &cfg.asr.mimo;
        if !mimo_cfg.api_key.is_empty() {
            log::info!(
                "Registering Mimo ASR engine: {} at {}",
                mimo_cfg.model,
                mimo_cfg.base_url
            );
            asr_mgr.register(Box::new(MimoAsrEngine::new(
                &mimo_cfg.base_url,
                &mimo_cfg.api_key,
                &mimo_cfg.model,
            )));
        }
    }

    {
        let cfg = config_mgr.read();
        let aliyun_cfg = &cfg.asr.aliyun;
        if !aliyun_cfg.appkey.is_empty() && !aliyun_cfg.token.is_empty() {
            log::info!("Registering Aliyun ASR engine");
            asr_mgr.register(Box::new(AliyunAsrEngine::new(
                &aliyun_cfg.appkey,
                &aliyun_cfg.token,
            )));
        }
    }

    // Doubao streaming ASR 2.0 - registered when an API key is configured.
    // Shares the key with the Doubao TTS engine. When registered, it becomes
    // the default primary engine (unless the user explicitly chose another).
    let doubao_asr_registered: bool;
    {
        let cfg = config_mgr.read();
        let doubao_cfg = &cfg.asr.doubao;
        if !doubao_cfg.api_key.is_empty() {
            log::info!("Registering Doubao ASR engine (volc.seedasr.sauc.duration)");
            asr_mgr.register(Box::new(asr::doubao_asr::DoubaoAsrEngine::new(
                &doubao_cfg.api_key,
            )));
            doubao_asr_registered = true;
        } else {
            log::info!("Doubao ASR engine skipped: no api_key configured");
            doubao_asr_registered = false;
        }
    }
    // Auto-select doubao-asr as primary when it's registered AND the user
    // hasn't explicitly chosen another engine (i.e. primary is still the
    // factory default "whisper-cpp").
    if doubao_asr_registered {
        let primary = asr_mgr.active_engine();
        if primary == "whisper-cpp" {
            log::info!("Auto-switching ASR primary to doubao-asr");
            let _ = asr_mgr.set_active("doubao-asr");
        }
    }

    asr_mgr
}

/// Build the TTS manager from config: Edge TTS (free, always) + Mimo TTS
/// (when an API key is present).
fn build_tts_manager(config_mgr: &ConfigManager) -> TtsManager {
    use tts::edge_tts::EdgeTtsEngine;

    let primary = {
        let cfg = config_mgr.read();
        cfg.tts.primary_engine.clone()
    };
    let mut tts_mgr = TtsManager::new(primary);

    // LongCat local voice cloning. Vox owns request chunking; the standalone
    // server and its WebUI remain unchanged.
    {
        let cfg = config_mgr.read();
        log::info!(
            "Registering LongCat TTS engine at {}",
            cfg.tts.longcat.base_url
        );
        tts_mgr.register(Box::new(tts::longcat_tts::LongCatTtsEngine::new(
            &cfg.tts.longcat,
        )));
    }

    // Edge TTS — free, no API key. Always registered.
    {
        let cfg = config_mgr.read();
        let edge = &cfg.tts.edge;
        log::info!("Registering Edge TTS engine (voice={})", edge.voice);
        tts_mgr.register(Box::new(EdgeTtsEngine::new(
            &edge.voice,
            &edge.rate,
            &edge.volume,
            &edge.pitch,
        )));
    }

    // Mimo TTS — only when an API key is configured.
    {
        let cfg = config_mgr.read();
        let mimo_cfg = &cfg.asr.mimo; // share base URL and API key with ASR
        let tts_cfg = &cfg.tts.mimo;
        if !mimo_cfg.api_key.is_empty() {
            log::info!(
                "Registering Mimo TTS engine: {} at {}",
                tts_cfg.model,
                mimo_cfg.base_url
            );
            let engine = MimoTtsEngine::new(&mimo_cfg.base_url, &mimo_cfg.api_key, &tts_cfg.model);
            tts_mgr.register(Box::new(engine));
        }
    }

    // Doubao TTS 2.0 (seed-tts-2.0) - registered when the shared Doubao API
    // key is configured. When registered, it becomes the default primary TTS
    // engine (unless the user explicitly chose another).
    let doubao_tts_registered: bool;
    {
        let cfg = config_mgr.read();
        // The API key lives on asr.doubao and is shared with the TTS engine.
        let api_key = &cfg.asr.doubao.api_key;
        if !api_key.is_empty() {
            let tts_cfg = &cfg.tts.doubao;
            log::info!(
                "Registering Doubao TTS engine (seed-tts-2.0, speaker={})",
                tts_cfg.speaker
            );
            let engine = tts::doubao_tts::DoubaoTtsEngine::new(
                api_key,
                &tts_cfg.speaker,
                tts_cfg.speech_rate,
                tts_cfg.loudness_rate,
                tts_cfg.sample_rate,
            );
            tts_mgr.register(Box::new(engine));
            doubao_tts_registered = true;
        } else {
            log::info!("Doubao TTS engine skipped: no api_key configured");
            doubao_tts_registered = false;
        }
    }
    // Auto-select doubao-tts as primary when it's registered AND the user
    // hasn't explicitly chosen another engine (i.e. primary is still the
    // factory default "edge-tts").
    if doubao_tts_registered {
        let primary = tts_mgr.active_engine();
        if primary == "edge-tts" {
            log::info!("Auto-switching TTS primary to doubao-tts");
            let _ = tts_mgr.set_active("doubao-tts");
        }
    }

    tts_mgr
}
