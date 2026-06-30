//! Minimal settings window using egui.
//!
//! Spawned from the tray menu "Settings". Runs in its own thread so the
//! main event loop is not blocked. Reads/writes the TOML config file.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

use crate::config::Config;

/// Spawn the settings window in a new thread.
/// Returns immediately; the window runs independently.
///
/// `reload_tx` is notified after the user saves, so the running app can
/// reload the config and apply changes live (no restart needed).
pub fn spawn_settings_window(
    config_path: &std::path::Path,
    reload_tx: crossbeam::channel::Sender<()>,
) {
    let path = config_path.to_path_buf();
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    std::thread::spawn(move || {
        let config = load_config(&path);

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([480.0, 520.0]),
            ..Default::default()
        };

        let app = SettingsApp {
            path,
            config: config.into(),
            running: running_clone,
            reload_tx,
        };

        if let Err(e) = eframe::run_native(
            "vox Settings",
            options,
            Box::new(|_cc| Ok(Box::new(app))),
        ) {
            log::error!("Settings window error: {}", e);
        }
    });
}

fn load_config(path: &std::path::Path) -> Config {
    if path.exists() {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Config::default()
    }
}

// ---------------------------------------------------------------------------
// Egui app
// ---------------------------------------------------------------------------

struct SettingsApp {
    path: std::path::PathBuf,
    config: Box<Config>,
    running: Arc<AtomicBool>,
    reload_tx: crossbeam::channel::Sender<()>,
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.running.load(Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("vox Settings");
                ui.separator();
                ui.add_space(8.0);

                // Hotkeys
                ui.label(egui::RichText::new("Hotkeys").strong());
                ui.horizontal(|ui| {
                    ui.label("Record Toggle:");
                    ui.add(egui::TextEdit::singleline(&mut self.config.hotkey.record_toggle));
                });
                ui.horizontal(|ui| {
                    ui.label("Switch Engine: ");
                    ui.add(egui::TextEdit::singleline(&mut self.config.hotkey.engine_switch));
                });
                ui.horizontal(|ui| {
                    ui.label("Switch Inject: ");
                    ui.add(egui::TextEdit::singleline(&mut self.config.hotkey.inject_mode_switch));
                });
                ui.add_space(12.0);

                // Engine selection
                ui.label(egui::RichText::new("Engine").strong());
                egui::ComboBox::from_label("Primary")
                    .selected_text(&self.config.asr.primary_engine)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.config.asr.primary_engine, "mimo".to_string(), "Mimo AI");
                        ui.selectable_value(&mut self.config.asr.primary_engine, "openai".to_string(), "OpenAI Whisper");
                        ui.selectable_value(&mut self.config.asr.primary_engine, "aliyun".to_string(), "Aliyun ASR");
                        ui.selectable_value(&mut self.config.asr.primary_engine, "whisper-local".to_string(), "Whisper Local");
                    });
                ui.add_space(12.0);

                // Mimo config
                ui.label(egui::RichText::new("Mimo ASR").strong());
                ui.horizontal(|ui| {
                    ui.label("Base URL:");
                    ui.add(egui::TextEdit::singleline(&mut self.config.asr.mimo.base_url));
                });
                ui.horizontal(|ui| {
                    ui.label("API Key: ");
                    ui.add(egui::TextEdit::singleline(&mut self.config.asr.mimo.api_key).password(true));
                });
                ui.horizontal(|ui| {
                    ui.label("Model:   ");
                    ui.add(egui::TextEdit::singleline(&mut self.config.asr.mimo.model));
                });
                ui.add_space(12.0);

                // OpenAI config
                ui.label(egui::RichText::new("OpenAI Whisper").strong());
                ui.horizontal(|ui| {
                    ui.label("API Key: ");
                    ui.add(egui::TextEdit::singleline(&mut self.config.asr.openai.api_key).password(true));
                });
                ui.horizontal(|ui| {
                    ui.label("Model:   ");
                    ui.add(egui::TextEdit::singleline(&mut self.config.asr.openai.model));
                });
                ui.add_space(12.0);

                // Aliyun config
                ui.label(egui::RichText::new("Aliyun ASR").strong());
                ui.horizontal(|ui| {
                    ui.label("Appkey: ");
                    ui.add(egui::TextEdit::singleline(&mut self.config.asr.aliyun.appkey));
                });
                ui.horizontal(|ui| {
                    ui.label("Token:  ");
                    ui.add(egui::TextEdit::singleline(&mut self.config.asr.aliyun.token).password(true));
                });
                ui.add_space(12.0);

                // Inject mode
                ui.label(egui::RichText::new("Inject Mode").strong());
                egui::ComboBox::from_label("Mode")
                    .selected_text(&self.config.inject.mode)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.config.inject.mode, "keyboard".to_string(), "Keyboard");
                        ui.selectable_value(&mut self.config.inject.mode, "clipboard".to_string(), "Clipboard");
                    });
                ui.add_space(12.0);

                // General
                ui.label(egui::RichText::new("General").strong());
                ui.checkbox(&mut self.config.general.autostart, "Autostart");
                ui.add_space(16.0);

                // Save / Cancel
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("💾 Save && Close").clicked() {
                        self.save_config();
                        // Notify the running app to reload config from disk.
                        let _ = self.reload_tx.send(());
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("✕ Cancel").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });
    }
}

impl SettingsApp {
    fn save_config(&self) {
        match toml::to_string_pretty(&*self.config) {
            Ok(raw) => {
                if let Err(e) = std::fs::write(&self.path, &raw) {
                    log::error!("Failed to save config: {}", e);
                } else {
                    log::info!("Settings saved to {:?}", self.path);
                }
            }
            Err(e) => {
                log::error!("Failed to serialize config: {}", e);
            }
        }
    }
}
