//! Local, always-on-top captions sourced from the microphone router's native
//! WASAPI system-audio tap.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::asr::crisper_whisper::CrisperWhisperEngine;
use crate::asr::AsrEngine;
use crate::config::{Config, ConfigManager};
use crate::translation::Translator;

pub fn run(translate: bool) -> Result<(), Box<dyn std::error::Error>> {
    let manager = ConfigManager::load_or_create()?;
    let config = manager.read().clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    spawn_caption_worker(config.clone(), translate, tx, stop.clone());

    let title = if translate {
        "Vox — Live English Subtitles"
    } else {
        "Vox — Live Subtitles"
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1200.0, 180.0])
            .with_min_inner_size([420.0, 90.0])
            .with_always_on_top()
            .with_decorations(false)
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(move |_cc| {
            Ok(Box::new(SubtitleApp {
                rx,
                lines: VecDeque::new(),
                stop,
                font_size: config.subtitles.font_size.clamp(14.0, 72.0),
                max_lines: config.subtitles.max_lines.clamp(1, 10),
            }))
        }),
    )?;
    Ok(())
}

struct SubtitleApp {
    rx: std::sync::mpsc::Receiver<String>,
    lines: VecDeque<String>,
    stop: Arc<AtomicBool>,
    font_size: f32,
    max_lines: usize,
}

impl Drop for SubtitleApp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl eframe::App for SubtitleApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(line) = self.rx.try_recv() {
            if !line.trim().is_empty() {
                self.lines.push_back(line);
                while self.lines.len() > self.max_lines {
                    self.lines.pop_front();
                }
            }
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::Frame::NONE
            .fill(egui::Color32::from_black_alpha(205))
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(18))
            .show(ui, |ui| {
                let response = ui.allocate_response(ui.available_size(), egui::Sense::drag());
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                let text = if self.lines.is_empty() {
                    "Listening to system audio…".to_string()
                } else {
                    self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
                };
                ui.put(
                    response.rect,
                    egui::Label::new(
                        egui::RichText::new(text)
                            .size(self.font_size)
                            .color(egui::Color32::WHITE),
                    )
                    .wrap()
                    .halign(egui::Align::Center),
                );
            });
        ctx.request_repaint_after(Duration::from_millis(75));
    }
}

fn spawn_caption_worker(
    config: Config,
    translate: bool,
    sender: std::sync::mpsc::Sender<String>,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => {
                log::error!("Could not create subtitle runtime: {error}");
                return;
            }
        };
        runtime.block_on(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(45))
                .build()
                .expect("subtitle HTTP client");
            let crisper = CrisperWhisperEngine::new(&config.asr.crisper);
            let translator = Translator::new(&config.translate);
            let seconds = config.subtitles.chunk_seconds.clamp(0.5, 15.0);
            let url = format!(
                "{}/v1/system-audio/take?min_seconds={seconds}",
                config.subtitles.router_url.trim_end_matches('/')
            );
            let mut router_was_down = false;
            while !stop.load(Ordering::Acquire) {
                let response = match client.get(&url).send().await {
                    Ok(response) => {
                        router_was_down = false;
                        response
                    }
                    Err(error) => {
                        if !router_was_down {
                            log::warn!("Subtitle router unavailable: {error}");
                            router_was_down = true;
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                if response.status() == reqwest::StatusCode::NO_CONTENT {
                    tokio::time::sleep(Duration::from_millis(125)).await;
                    continue;
                }
                if !response.status().is_success() {
                    log::warn!("Subtitle router returned {}", response.status());
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                let wav = match response.bytes().await {
                    Ok(wav) => wav,
                    Err(error) => {
                        log::warn!("Could not read subtitle audio: {error}");
                        continue;
                    }
                };
                let transcript = match crisper.transcribe(&wav).await {
                    Ok(text) => text,
                    Err(error) => {
                        log::debug!("Subtitle audio produced no transcript: {error}");
                        continue;
                    }
                };
                let text = if translate {
                    match translator.translate(&transcript).await {
                        Ok(translated) => translated,
                        Err(error) => {
                            log::warn!("Subtitle translation failed: {error}");
                            transcript
                        }
                    }
                } else {
                    transcript
                };
                if sender.send(text).is_err() {
                    break;
                }
            }
        });
    });
}
