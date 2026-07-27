//! Minimal settings window using egui.
//!
//! ## Decoupling
//! The window holds a plain `Config` *snapshot* and edits it in memory. On
//! save it sends the edited snapshot back to the main thread through a
//! channel; the main thread owns persistence (writing TOML) and applies the
//! changes live. This module never touches the filesystem or `toml` — it is
//! a pure view over a plain-data struct.

use eframe::egui;

use crate::config::Config;

/// Spawn the settings window in a new thread.
///
/// `initial` is the current config snapshot to edit. `save_tx` receives the
/// edited snapshot when the user saves, so the backend can persist + apply it.
pub fn spawn_settings_window(initial: Config, save_tx: crossbeam::channel::Sender<Config>) {
    std::thread::spawn(move || {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([500.0, 640.0])
                .with_resizable(true),
            ..Default::default()
        };

        let app = SettingsApp {
            config: initial,
            save_tx,
        };

        if let Err(e) =
            eframe::run_native("vox Settings", options, Box::new(|_cc| Ok(Box::new(app))))
        {
            log::error!("Settings window error: {}", e);
        }
    });
}

// ---------------------------------------------------------------------------
// Egui app
// ---------------------------------------------------------------------------

struct SettingsApp {
    config: Config,
    save_tx: crossbeam::channel::Sender<Config>,
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("vox Settings");
                ui.separator();
                ui.add_space(8.0);

                self.render_hotkeys(ui);
                self.render_asr(ui);
                self.render_inject(ui);
                self.render_tts(ui);
                self.render_general(ui);

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("💾 Save && Close").clicked() {
                        // Hand the edited snapshot to the backend; it owns
                        // persistence and live application.
                        let _ = self.save_tx.send(self.config.clone());
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
    fn render_hotkeys(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Hotkeys").strong());
        ui.horizontal(|ui| {
            ui.label("Record Toggle:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.record_toggle,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Switch Engine: ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.engine_switch,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Switch Inject: ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.inject_mode_switch,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("TTS Trigger:   ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.tts_trigger,
            ));
        });
        ui.add_space(12.0);
    }

    fn render_asr(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("ASR Engine").strong());
        egui::ComboBox::from_label("Primary")
            .selected_text(&self.config.asr.primary_engine)
            .show_ui(ui, |ui| {
                for name in [
                    "whisper-cpp",
                    "openai",
                    "mimo",
                    "aliyun",
                    "whisper-local",
                    "doubao-asr",
                ] {
                    ui.selectable_value(
                        &mut self.config.asr.primary_engine,
                        name.to_string(),
                        name,
                    );
                }
            });
        ui.add_space(6.0);

        // whisper.cpp
        ui.label(egui::RichText::new("whisper.cpp (local HTTP)").strong());
        ui.horizontal(|ui| {
            ui.label("Base URL:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.whisper_cpp.base_url,
            ));
        });
        ui.add_space(6.0);

        // OpenAI-compatible
        ui.label(egui::RichText::new("OpenAI-compatible").strong());
        ui.horizontal(|ui| {
            ui.label("Base URL:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.openai.base_url,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("API Key: ");
            ui.add(egui::TextEdit::singleline(&mut self.config.asr.openai.api_key).password(true));
        });
        ui.horizontal(|ui| {
            ui.label("Model:   ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.openai.model,
            ));
        });
        ui.add_space(6.0);

        // Mimo
        ui.label(egui::RichText::new("Mimo ASR").strong());
        ui.horizontal(|ui| {
            ui.label("Base URL:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.mimo.base_url,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("API Key: ");
            ui.add(egui::TextEdit::singleline(&mut self.config.asr.mimo.api_key).password(true));
        });
        ui.horizontal(|ui| {
            ui.label("Model:   ");
            ui.add(egui::TextEdit::singleline(&mut self.config.asr.mimo.model));
        });
        ui.add_space(6.0);

        // Aliyun
        ui.label(egui::RichText::new("Aliyun ASR").strong());
        ui.horizontal(|ui| {
            ui.label("Appkey: ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.aliyun.appkey,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Token:  ");
            ui.add(egui::TextEdit::singleline(&mut self.config.asr.aliyun.token).password(true));
        });
        ui.add_space(6.0);

        // Doubao (Volcano Engine) - shared API key for ASR + TTS
        ui.label(egui::RichText::new("Doubao ASR / TTS (shared key)").strong());
        ui.horizontal(|ui| {
            ui.label("API Key:");
            ui.add(egui::TextEdit::singleline(&mut self.config.asr.doubao.api_key).password(true));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Agent Plan subscription key. Shared by ASR + TTS.")
                    .small(),
            );
        });
        ui.add_space(12.0);
    }

    fn render_inject(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Input").strong());

        ui.horizontal(|ui| {
            ui.label("Inject Mode:");
            egui::ComboBox::from_id_salt("inject_mode")
                .selected_text(&self.config.inject.mode)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.inject.mode,
                        "keyboard".to_string(),
                        "Keyboard",
                    );
                    ui.selectable_value(
                        &mut self.config.inject.mode,
                        "clipboard".to_string(),
                        "Clipboard",
                    );
                });
        });

        ui.horizontal(|ui| {
            ui.label("Record Mode:");
            let label = match self.config.general.record_mode.as_str() {
                "toggle" => "Toggle (press)",
                _ => "Push-to-Talk (hold)",
            };
            egui::ComboBox::from_id_salt("record_mode")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.general.record_mode,
                        "ptt".to_string(),
                        "Push-to-Talk (hold Alt+`)",
                    );
                    ui.selectable_value(
                        &mut self.config.general.record_mode,
                        "toggle".to_string(),
                        "Toggle (press Alt+`)",
                    );
                });
        });

        ui.add_space(12.0);
    }

    fn render_tts(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("TTS").strong());
        egui::ComboBox::from_label("Engine")
            .selected_text(&self.config.tts.primary_engine)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.config.tts.primary_engine,
                    "edge-tts".to_string(),
                    "edge-tts (free)",
                );
                ui.selectable_value(
                    &mut self.config.tts.primary_engine,
                    "mimo-tts".to_string(),
                    "mimo-tts",
                );
                ui.selectable_value(
                    &mut self.config.tts.primary_engine,
                    "doubao-tts".to_string(),
                    "doubao-tts",
                );
            });
        egui::ComboBox::from_label("Input")
            .selected_text(&self.config.tts.input_mode)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.config.tts.input_mode,
                    "selection".to_string(),
                    "Selection (Ctrl+C)",
                );
                ui.selectable_value(
                    &mut self.config.tts.input_mode,
                    "clipboard".to_string(),
                    "Clipboard",
                );
            });
        ui.add_space(6.0);

        // Edge TTS
        ui.label(egui::RichText::new("Edge TTS").strong());
        ui.horizontal(|ui| {
            ui.label("Voice:");
            ui.add(egui::TextEdit::singleline(&mut self.config.tts.edge.voice));
        });
        ui.horizontal(|ui| {
            ui.label("Rate:  ");
            ui.add(egui::TextEdit::singleline(&mut self.config.tts.edge.rate));
        });
        ui.horizontal(|ui| {
            ui.label("Volume:");
            ui.add(egui::TextEdit::singleline(&mut self.config.tts.edge.volume));
        });
        ui.horizontal(|ui| {
            ui.label("Pitch: ");
            ui.add(egui::TextEdit::singleline(&mut self.config.tts.edge.pitch));
        });
        ui.add_space(6.0);

        // Doubao TTS (seed-tts-2.0). API key is shared with [asr.doubao].
        ui.label(egui::RichText::new("Doubao TTS (shares ASR key)").strong());
        ui.horizontal(|ui| {
            ui.label("Speaker:     ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.tts.doubao.speaker,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Speech Rate:");
            ui.add(egui::DragValue::new(&mut self.config.tts.doubao.speech_rate).range(-50..=100));
            ui.label("[-50..100]");
        });
        ui.horizontal(|ui| {
            ui.label("Loudness:    ");
            ui.add(
                egui::DragValue::new(&mut self.config.tts.doubao.loudness_rate).range(-50..=100),
            );
            ui.label("[-50..100]");
        });
        ui.horizontal(|ui| {
            ui.label("Sample Rate:");
            let mut sr = self.config.tts.doubao.sample_rate as i32;
            ui.add(egui::DragValue::new(&mut sr).range(8000..=48000));
            self.config.tts.doubao.sample_rate = sr.max(1) as u32;
        });
        ui.add_space(12.0);
    }

    fn render_general(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("General").strong());
        ui.checkbox(&mut self.config.general.autostart, "Autostart");
        ui.add_space(16.0);
    }
}
