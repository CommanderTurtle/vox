//! System tray icon and context menu for vox.
//!
//! On Windows, the tray icon **must** live on a thread that pumps Windows
//! messages, otherwise right-click and menu events won't work. This module
//! spawns a dedicated thread for that purpose.
//!
//! ## Decoupling
//! The menu is built from a plain-data [`MenuModel`] rather than being
//! hard-coded. The main thread owns the model (it knows the active engine /
//! mode / state) and pushes a refreshed model to the tray thread via
//! [`TrayCommand::RefreshMenu`] whenever something changes; the tray thread
//! rebuilds the menu and tooltip from it. Neither side knows the other's
//! internals.

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
pub use tray_icon::TrayIcon;
use tray_icon::{Icon, TrayIconBuilder};

use crate::app::state::AppState;

/// Events emitted by the tray (user interactions).
#[derive(Debug, Clone)]
pub enum TrayEvent {
    Quit,
    /// Left-click / "Toggle Recording" menu item.
    ToggleRecording,
    SetEngine(String),
    SetInjectMode(String),
    OpenSettings,
    SetTtsEngine(String),
    SetTtsInputMode(String),
    SetTtsVoiceProfile(String),
    SetTranslateRoute(String),
    SetTranslateTarget(String),
    SetCrisperMode(String),
    /// "ptt" | "toggle"
    SetRecordMode(String),
}

/// Commands sent FROM the main thread TO the tray thread.
#[derive(Debug)]
pub enum TrayCommand {
    /// Refresh the menu + tooltip from a fresh model (active selections,
    /// app state, engine lists).
    RefreshMenu(MenuModel),
    /// Shut down the tray thread.
    Shutdown,
}

/// Plain-data description of the tray menu. Built by the main thread from
/// live app state; the tray thread renders it.
#[derive(Debug, Clone)]
pub struct MenuModel {
    pub asr_engines: Vec<String>,
    pub asr_active: String,
    /// "keyboard" | "clipboard"
    pub inject_mode: String,
    pub tts_engines: Vec<String>,
    pub tts_active: String,
    /// "selection" | "clipboard"
    pub tts_input_mode: String,
    pub tts_voice_profiles: Vec<String>,
    pub tts_voice_active: String,
    pub translate_enabled: bool,
    pub translate_route: String,
    pub translate_target: String,
    pub crisper_mode: String,
    /// "ptt" | "toggle"
    pub record_mode: String,
    pub app_state: AppState,
}

impl MenuModel {
    fn tooltip(&self) -> String {
        let state = match self.app_state {
            AppState::Idle => "Idle",
            AppState::Recording => "Recording…",
            AppState::Transcribing => "Transcribing…",
        };
        format!(
            "vox — {} | ASR: {} | TTS: {} | Voice: {}",
            state, self.asr_active, self.tts_active, self.tts_voice_active
        )
    }
}

// ── Menu item id helpers ──────────────────────────────────────────────
const ID_TOGGLE: &str = "toggle";
const ID_SETTINGS: &str = "settings";
const ID_QUIT: &str = "quit";

fn asr_id(name: &str) -> String {
    format!("asr:{}", name)
}
fn inject_id(mode: &str) -> String {
    format!("inject:{}", mode)
}
fn tts_id(name: &str) -> String {
    format!("tts:{}", name)
}
fn ttsinput_id(mode: &str) -> String {
    format!("ttsinput:{}", mode)
}
fn ttsvoice_id(name: &str) -> String {
    format!("ttsvoice:{}", name)
}
fn translate_route_id(route: &str) -> String {
    format!("translate-route:{}", route)
}
fn translate_target_id(language: &str) -> String {
    format!("translate-target:{}", language)
}
fn crisper_mode_id(mode: &str) -> String {
    format!("crisper-mode:{}", mode)
}

fn recordmode_id(mode: &str) -> String {
    format!("recordmode:{}", mode)
}

/// Build the tray menu from a model, with checkmarks on active items.
fn build_menu(m: &MenuModel) -> Menu {
    let menu = Menu::new();

    // ── Input group: ASR engine ────────────────────────────────────────
    let asr_menu = Submenu::new("ASR Engine", true);
    for name in &m.asr_engines {
        let checked = name == &m.asr_active;
        let _ = asr_menu.append(&CheckMenuItem::with_id(
            asr_id(name),
            name.clone(),
            true,
            checked,
            None,
        ));
    }
    let _ = menu.append(&asr_menu);

    let crisper_menu = Submenu::new("Crisper Transcript", true);
    for (mode, label) in [
        ("intended", "Intended / non-literal"),
        ("literal", "Literal / verbatim"),
    ] {
        let _ = crisper_menu.append(&CheckMenuItem::with_id(
            crisper_mode_id(mode),
            label,
            true,
            mode == m.crisper_mode,
            None,
        ));
    }
    let _ = menu.append(&crisper_menu);

    // ── Input group: inject mode ───────────────────────────────────────
    let inject_menu = Submenu::new("Inject Mode", true);
    for mode in &["keyboard", "clipboard"] {
        let checked = mode == &m.inject_mode;
        let label = if *mode == "keyboard" {
            "Keyboard"
        } else {
            "Clipboard"
        };
        let _ = inject_menu.append(&CheckMenuItem::with_id(
            inject_id(mode),
            label,
            true,
            checked,
            None,
        ));
    }
    let _ = menu.append(&inject_menu);

    // ── Input group: record mode (push-to-talk vs toggle) ────────────
    let record_menu = Submenu::new("Record Mode", true);
    for (mode, label) in &[("ptt", "Push-to-Talk (hold)"), ("toggle", "Toggle (press)")] {
        let checked = mode == &m.record_mode;
        let _ = record_menu.append(&CheckMenuItem::with_id(
            recordmode_id(mode),
            label,
            true,
            checked,
            None,
        ));
    }
    let _ = menu.append(&record_menu);

    let _ = menu.append(&PredefinedMenuItem::separator());

    // ── Output group: TTS engine ───────────────────────────────────────
    let tts_menu = Submenu::new("TTS Engine", true);
    for name in &m.tts_engines {
        let checked = name == &m.tts_active;
        let _ = tts_menu.append(&CheckMenuItem::with_id(
            tts_id(name),
            name.clone(),
            true,
            checked,
            None,
        ));
    }
    let _ = menu.append(&tts_menu);

    // ── Output group: TTS input mode ───────────────────────────────────
    let ttsinput_menu = Submenu::new("TTS Input", true);
    for mode in &["selection", "clipboard"] {
        let checked = mode == &m.tts_input_mode;
        let label = if *mode == "selection" {
            "Selection (Ctrl+C)"
        } else {
            "Clipboard"
        };
        let _ = ttsinput_menu.append(&CheckMenuItem::with_id(
            ttsinput_id(mode),
            label,
            true,
            checked,
            None,
        ));
    }
    let _ = menu.append(&ttsinput_menu);

    if !m.tts_voice_profiles.is_empty() {
        let voice_menu = Submenu::new("LongCat Voice", true);
        for name in &m.tts_voice_profiles {
            let _ = voice_menu.append(&CheckMenuItem::with_id(
                ttsvoice_id(name),
                name.clone(),
                true,
                name == &m.tts_voice_active,
                None,
            ));
        }
        let _ = menu.append(&voice_menu);
    }

    if m.translate_enabled {
        let translate_menu = Submenu::new("Translation Route", true);
        for (route, label) in [
            ("inbound", "Inbound: detect/source → English"),
            ("outbound", "Outbound: English → selected language"),
        ] {
            let _ = translate_menu.append(&CheckMenuItem::with_id(
                translate_route_id(route),
                label,
                true,
                route == m.translate_route,
                None,
            ));
        }
        let _ = menu.append(&translate_menu);

        let target_menu = Submenu::new("Outbound Language", true);
        for language in [
            "English",
            "Spanish",
            "French",
            "German",
            "Italian",
            "Portuguese",
            "Chinese",
            "Japanese",
            "Korean",
            "Arabic",
            "Hindi",
            "Russian",
            "Dutch",
            "Polish",
            "Turkish",
            "Vietnamese",
            "Thai",
            "Indonesian",
        ] {
            let _ = target_menu.append(&CheckMenuItem::with_id(
                translate_target_id(language),
                language,
                true,
                language.eq_ignore_ascii_case(&m.translate_target),
                None,
            ));
        }
        let _ = menu.append(&target_menu);
    }

    let _ = menu.append(&PredefinedMenuItem::separator());

    // ── Actions ────────────────────────────────────────────────────────
    let _ = menu.append(&MenuItem::with_id(
        ID_TOGGLE,
        "Toggle Recording",
        true,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(ID_SETTINGS, "Settings…", true, None));
    let _ = menu.append(&MenuItem::with_id(ID_QUIT, "Quit", true, None));

    menu
}

/// Map a menu event id to a [`TrayEvent`].
fn map_event(id: &str, model: &MenuModel) -> Option<TrayEvent> {
    if id == ID_QUIT {
        return Some(TrayEvent::Quit);
    }
    if id == ID_SETTINGS {
        return Some(TrayEvent::OpenSettings);
    }
    if id == ID_TOGGLE {
        return Some(TrayEvent::ToggleRecording);
    }
    if let Some(name) = id.strip_prefix("asr:") {
        if model.asr_engines.iter().any(|e| e == name) {
            return Some(TrayEvent::SetEngine(name.to_string()));
        }
    }
    if let Some(mode) = id.strip_prefix("inject:") {
        if mode == "keyboard" || mode == "clipboard" {
            return Some(TrayEvent::SetInjectMode(mode.to_string()));
        }
    }
    if let Some(name) = id.strip_prefix("tts:") {
        if model.tts_engines.iter().any(|e| e == name) {
            return Some(TrayEvent::SetTtsEngine(name.to_string()));
        }
    }
    if let Some(mode) = id.strip_prefix("ttsinput:") {
        if mode == "selection" || mode == "clipboard" {
            return Some(TrayEvent::SetTtsInputMode(mode.to_string()));
        }
    }
    if let Some(name) = id.strip_prefix("ttsvoice:") {
        if model
            .tts_voice_profiles
            .iter()
            .any(|profile| profile == name)
        {
            return Some(TrayEvent::SetTtsVoiceProfile(name.to_string()));
        }
    }
    if let Some(route) = id.strip_prefix("translate-route:") {
        if route == "inbound" || route == "outbound" {
            return Some(TrayEvent::SetTranslateRoute(route.to_string()));
        }
    }
    if let Some(language) = id.strip_prefix("translate-target:") {
        return Some(TrayEvent::SetTranslateTarget(language.to_string()));
    }
    if let Some(mode) = id.strip_prefix("crisper-mode:") {
        if mode == "intended" || mode == "literal" {
            return Some(TrayEvent::SetCrisperMode(mode.to_string()));
        }
    }
    if let Some(mode) = id.strip_prefix("recordmode:") {
        if mode == "ptt" || mode == "toggle" {
            return Some(TrayEvent::SetRecordMode(mode.to_string()));
        }
    }
    None
}

/// Spawn the tray icon on a dedicated thread and return a channel for
/// sending commands to it. Tray events are delivered via `event_sender`.
pub fn spawn_tray(
    initial_model: MenuModel,
    event_sender: crossbeam::channel::Sender<TrayEvent>,
) -> (
    crossbeam::channel::Sender<TrayCommand>,
    std::thread::JoinHandle<()>,
) {
    let (cmd_tx, cmd_rx) = crossbeam::channel::unbounded::<TrayCommand>();

    let handle = std::thread::spawn(move || {
        // ── Icon ───────────────────────────────────────────────────────
        let icon = match create_default_icon() {
            Ok(i) => i,
            Err(e) => {
                log::error!(
                    "Failed to create tray icon: {}. \
                     The tray UI is unavailable, but hotkeys and CLI subcommands \
                     still work. On Linux, ensure a supported desktop session \
                     (X11/GTK or Wayland) is running.",
                    e
                );
                return;
            }
        };

        // ── Initial menu ───────────────────────────────────────────────
        let menu = build_menu(&initial_model);

        let tray = match TrayIconBuilder::new()
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(initial_model.tooltip())
            .build()
        {
            Ok(t) => t,
            Err(e) => {
                log::error!(
                    "Failed to build tray icon: {}. \
                     The tray UI is unavailable, but hotkeys and CLI subcommands \
                     still work. On Linux, ensure a supported desktop session \
                     (X11/GTK or Wayland) is running.",
                    e
                );
                return;
            }
        };

        log::info!("Tray icon created on dedicated thread");

        // Current model, updated on each RefreshMenu command. The menu
        // event listener reads it to resolve which item was clicked.
        let model = std::sync::Arc::new(std::sync::Mutex::new(initial_model));
        let model_for_listener = model.clone();

        // ── Menu event listener ────────────────────────────────────────
        let evt_sender = event_sender.clone();
        std::thread::spawn(move || {
            let rx = MenuEvent::receiver();
            while let Ok(event) = rx.recv() {
                let id = event.id().0.as_str().to_string();
                let current = match model_for_listener.lock() {
                    Ok(g) => g.clone(),
                    // Poisoned: nothing useful to do; skip this event.
                    Err(_) => continue,
                };
                if let Some(ev) = map_event(&id, &current) {
                    let is_quit = matches!(ev, TrayEvent::Quit);
                    let _ = evt_sender.send(ev);
                    if is_quit {
                        break;
                    }
                }
            }
        });

        // ── Command listener + message pump ───────────────────────────
        #[cfg(target_os = "windows")]
        pump_windows_messages(&tray, &cmd_rx, &model);

        #[cfg(not(target_os = "windows"))]
        block_on_commands(&tray, &cmd_rx, &model);

        log::info!("Tray thread exiting.");
    });

    (cmd_tx, handle)
}

/// Build a [`MenuModel`] snapshot from the live app context (used by main).
pub fn refresh_menu(cmd_tx: &crossbeam::channel::Sender<TrayCommand>, model: MenuModel) {
    let _ = cmd_tx.send(TrayCommand::RefreshMenu(model));
}

// ── Windows message pump ─────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn pump_windows_messages(
    tray: &TrayIcon,
    cmd_rx: &crossbeam::channel::Receiver<TrayCommand>,
    model: &std::sync::Arc<std::sync::Mutex<MenuModel>>,
) {
    use std::ptr;

    unsafe {
        loop {
            // Drain commands (non-blocking).
            loop {
                match cmd_rx.try_recv() {
                    Ok(TrayCommand::RefreshMenu(new_model)) => {
                        let tip = new_model.tooltip();
                        if let Ok(mut g) = model.lock() {
                            *g = new_model.clone();
                        }
                        let menu = build_menu(&new_model);
                        tray.set_menu(Some(Box::new(menu)));
                        let _ = tray.set_tooltip(Some(tip));
                    }
                    Ok(TrayCommand::Shutdown)
                    | Err(crossbeam::channel::TryRecvError::Disconnected) => return,
                    Err(crossbeam::channel::TryRecvError::Empty) => break,
                }
            }

            // Pump all pending Windows messages.
            let mut msg: windows_sys::Win32::UI::WindowsAndMessaging::MSG = std::mem::zeroed();
            while windows_sys::Win32::UI::WindowsAndMessaging::PeekMessageW(
                &mut msg,
                ptr::null_mut(),
                0,
                0,
                1,
            ) != 0
            {
                if msg.message == windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT {
                    return;
                }
                windows_sys::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
                windows_sys::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn block_on_commands(
    tray: &TrayIcon,
    cmd_rx: &crossbeam::channel::Receiver<TrayCommand>,
    model: &std::sync::Arc<std::sync::Mutex<MenuModel>>,
) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            TrayCommand::RefreshMenu(new_model) => {
                let tip = new_model.tooltip();
                if let Ok(mut g) = model.lock() {
                    *g = new_model.clone();
                }
                let menu = build_menu(&new_model);
                tray.set_menu(Some(Box::new(menu)));
                let _ = tray.set_tooltip(Some(tip));
            }
            TrayCommand::Shutdown => break,
        }
    }
}

// ── Icon generation ─────────────────────────────────────────────────

/// Create a simple 32×32 RGBA icon programmatically (microphone shape).
fn create_default_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    let width = 32;
    let height = 32;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);

    for y in 0..height {
        for x in 0..width {
            let cx = 15.5;
            let cy = 15.5;
            let dx = (x as f64) - cx;
            let dy = (y as f64) - cy;
            let dist = (dx * dx + dy * dy).sqrt();

            let is_in_circle = dist < 11.0;
            let is_stand = (20..26).contains(&y) && (13..19).contains(&x);
            let is_base = (26..28).contains(&y) && (10..22).contains(&x);

            if is_in_circle || is_stand || is_base {
                rgba.push(64);
                rgba.push(160);
                rgba.push(255);
                rgba.push(255);
            } else {
                rgba.push(0);
                rgba.push(0);
                rgba.push(0);
                rgba.push(0);
            }
        }
    }

    let icon = Icon::from_rgba(rgba, width, height)?;
    Ok(icon)
}
