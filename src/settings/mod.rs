//! Minimal settings window using egui.
//!
//! ## Decoupling
//! The window holds a plain `Config` *snapshot* and edits it in memory. On
//! save it sends the edited snapshot back to the main thread through a
//! channel; the main thread owns persistence (writing TOML) and applies the
//! changes live. This module never touches the filesystem or `toml` — it is
//! a pure view over a plain-data struct.

use eframe::egui;
use std::sync::OnceLock;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
    Routes,
    Translation,
    Hotkeys,
    SpeechInput,
    SpeechOutput,
    General,
}

struct SettingsRequest {
    initial: Config,
    save_tx: crossbeam::channel::Sender<Config>,
}

static SETTINGS_REQUESTS: OnceLock<crossbeam::channel::Sender<SettingsRequest>> = OnceLock::new();

/// Open Settings on one long-lived worker thread.
///
/// Eframe keeps Winit's event loop in thread-local storage specifically so a
/// native window can be closed and recreated. Spawning a brand-new thread for
/// every opening discarded that storage and caused Windows to reject the
/// second event loop. The worker remains alive; only the ordinary window is
/// closed and reopened.
pub fn spawn_settings_window(initial: Config, save_tx: crossbeam::channel::Sender<Config>) {
    let requests = SETTINGS_REQUESTS.get_or_init(|| {
        let (request_tx, request_rx) = crossbeam::channel::unbounded::<SettingsRequest>();
        std::thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                run_settings_window(request);
            }
        });
        request_tx
    });

    if let Err(error) = requests.send(SettingsRequest { initial, save_tx }) {
        log::error!("Could not open Settings: {}", error);
    }
}

fn run_settings_window(request: SettingsRequest) {
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 780.0])
            .with_resizable(true),
        ..Default::default()
    };

    // Vox owns its tray/message pump on the process main thread and the
    // settings window intentionally lives on a worker thread. Winit rejects
    // the first creation on Windows unless this explicit contract is enabled.
    #[cfg(target_os = "windows")]
    {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }));
    }

    let app = SettingsApp {
        config: request.initial,
        save_tx: request.save_tx,
        page: SettingsPage::Routes,
    };

    if let Err(error) =
        eframe::run_native("vox Settings", options, Box::new(|_cc| Ok(Box::new(app))))
    {
        log::error!("Settings window error: {}", error);
    }
}

// ---------------------------------------------------------------------------
// Egui app
// ---------------------------------------------------------------------------

struct SettingsApp {
    config: Config,
    save_tx: crossbeam::channel::Sender<Config>,
    page: SettingsPage,
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.heading("vox Settings");
            ui.horizontal_wrapped(|ui| {
                for (page, label) in [
                    (SettingsPage::Routes, "Routes"),
                    (SettingsPage::Translation, "Translation backend"),
                    (SettingsPage::Hotkeys, "Legacy hotkeys"),
                    (SettingsPage::SpeechInput, "Speech input"),
                    (SettingsPage::SpeechOutput, "Speech output"),
                    (SettingsPage::General, "General"),
                ] {
                    ui.selectable_value(&mut self.page, page, label);
                }
            });
            ui.separator();
            let content_height = (ui.available_height() - 44.0).max(160.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(content_height)
                .show(ui, |ui| match self.page {
                    SettingsPage::Routes => self.render_routes(ui),
                    SettingsPage::Translation => self.render_translate(ui),
                    SettingsPage::Hotkeys => self.render_hotkeys(ui),
                    SettingsPage::SpeechInput => {
                        self.render_inject(ui);
                        self.render_asr(ui);
                    }
                    SettingsPage::SpeechOutput => self.render_tts(ui),
                    SettingsPage::General => self.render_general(ui),
                });
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
    }
}

impl SettingsApp {
    fn render_routes(&mut self, ui: &mut egui::Ui) {
        ui.heading("Programmable local routes");
        ui.label("Each row is independent: its own source, Crisper language, optional translation target, destination, tray action, and live-reloadable hotkey.");
        ui.label(egui::RichText::new("Caption routes launch independent windows and router cursors, so native and translated views—even two translations from the same device—can remain active together.").small());
        ui.add_space(8.0);

        egui::Grid::new("route-overview")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                ui.strong("On");
                ui.strong("Route");
                ui.strong("Flow");
                ui.strong("Hotkey");
                ui.end_row();
                for preset in &self.config.route_presets {
                    ui.label(if preset.enabled { "✓" } else { "—" });
                    ui.label(&preset.name);
                    ui.label(preset.summary());
                    ui.monospace(if preset.hotkey.trim().is_empty() {
                        "tray only"
                    } else {
                        &preset.hotkey
                    });
                    ui.end_row();
                }
            });
        ui.add_space(10.0);

        let mut remove = None;
        for (index, preset) in self.config.route_presets.iter_mut().enumerate() {
            ui.push_id(format!("route-editor-{index}"), |ui| {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut preset.enabled, "Enabled");
                        ui.label("Name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut preset.name)
                                .desired_width(360.0),
                        );
                        ui.label("Hotkey:");
                        ui.add(
                            egui::TextEdit::singleline(&mut preset.hotkey)
                                .desired_width(130.0)
                                .hint_text("blank = tray only"),
                        );
                        if ui.button("Remove").clicked() {
                            remove = Some(index);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Input:");
                        egui::ComboBox::from_id_salt("input")
                            .selected_text(&preset.input)
                            .show_ui(ui, |ui| {
                                for (value, label) in [
                                    ("microphone", "Microphone"),
                                    ("system", "System audio"),
                                    ("selection", "Selected text"),
                                    ("clipboard", "Clipboard text"),
                                ] {
                                    ui.selectable_value(
                                        &mut preset.input,
                                        value.to_string(),
                                        label,
                                    );
                                }
                            });
                        if preset.is_audio_input() {
                            Self::spoken_language_combo(
                                ui,
                                "source-language",
                                "Crisper source",
                                &mut preset.source_language,
                            );
                            egui::ComboBox::from_id_salt("transcript-mode")
                                .selected_text(&preset.transcript_mode)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut preset.transcript_mode,
                                        "intended".into(),
                                        "Intended",
                                    );
                                    ui.selectable_value(
                                        &mut preset.transcript_mode,
                                        "literal".into(),
                                        "Literal",
                                    );
                                });
                        }
                        ui.checkbox(&mut preset.translate, "Translate");
                        if preset.translate {
                            ui.label("Target:");
                            ui.add(
                                egui::TextEdit::singleline(&mut preset.target_language)
                                    .desired_width(110.0),
                            );
                        }
                        ui.label("Output:");
                        egui::ComboBox::from_id_salt("output")
                            .selected_text(&preset.output)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut preset.output,
                                    "caption".into(),
                                    "Caption window",
                                );
                                ui.selectable_value(
                                    &mut preset.output,
                                    "clipboard".into(),
                                    "Clipboard text",
                                );
                                ui.selectable_value(
                                    &mut preset.output,
                                    "mic_forwarder".into(),
                                    "TTS → microphone",
                                );
                            });
                    });
                    ui.monospace(preset.summary());
                    if preset.input == "system" && preset.output != "caption" {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "System-audio presets currently use continuous caption output; use vox-http for one-shot system-audio routing.",
                        );
                    }
                    if preset.is_text_input() && preset.output == "caption" {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "Text input needs Clipboard or TTS → microphone output.",
                        );
                    }
                });
            });
            ui.add_space(6.0);
        }
        if let Some(index) = remove {
            self.config.route_presets.remove(index);
        }
        if ui.button("＋ Add route").clicked() {
            let number = self.config.route_presets.len() + 1;
            self.config
                .route_presets
                .push(crate::config::RoutePresetConfig {
                    id: format!("user-route-{number}"),
                    name: format!("User route {number}"),
                    enabled: true,
                    hotkey: String::new(),
                    input: "clipboard".into(),
                    source_language: "detect".into(),
                    transcript_mode: "intended".into(),
                    translate: false,
                    target_language: "English".into(),
                    output: "clipboard".into(),
                });
        }

        ui.add_space(16.0);
        ui.label(egui::RichText::new("Live-caption engine").strong());
        ui.horizontal(|ui| {
            ui.label("Microphone router:");
            ui.add(
                egui::TextEdit::singleline(&mut self.config.subtitles.router_url)
                    .desired_width(330.0),
            );
            ui.label("Rolling window:");
            ui.add(
                egui::DragValue::new(&mut self.config.subtitles.chunk_seconds)
                    .range(0.5..=15.0)
                    .suffix("s"),
            );
            ui.label("Font:");
            ui.add(egui::DragValue::new(&mut self.config.subtitles.font_size).range(14.0..=72.0));
            ui.label("Lines:");
            ui.add(egui::DragValue::new(&mut self.config.subtitles.max_lines).range(1..=10));
        });
    }

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
            ui.label("Seed +1:       ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.tts_seed_increment,
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Seed −1:       ");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.tts_seed_decrement,
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
            ui.label("Speech → translated text:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.hotkey.record_translate_text,
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
            Self::spoken_language_combo(
                ui,
                "crisper-primary-language",
                "Language",
                &mut self.config.asr.crisper.language,
            );
            ui.label("Hotwords:");
            ui.add(egui::TextEdit::singleline(
                &mut self.config.asr.crisper.hotwords,
            ));
        });
        let detect_language = self
            .config
            .asr
            .crisper
            .language
            .eq_ignore_ascii_case("detect");
        if detect_language {
            ui.label(
                egui::RichText::new(
                    "Optional detect lane selected — language is resolved independently for this utterance only.",
                )
                .strong(),
            );
        } else {
            ui.label(
                egui::RichText::new(
                    "Fixed-language fast path — one full Crisper pass; the MITM is bypassed completely.",
                )
                .small(),
            );
        }
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
        if detect_language {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label("MITM service:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.config.asr.crisper.mitm_url)
                            .desired_width(260.0),
                    );
                    ui.label("Candidate tokens:");
                    ui.add(
                        egui::DragValue::new(
                            &mut self.config.asr.crisper.candidate_max_new_tokens,
                        )
                        .range(1..=128),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "All Crisper languages decode in one low-token batch. XLM-R cheaply narrows the prompt, EraX-VL rejects fragments, and a high-confidence acoustic prior resolves obvious cases. If multiple complete rows remain, only those rows are translated for one tiny final comparison. The chosen token is used for one full Crisper pass and is never latched.",
                    )
                    .small(),
                );
            });
        }
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
        ui.checkbox(
            &mut self.config.inject.restore_clipboard,
            "Restore clipboard afterwards",
        );
        ui.checkbox(&mut self.config.inject.copy_only, "Copy only (no paste)");
        ui.label(
            egui::RichText::new(
                "These switches apply globally. Copy only leaves STT output on the clipboard and skips keyboard/paste injection.",
            )
            .small(),
        );

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
        egui::ComboBox::from_label("Synthesized audio output")
            .selected_text(match self.config.tts.output.mode.as_str() {
                "clipboard_wav" => "Clipboard WAV file",
                "mic_forwarder" => "Microphone router",
                _ => "Speakers",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.config.tts.output.mode,
                    "playback".to_string(),
                    "Speakers",
                );
                ui.selectable_value(
                    &mut self.config.tts.output.mode,
                    "clipboard_wav".to_string(),
                    "Clipboard WAV file",
                );
                ui.selectable_value(
                    &mut self.config.tts.output.mode,
                    "mic_forwarder".to_string(),
                    "Microphone router",
                );
            });
        ui.horizontal(|ui| {
            ui.label("Router URL:");
            ui.add(
                egui::TextEdit::singleline(&mut self.config.tts.output.mic_forwarder_url)
                    .desired_width(330.0),
            );
        });
        ui.label(
            egui::RichText::new(
                "Clipboard output publishes vox-output.wav as a pasteable Windows file. The router is the separately built native mixer.",
            )
            .small(),
        );
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
                    if ui.button("Browse…").clicked() {
                        if let Some(path) = pick_audio_file() {
                            profile.audio_path = path;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Transcript:");
                    ui.add(
                        egui::TextEdit::singleline(&mut profile.transcript_path)
                            .desired_width(430.0)
                            .hint_text("Path to verbatim UTF-8 .txt"),
                    );
                    if ui.button("Browse…").clicked() {
                        if let Some(path) = pick_text_file() {
                            profile.transcript_path = path;
                        }
                    }
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
                if ui.button("Browse…").clicked() {
                    if let Some(path) = pick_audio_file() {
                        self.config.tts.longcat.prompt_audio_path = path;
                    }
                }
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
        });
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.config.tts.longcat.concatenate_chunks,
                "Automatically concatenate sentence-bounded LongCat requests",
            );
            ui.label("Characters per request:");
            ui.add(
                egui::DragValue::new(&mut self.config.tts.longcat.characters_per_request)
                    .range(1..=4000),
            );
        });
        ui.label(egui::RichText::new("When enabled, Vox scans backward from the character ceiling to the previous period, question mark, exclamation mark, or equivalent full-width terminator, then joins the resulting WAVs in order. It never cuts inside a sentence. When disabled, LongCat receives the complete text once and Vox uses that one WAV directly.").small());
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
        ui.heading("Private translation backend");
        ui.label("Multilingual EraX translation and the optional EraX-VL/XLM spoken-language MITM remain entirely local. Configure end-to-end actions on the Routes page.");
        ui.add_space(6.0);
        ui.checkbox(
            &mut self.config.translate.enabled,
            "Enable the local translation backend",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Backend:");
            ui.monospace(&self.config.translate.base_url);
            if ui.button("Use local default").clicked() {
                self.config.translate.base_url = "http://127.0.0.1:8176/v1".to_string();
                self.config.translate.api_key.clear();
                self.config.translate.model.clear();
                self.config.translate.max_tokens = 256;
            }
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
        ui.label(egui::RichText::new("A blank model discovers the model advertised by the configured local /models endpoint.").small());
        ui.add_space(10.0);

        ui.label(egui::RichText::new("Default behavior of the ordinary hotkeys").strong());
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.config.translate.asr,
                "Record hotkey translates before text injection",
            );
            ui.checkbox(
                &mut self.config.translate.tts,
                "TTS hotkey translates before speech",
            );
        });
        ui.label(egui::RichText::new("The dedicated translation hotkeys below remain explicit; these two switches only compose translation into the ordinary record and TTS actions.").small());
        ui.add_space(10.0);

        ui.label(egui::RichText::new("Active language route").strong());
        egui::ComboBox::from_label("Active route")
            .selected_text(&self.config.translate.active_route)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.config.translate.active_route,
                    "inbound".to_string(),
                    "Inbound: any source → English",
                );
                ui.selectable_value(
                    &mut self.config.translate.active_route,
                    "outbound".to_string(),
                    "Outbound: any source → selected language",
                );
            });
        Self::render_language_route(ui, "inbound", "Inbound", &mut self.config.translate.inbound);
        Self::render_language_route(
            ui,
            "outbound",
            "Outbound",
            &mut self.config.translate.outbound,
        );
        ui.add_space(10.0);

        ui.label("System prompt:");
        ui.add(egui::TextEdit::multiline(&mut self.config.translate.system_prompt).desired_rows(3));
        ui.label(egui::RichText::new("EraX detects its own source language and is instructed to emit only the selected target language. The MITM token is used only by CrisperWhisper.").small());
        ui.add_space(12.0);
    }

    fn spoken_language_combo(ui: &mut egui::Ui, id: &str, label: &str, value: &mut String) {
        egui::ComboBox::from_id_salt(id)
            .selected_text(format!("{label}: {value}"))
            .show_ui(ui, |ui| {
                for (code, name) in [
                    ("detect", "Detect automatically"),
                    ("en", "English"),
                    ("es", "Spanish"),
                    ("de", "German"),
                    ("fr", "French"),
                    ("it", "Italian"),
                    ("pt", "Portuguese"),
                    ("nl", "Dutch"),
                    ("pl", "Polish"),
                    ("ru", "Russian"),
                    ("uk", "Ukrainian"),
                    ("zh", "Chinese"),
                    ("ja", "Japanese"),
                    ("ko", "Korean"),
                    ("hi", "Hindi"),
                    ("tr", "Turkish"),
                    ("ar", "Arabic"),
                ] {
                    ui.selectable_value(value, code.to_string(), format!("{name} ({code})"));
                }
            });
    }

    fn render_language_route(
        ui: &mut egui::Ui,
        id: &str,
        label: &str,
        route: &mut crate::config::TranslateRouteConfig,
    ) {
        ui.group(|ui| {
            ui.label(egui::RichText::new(label).strong());

            route.source_language = "auto".to_string();
            ui.label(
                egui::RichText::new("Source: automatic inside EraX (no source token required)")
                    .small(),
            );

            let mut target_mode = if route.target_language.eq_ignore_ascii_case("English") {
                0
            } else {
                1
            };
            ui.horizontal_wrapped(|ui| {
                ui.label("Target:");
                ui.radio_value(&mut target_mode, 0, "English");
                ui.radio_value(&mut target_mode, 1, "Selected language");
                if target_mode == 0 {
                    route.target_language = "English".to_string();
                } else {
                    if route.target_language.eq_ignore_ascii_case("English") {
                        route.target_language = "Spanish".to_string();
                    }
                    egui::ComboBox::from_id_salt(format!("{id}-target-list"))
                        .selected_text(&route.target_language)
                        .show_ui(ui, |ui| {
                            for language in [
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
                                    &mut route.target_language,
                                    language.to_string(),
                                    language,
                                );
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut route.target_language)
                            .desired_width(150.0)
                            .hint_text("or type any language"),
                    );
                }
            });
        });
    }

    fn render_general(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("General").strong());
        ui.checkbox(&mut self.config.general.autostart, "Autostart");
        ui.add_space(16.0);
    }
}

#[cfg(windows)]
fn pick_audio_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Audio", &["wav", "mp3", "m4a"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(not(windows))]
fn pick_audio_file() -> Option<String> {
    None
}

#[cfg(windows)]
fn pick_text_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Text", &["txt"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(not(windows))]
fn pick_text_file() -> Option<String> {
    None
}
