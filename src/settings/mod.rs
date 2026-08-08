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
                .with_inner_size([620.0, 720.0])
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
                self.render_translate(ui);
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
        ui.horizontal(|ui| {
            ui.label("Switch Voice:  ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.tts_voice_switch,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Translate Text:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.translate_text,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Translate + TTS:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.translate_tts,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Speech → TTS:  ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.record_translate_tts,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Raw STT → TTS: ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.record_tts,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Switch Route:  ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.translate_route_switch,
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
                    "crisper-whisper",
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

        ui.label(egui::RichText::new("CrisperWhisper 2.0 (local)").strong());
        ui.horizontal(|ui| {
            ui.label("Base URL:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.crisper.base_url,
            ));
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_label("Transcript")
                .selected_text(&self.config.asr.crisper.mode)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.asr.crisper.mode,
                        "intended".to_string(),
                        "Intended / non-literal",
                    );
                    ui.selectable_value(
                        &mut self.config.asr.crisper.mode,
                        "literal".to_string(),
                        "Literal / verbatim",
                    );
                });
            ui.label("Language:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.crisper.language,
            ));
            ui.label("Hotwords:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.crisper.hotwords,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Chunk / stride:");
            ui.add(
                egui::DragValue::new(&mut self.config.asr.crisper.chunk_duration).range(1.0..=30.0),
            );
            ui.add(egui::DragValue::new(&mut self.config.asr.crisper.stride).range(1.0..=30.0));
        });
        ui.horizontal(|ui| {
            ui.label("Context / tokens:");
            ui.add(egui::DragValue::new(&mut self.config.asr.crisper.context_words).range(0..=100));
            ui.add(
                egui::DragValue::new(&mut self.config.asr.crisper.max_new_tokens).range(1..=2048),
            );
        });
        ui.label(
            egui::RichText::new(
                "Intended mode cleans spoken disfluencies; literal mode preserves the spoken wording.",
            )
            .small(),
        );
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
                egui::RichText::new("Agent Plan subscription key. Shared by ASR + TTS.").small(),
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
                    "longcat".to_string(),
                    "longcat (local voice clone)",
                );
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

        ui.label(egui::RichText::new("LongCat (local voice clone)").strong());
        ui.horizontal(|ui| {
            ui.label("Base URL:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.tts.longcat.base_url,
            ));
        });
        let profiles = &mut self.config.tts.longcat.voice_profiles;
        if profiles.is_empty() {
            ui.label(egui::RichText::new("No named voices yet. Add one to pair a reference recording with its verbatim .txt transcript.").small());
        } else {
            egui::ComboBox::from_label("Active voice")
                .selected_text(self.config.tts.longcat.active_voice_profile.clone())
                .show_ui(ui, |ui| {
                    for profile in profiles.iter() {
                        ui.selectable_value(
                            &mut self.config.tts.longcat.active_voice_profile,
                            profile.name.clone(),
                            &profile.name,
                        );
                    }
                });
        }

        let mut remove_profile = None;
        for (index, profile) in profiles.iter_mut().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Voice {}", index + 1));
                    ui.add(egui::TextEdit::singleline(&mut profile.name).hint_text("Profile name"));
                    if ui.button("Remove").clicked() {
                        remove_profile = Some(index);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Audio:");
                    ui.add(
                        egui::TextEdit::singleline(&mut profile.audio_path)
                            .desired_width(460.0)
                            .hint_text("Path to .wav, .mp3, or .m4a"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Transcript:");
                    ui.add(
                        egui::TextEdit::singleline(&mut profile.transcript_path)
                            .desired_width(430.0)
                            .hint_text("Path to verbatim UTF-8 .txt"),
                    );
                });
            });
        }
        if let Some(index) = remove_profile {
            let removed = profiles.remove(index);
            if self.config.tts.longcat.active_voice_profile == removed.name {
                self.config.tts.longcat.active_voice_profile = profiles
                    .first()
                    .map(|profile| profile.name.clone())
                    .unwrap_or_default();
            }
        }
        if ui.button("＋ Add voice pair").clicked() {
            let name = format!("Voice {}", profiles.len() + 1);
            profiles.push(crate::config::LongCatVoiceProfile {
                name: name.clone(),
                audio_path: String::new(),
                transcript_path: String::new(),
            });
            if self.config.tts.longcat.active_voice_profile.is_empty() {
                self.config.tts.longcat.active_voice_profile = name;
            }
        }
        ui.collapsing("Legacy inline pair", |ui| {
            ui.label(egui::RichText::new("Used only when no named voice profiles exist.").small());
            ui.horizontal(|ui| {
                ui.label("Reference audio:");
                ui.add(egui::TextEdit::singleline(
                    &mut self.config.tts.longcat.prompt_audio_path,
                ));
            });
            ui.label("Verbatim reference transcript:");
            ui.add(
                egui::TextEdit::multiline(&mut self.config.tts.longcat.prompt_text).desired_rows(3),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Steps:");
            ui.add(egui::DragValue::new(&mut self.config.tts.longcat.steps).range(2..=64));
            ui.label("Guidance:");
            ui.add(
                egui::DragValue::new(&mut self.config.tts.longcat.guidance_strength)
                    .range(0.0..=20.0),
            );
            egui::ComboBox::from_id_salt("longcat_guidance")
                .selected_text(&self.config.tts.longcat.guidance_method)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.tts.longcat.guidance_method,
                        "apg".into(),
                        "APG",
                    );
                    ui.selectable_value(
                        &mut self.config.tts.longcat.guidance_method,
                        "cfg".into(),
                        "CFG",
                    );
                });
        });
        ui.horizontal(|ui| {
            ui.label("Seed:");
            ui.add(egui::DragValue::new(&mut self.config.tts.longcat.seed));
            ui.label("Duration scale:");
            ui.add(
                egui::DragValue::new(&mut self.config.tts.longcat.duration_scale).range(0.5..=2.0),
            );
            ui.label("Max chunk:");
            ui.add(
                egui::DragValue::new(&mut self.config.tts.longcat.max_chunk_seconds)
                    .range(1.0..=20.0)
                    .suffix("s"),
            );
        });
        ui.label(egui::RichText::new("Vox sentence-splits every request at or below 20 seconds; reference conditioning receives an additional safety margin.").small());
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

    fn render_translate(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Local Translation").strong());
        ui.checkbox(&mut self.config.translate.enabled, "Enable translation");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.translate.asr, "ASR → injection");
            ui.checkbox(&mut self.config.translate.tts, "TTS input → speech");
        });
        ui.horizontal(|ui| {
            ui.label("Base URL:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.translate.base_url,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("API Key:");
            ui.add(egui::TextEdit::singleline(&mut self.config.translate.api_key).password(true));
            ui.label("Model:");
            ui.add(egui::TextEdit::singleline(&mut self.config.translate.model));
            ui.label("Max tokens:");
            ui.add(egui::DragValue::new(&mut self.config.translate.max_tokens).range(16..=4096));
        });
        egui::ComboBox::from_label("Active route")
            .selected_text(&self.config.translate.active_route)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.config.translate.active_route,
                    "inbound".to_string(),
                    "Inbound: source/detect → English",
                );
                ui.selectable_value(
                    &mut self.config.translate.active_route,
                    "outbound".to_string(),
                    "Outbound: English → selected language",
                );
            });
        ui.horizontal(|ui| {
            ui.label("Inbound:");
            ui.add(
                egui::TextEdit::singleline(&mut self.config.translate.inbound.source_language)
                    .hint_text("auto"),
            );
            ui.label("→");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.translate.inbound.target_language,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Outbound:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.translate.outbound.source_language,
            ));
            ui.label("→");
            egui::ComboBox::from_id_salt("outbound_target_language")
                .selected_text(&self.config.translate.outbound.target_language)
                .show_ui(ui, |ui| {
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
                        ui.selectable_value(
                            &mut self.config.translate.outbound.target_language,
                            language.to_string(),
                            language,
                        );
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.config.translate.outbound.target_language)
                    .hint_text("or type any language"),
            );
        });
        ui.label("System prompt:");
        ui.add(egui::TextEdit::multiline(&mut self.config.translate.system_prompt).desired_rows(3));
        ui.label(egui::RichText::new("Default: the standalone local translation service on port 8176. Blank model auto-selects its advertised model. No cloud fallback.").small());
        ui.add_space(12.0);
    }

    fn render_general(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("General").strong());
        ui.checkbox(&mut self.config.general.autostart, "Autostart");
        ui.add_space(16.0);
    }
}
