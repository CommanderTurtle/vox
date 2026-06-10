//! System tray icon and context menu for vox.
//!
//! On Windows, the tray icon **must** live on a thread that pumps
//! Windows messages, otherwise right-click and menu events won't work.
//! This module spawns a dedicated thread for that purpose.

use tray_icon::{Icon, TrayIconBuilder};
pub use tray_icon::TrayIcon;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, Submenu};

use crate::app::state::AppState;

/// Events emitted by the tray (user interactions).
#[derive(Debug, Clone)]
pub enum TrayEvent {
    Quit,
    #[allow(dead_code)]
    ToggleRecording,
    SetEngine(String),
    SetInjectMode(String),
    OpenSettings,
    SetTtsEngine(String),
    SetTtsInputMode(String),
}

/// Commands sent FROM the main thread TO the tray thread.
#[derive(Debug)]
pub enum TrayCommand {
    /// Update the tooltip text.
    SetTooltip(String),
    /// Shut down the tray thread.
    Shutdown,
}

/// Spawn the tray icon on a dedicated thread and return a channel
/// for receiving user events and a sender for sending commands back.
///
/// The tray thread will run a Windows message pump (on Windows) or
/// simply block (on other platforms).  It closes when `cmd_tx` is
/// dropped or a `Shutdown` command is received.
pub fn spawn_tray(
    engines: &[String],
    tts_engines: &[String],
    event_sender: crossbeam::channel::Sender<TrayEvent>,
) -> (crossbeam::channel::Sender<TrayCommand>, std::thread::JoinHandle<()>) {
    let engine_list = engines.to_vec();
    let tts_list = tts_engines.to_vec();
    let (cmd_tx, cmd_rx) = crossbeam::channel::unbounded::<TrayCommand>();

    let handle = std::thread::spawn(move || {
        // ── Build icon ────────────────────────────────────────────
        let icon = match create_default_icon() {
            Ok(i) => i,
            Err(e) => {
                log::error!("Failed to create tray icon: {}", e);
                return;
            }
        };

        // ── ASR Engine submenu ────────────────────────────────────
        let engine_menu = Submenu::new("ASR Engine", true);
        for name in &engine_list {
            let item = MenuItem::with_id(name.clone(), name.clone(), true, None);
            if let Err(e) = engine_menu.append(&item) {
                log::error!("Failed to add menu item: {}", e);
            }
        }

        // ── Inject mode submenu ───────────────────────────────────
        let inject_menu = Submenu::new("Inject Mode", true);
        for mode in &["keyboard", "clipboard"] {
            let item = MenuItem::with_id((*mode).to_string(), *mode, true, None);
            if let Err(e) = inject_menu.append(&item) {
                log::error!("Failed to add menu item: {}", e);
            }
        }

        // ── TTS Engine submenu ────────────────────────────────────
        let tts_engine_menu = Submenu::new("TTS Engine", true);
        for name in &tts_list {
            let item = MenuItem::with_id(format!("tts_{}", name), name, true, None);
            if let Err(e) = tts_engine_menu.append(&item) {
                log::error!("Failed to add menu item: {}", e);
            }
        }

        // ── TTS Input mode submenu ────────────────────────────────
        let tts_input_menu = Submenu::new("TTS Input", true);
        for mode in &["selection", "clipboard"] {
            let display = if *mode == "selection" { "Selection (Ctrl+C)" } else { "Clipboard" };
            let item = MenuItem::with_id(format!("tts_input_{}", mode), display, true, None);
            if let Err(e) = tts_input_menu.append(&item) {
                log::error!("Failed to add menu item: {}", e);
            }
        }

        // ── Main menu ─────────────────────────────────────────────
        let menu = Menu::new();
        if let Err(e) = menu.append(&engine_menu) { log::error!("menu: {}", e); }
        if let Err(e) = menu.append(&inject_menu) { log::error!("menu: {}", e); }
        if let Err(e) = menu.append(&tts_engine_menu) { log::error!("menu: {}", e); }
        if let Err(e) = menu.append(&tts_input_menu) { log::error!("menu: {}", e); }
        if let Err(e) = menu.append(&MenuItem::with_id("settings", "Settings", true, None)) { log::error!("menu: {}", e); }
        if let Err(e) = menu.append(&MenuItem::with_id("quit", "Quit", true, None)) { log::error!("menu: {}", e); }

        // ── Tray icon ─────────────────────────────────────────────
        let tray = match TrayIconBuilder::new()
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("vox — Idle")
            .build()
        {
            Ok(t) => t,
            Err(e) => {
                log::error!("Failed to build tray icon: {}", e);
                return;
            }
        };

        log::info!("Tray icon created on dedicated thread");

        // ── Menu event listener ───────────────────────────────────
        let evt_sender = event_sender.clone();
        let eng_list = engine_list.clone();
        let tts_list_clone = tts_list.clone();
        std::thread::spawn(move || {
            let rx = MenuEvent::receiver();
            while let Ok(event) = rx.recv() {
                let id = event.id().0.as_str();
                match id {
                    "quit" => { let _ = evt_sender.send(TrayEvent::Quit); break; }
                    "settings" => { let _ = evt_sender.send(TrayEvent::OpenSettings); }
                    name if eng_list.iter().any(|e| e == name) => {
                        let _ = evt_sender.send(TrayEvent::SetEngine(name.to_string()));
                    }
                    "keyboard" | "clipboard" => {
                        let _ = evt_sender.send(TrayEvent::SetInjectMode(id.to_string()));
                    }
                    // TTS engine: prefix "tts_" followed by engine name
                    tts_name if tts_list_clone.iter().any(|e| id == &format!("tts_{}", e)) => {
                        let engine = tts_name.strip_prefix("tts_").unwrap_or(tts_name);
                        let _ = evt_sender.send(TrayEvent::SetTtsEngine(engine.to_string()));
                    }
                    // TTS input mode: prefix "tts_input_"
                    "tts_input_selection" => {
                        let _ = evt_sender.send(TrayEvent::SetTtsInputMode("selection".to_string()));
                    }
                    "tts_input_clipboard" => {
                        let _ = evt_sender.send(TrayEvent::SetTtsInputMode("clipboard".to_string()));
                    }
                    _ => {}
                }
            }
        });

        // ── Command listener + message pump ───────────────────────
        // On Windows we need to pump messages for the tray icon.
        // On other platforms we just block on the command channel.
        #[cfg(target_os = "windows")]
        pump_windows_messages(&tray, &cmd_rx);

        #[cfg(not(target_os = "windows"))]
        block_on_commands(&cmd_rx);

        // Tray is dropped here when we exit — clean shutdown.
        log::info!("Tray thread exiting.");
    });

    (cmd_tx, handle)
}

/// Update the tray tooltip (sends a command to the tray thread).
pub fn update_tray_state(cmd_tx: &crossbeam::channel::Sender<TrayCommand>, state: AppState) {
    let tip = match state {
        AppState::Idle => "vox — Idle",
        AppState::Recording => "vox — Recording...",
        AppState::Transcribing => "vox — Transcribing...",
    };
    let _ = cmd_tx.send(TrayCommand::SetTooltip(tip.to_string()));
}

// ── Windows message pump ─────────────────────────────────────────────

/// Run a Windows message pump while also listening for tray commands.
/// Uses PeekMessageW with a sleep so we can also check the command channel.
#[cfg(target_os = "windows")]
fn pump_windows_messages(
    tray: &TrayIcon,
    cmd_rx: &crossbeam::channel::Receiver<TrayCommand>,
) {
    use std::ptr;

    unsafe {
        loop {
            // Check for commands from the main thread (non-blocking)
            loop {
                match cmd_rx.try_recv() {
                    Ok(TrayCommand::SetTooltip(text)) => {
                        let _ = tray.set_tooltip(Some(text));
                    }
                    Ok(TrayCommand::Shutdown) | Err(crossbeam::channel::TryRecvError::Disconnected) => {
                        return;
                    }
                    Err(crossbeam::channel::TryRecvError::Empty) => break,
                }
            }

            // Pump all pending Windows messages
            let mut msg: windows_sys::Win32::UI::WindowsAndMessaging::MSG =
                std::mem::zeroed();
            while windows_sys::Win32::UI::WindowsAndMessaging::
                PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, 1) != 0
            {
                if msg.message == windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT {
                    return;
                }
                windows_sys::Win32::UI::WindowsAndMessaging::
                    TranslateMessage(&msg);
                windows_sys::Win32::UI::WindowsAndMessaging::
                    DispatchMessageW(&msg);
            }

            // Sleep a bit before polling again
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn block_on_commands(cmd_rx: &crossbeam::channel::Receiver<TrayCommand>) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            TrayCommand::SetTooltip(_) => {
                // On non-Windows we don't hold the TrayIcon reference here,
                // so tooltip updates from commands aren't supported this way.
                log::warn!("Tooltip update not supported on this platform via tray thread");
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
            let is_stand = (y >= 20 && y < 26) && (x >= 13 && x < 19);
            let is_base = (y >= 26 && y < 28) && (x >= 10 && x < 22);

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
