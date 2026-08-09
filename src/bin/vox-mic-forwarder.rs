//! Native microphone router for Vox.
//!
//! Captures one or more Windows input devices, mixes them with WAV audio
//! posted by Vox, and renders the combined signal to a selected output
//! endpoint. Select the playback side of a virtual audio cable as the output;
//! applications then use that cable's capture side as their microphone.

use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use crossbeam::queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tiny_http::{Header, Method, Response, Server, StatusCode};

const VB_CABLE_RENDER_MARKER: &str = "cable input";
const VB_CABLE_CAPTURE_MARKER: &str = "cable output";
const VB_CABLE_VENDOR_MARKER: &str = "vb-audio";
const CONSUMER_RETENTION_FRAMES: u64 = 8;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct InputConfig {
    /// "default", an exact Windows device name, or a unique name fragment.
    name: String,
    #[serde(default = "one")]
    gain: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RouterConfig {
    #[serde(default = "default_bind")]
    bind: String,
    #[serde(default = "default_inputs")]
    inputs: Vec<InputConfig>,
    /// "default", an exact output endpoint, or a unique name fragment.
    #[serde(default = "default_device")]
    output_device: String,
    /// Native WASAPI loopback tap for local subtitle consumers. This stream
    /// is deliberately not mixed into the routed microphone output.
    #[serde(default = "default_true")]
    system_audio_enabled: bool,
    #[serde(default = "default_device")]
    system_audio_device: String,
    #[serde(default = "default_system_audio_rate")]
    system_audio_sample_rate: u32,
    #[serde(default = "one")]
    injected_gain: f32,
    #[serde(default = "default_queue_seconds")]
    queue_seconds: usize,
}

fn one() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}
fn default_system_audio_rate() -> u32 {
    16_000
}
fn default_bind() -> String {
    "127.0.0.1:8182".into()
}
fn default_device() -> String {
    "default".into()
}
fn default_queue_seconds() -> usize {
    300
}
fn default_inputs() -> Vec<InputConfig> {
    vec![
        InputConfig {
            name: "default".into(),
            gain: 1.0,
        },
        InputConfig {
            name: String::new(),
            gain: 1.0,
        },
    ]
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            inputs: default_inputs(),
            output_device: default_device(),
            system_audio_enabled: true,
            system_audio_device: default_device(),
            system_audio_sample_rate: default_system_audio_rate(),
            injected_gain: 1.0,
            queue_seconds: default_queue_seconds(),
        }
    }
}

#[derive(Clone)]
struct Route {
    name: String,
    gain: f32,
    samples: Arc<ArrayQueue<f32>>,
}

#[derive(Clone)]
struct Mixer {
    inputs: Vec<Route>,
    injected: Arc<ArrayQueue<f32>>,
    injected_gain: f32,
    output_rate: u32,
    output_channels: usize,
    /// Physical microphone mix before generated audio is added. This gives
    /// captions a clean microphone lane without LongCat feeding itself.
    microphone_audio: AudioTap,
    /// Native WASAPI loopback from the selected physical playback endpoint.
    /// It is never mixed into the virtual microphone output.
    system_audio: Option<AudioTap>,
}

/// One bounded capture history with independent cursors for every consumer.
/// Subtitle windows therefore never steal chunks from one another, and the
/// HTTP-only router can observe the same source concurrently.
#[derive(Clone)]
struct AudioTap {
    name: String,
    sample_rate: u32,
    capacity: usize,
    state: Arc<Mutex<AudioTapState>>,
}

struct AudioTapState {
    samples: VecDeque<f32>,
    start_sequence: u64,
    next_sequence: u64,
    consumers: HashMap<String, ConsumerCursor>,
    peak: f32,
}

struct ConsumerCursor {
    sequence: u64,
    last_request: u64,
}

impl AudioTap {
    fn new(name: impl Into<String>, sample_rate: u32, queue_seconds: usize) -> Self {
        Self {
            name: name.into(),
            sample_rate,
            capacity: sample_rate as usize * queue_seconds.clamp(5, 3600),
            state: Arc::new(Mutex::new(AudioTapState {
                samples: VecDeque::new(),
                start_sequence: 0,
                next_sequence: 0,
                consumers: HashMap::new(),
                peak: 0.0,
            })),
        }
    }

    fn push_locked(state: &mut AudioTapState, capacity: usize, sample: f32) {
        state.peak = (state.peak * 0.94).max(sample.abs());
        if state.samples.len() == capacity {
            state.samples.pop_front();
            state.start_sequence += 1;
        }
        state.samples.push_back(sample);
        state.next_sequence += 1;
    }

    fn buffered_seconds(&self) -> f32 {
        self.state
            .lock()
            .map(|state| state.samples.len() as f32 / self.sample_rate as f32)
            .unwrap_or_default()
    }

    fn peak(&self) -> f32 {
        self.state
            .lock()
            .map(|state| state.peak)
            .unwrap_or_default()
    }

    fn clear(&self) -> usize {
        let mut state = self.state.lock().expect("audio tap lock poisoned");
        let cleared = state.samples.len();
        state.samples.clear();
        state.start_sequence = state.next_sequence;
        let next = state.next_sequence;
        for cursor in state.consumers.values_mut() {
            cursor.sequence = next;
        }
        cleared
    }

    fn take(
        &self,
        consumer: &str,
        minimum_seconds: f32,
        latest_seconds: Option<f32>,
    ) -> Result<Option<Vec<u8>>, String> {
        let minimum = (self.sample_rate as f32 * minimum_seconds) as u64;
        let keep = latest_seconds.map(|seconds| (self.sample_rate as f32 * seconds) as u64);
        let mut state = self.state.lock().map_err(|_| "audio tap lock poisoned")?;
        let request = state.next_sequence;
        let start = state.start_sequence;
        let end = state.next_sequence;
        let cursor = state
            .consumers
            .entry(consumer.to_string())
            .or_insert(ConsumerCursor {
                sequence: end,
                last_request: request,
            });
        cursor.sequence = cursor.sequence.max(start).min(end);
        cursor.last_request = request;
        let mut from = cursor.sequence;
        if let Some(keep) = keep {
            from = from.max(end.saturating_sub(keep));
        }
        if end.saturating_sub(from) < minimum {
            return Ok(None);
        }
        let offset = from.saturating_sub(start) as usize;
        let samples = state
            .samples
            .iter()
            .skip(offset)
            .copied()
            .collect::<Vec<_>>();
        if let Some(cursor) = state.consumers.get_mut(consumer) {
            cursor.sequence = end;
        }
        state.consumers.retain(|_, cursor| {
            end.saturating_sub(cursor.last_request)
                <= self.capacity as u64 * CONSUMER_RETENTION_FRAMES
        });
        drop(state);
        encode_wav(&samples, self.sample_rate).map(Some)
    }
}

/// Stateful linear conversion between independent Windows endpoint rates.
///
/// CPAL opens each endpoint in its native shared-mode format. Keeping the
/// interpolation state across callbacks avoids the audible sample-repeat/drop
/// behavior of nearest-neighbor conversion when a physical microphone is not
/// already running at the virtual cable's 48 kHz rate.
struct LinearResampler {
    step: f64,
    previous: Option<f32>,
    input_index: u64,
    next_output_position: f64,
}

impl LinearResampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            step: input_rate.max(1) as f64 / output_rate.max(1) as f64,
            previous: None,
            input_index: 0,
            next_output_position: 0.0,
        }
    }

    fn push(&mut self, sample: f32, mut emit: impl FnMut(f32)) {
        let Some(previous) = self.previous else {
            emit(sample);
            self.previous = Some(sample);
            self.input_index = 1;
            self.next_output_position = self.step;
            return;
        };

        let current_position = self.input_index as f64;
        let previous_position = current_position - 1.0;
        while self.next_output_position <= current_position + f64::EPSILON {
            let fraction = (self.next_output_position - previous_position).clamp(0.0, 1.0) as f32;
            emit(previous + (sample - previous) * fraction);
            self.next_output_position += self.step;
        }
        self.previous = Some(sample);
        self.input_index += 1;
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let host = cpal::default_host();
    if std::env::args().any(|argument| argument == "--list-devices") {
        print_devices(&host)?;
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--verify-cable") {
        verify_vb_cable(&host)?;
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--init-config") {
        let path = initialize_config(&host)?;
        println!("Configuration ready: {}", path.display());
        return Ok(());
    }

    let (path, config) = load_config()?;
    log::info!("Router config: {}", path.display());
    let output = select_output(&host, &config.output_device)?;
    let supported = output.default_output_config()?;
    let output_format = supported.sample_format();
    let output_config = supported.config();
    let output_rate = output_config.sample_rate.0;
    let output_channels = output_config.channels as usize;
    let output_name = output
        .name()
        .unwrap_or_else(|_| config.output_device.clone());
    let capacity = output_rate as usize * config.queue_seconds.clamp(5, 3600);

    let mut routes = Vec::new();
    let mut input_devices = Vec::new();
    for input in config
        .inputs
        .iter()
        .filter(|input| !input.name.trim().is_empty())
    {
        let device = select_input(&host, &input.name)?;
        let name = device.name().unwrap_or_else(|_| input.name.clone());
        if is_vb_cable_capture(&name) && is_vb_cable_render(&output_name) {
            return Err(format!(
                "'{name}' is the recording side of '{output_name}', not a physical microphone; selecting both would feed the virtual cable into itself"
            )
            .into());
        }
        if input_devices
            .iter()
            .any(|(existing, _): &(String, Device)| existing == &name)
        {
            log::warn!("Skipping duplicate input device: {name}");
            continue;
        }
        let route = Route {
            name: name.clone(),
            gain: input.gain,
            samples: Arc::new(ArrayQueue::new(capacity)),
        };
        routes.push(route);
        input_devices.push((name, device));
    }
    if routes.is_empty() {
        return Err("no input devices are configured".into());
    }

    let system_audio_rate = config.system_audio_sample_rate.clamp(8_000, 96_000);
    let system_audio = config
        .system_audio_enabled
        .then(|| AudioTap::new("system audio", system_audio_rate, config.queue_seconds));
    let mixer = Mixer {
        inputs: routes,
        injected: Arc::new(ArrayQueue::new(capacity)),
        injected_gain: config.injected_gain,
        output_rate,
        output_channels,
        microphone_audio: AudioTap::new("physical microphone", output_rate, config.queue_seconds),
        system_audio,
    };

    log::info!(
        "Routing {} input(s) + Vox injection to '{}' at {} Hz / {} channel(s)",
        mixer.inputs.len(),
        output_name,
        output_rate,
        output_channels
    );
    for route in &mixer.inputs {
        log::info!("Input: '{}' (gain {:.2})", route.name, route.gain);
    }

    let mut streams = Vec::new();
    for ((_, device), route) in input_devices.into_iter().zip(mixer.inputs.iter().cloned()) {
        streams.push(build_input_stream(
            &device,
            Some(route),
            output_rate,
            false,
            None,
        )?);
    }
    if let Some(tap) = &mixer.system_audio {
        let device = select_output(&host, &config.system_audio_device)?;
        let name = device
            .name()
            .unwrap_or_else(|_| config.system_audio_device.clone());
        if name.eq_ignore_ascii_case(&output_name) {
            return Err(format!(
                "system_audio_device and output_device both resolve to '{name}'; select physical speakers or headphones for the system-audio tap"
            )
            .into());
        }
        streams.push(build_input_stream(
            &device,
            None,
            tap.sample_rate,
            true,
            Some(tap.clone()),
        )?);
        log::info!(
            "System-audio subtitle tap: '{}' at {} Hz",
            name,
            tap.sample_rate
        );
    }
    streams.push(build_output_stream(
        &output,
        &output_config,
        output_format,
        mixer.clone(),
    )?);
    for stream in &streams {
        stream.play()?;
    }

    let server_mixer = mixer.clone();
    let bind = config.bind.clone();
    std::thread::spawn(move || {
        if let Err(error) = serve(&bind, server_mixer) {
            log::error!("Router API stopped: {error}");
        }
    });

    log::info!("Vox microphone router ready at http://{}", config.bind);
    log::info!("Press Ctrl+C to stop; audio streams remain active while this process runs.");
    if std::env::args().any(|argument| argument == "--headless") {
        loop {
            std::thread::park();
        }
    }

    let ui_mixer = mixer;
    let title = "Vox Microphone Router";
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([760.0, 470.0])
            .with_min_inner_size([560.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(move |_cc| Ok(Box::new(RouterApp::new(ui_mixer, output_name)))),
    )
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    drop(streams);
    Ok(())
}

struct RouterApp {
    mixer: Mixer,
    output_name: String,
    media_path: String,
    status: String,
}

impl RouterApp {
    fn new(mixer: Mixer, output_name: String) -> Self {
        Self {
            mixer,
            output_name,
            media_path: String::new(),
            status: "Ready. Physical microphone routing remains live.".into(),
        }
    }
}

impl eframe::App for RouterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("Vox microphone router");
        ui.label(format!(
            "Output: {} — {} Hz / {} channel(s)",
            self.output_name, self.mixer.output_rate, self.mixer.output_channels
        ));
        ui.label(format!(
            "Live inputs: {}",
            self.mixer
                .inputs
                .iter()
                .map(|route| route.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        let queued_seconds = self.mixer.injected.len() as f32 / self.mixer.output_rate as f32;
        let queue_capacity_seconds =
            self.mixer.injected.capacity() as f32 / self.mixer.output_rate as f32;
        ui.add_space(10.0);
        egui::Grid::new("router-live-state")
            .num_columns(2)
            .spacing([12.0, 7.0])
            .show(ui, |ui| {
                ui.label("Physical microphone");
                ui.add(
                    egui::ProgressBar::new(self.mixer.microphone_audio.peak().clamp(0.0, 1.0))
                        .desired_width(420.0)
                        .show_percentage(),
                );
                ui.end_row();
                ui.label("Hoisted media queue");
                ui.add(
                    egui::ProgressBar::new(
                        (queued_seconds / queue_capacity_seconds.max(0.001)).clamp(0.0, 1.0),
                    )
                    .desired_width(420.0)
                    .text(format!(
                        "{queued_seconds:.1}s remaining / {queue_capacity_seconds:.0}s"
                    )),
                );
                ui.end_row();
                if let Some(system) = &self.mixer.system_audio {
                    ui.label("System playback tap");
                    ui.add(
                        egui::ProgressBar::new(system.peak().clamp(0.0, 1.0))
                            .desired_width(420.0)
                            .text(format!("{:.1}s buffered", system.buffered_seconds())),
                    );
                    ui.end_row();
                }
            });
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Hoist media to the virtual microphone").strong());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.media_path)
                    .hint_text("WAV, MP3, or M4A path")
                    .desired_width(510.0),
            );
            if ui.button("Browse…").clicked() {
                if let Some(path) = pick_media_file() {
                    self.media_path = path;
                }
            }
        });
        if ui.button("Hoist to microphone").clicked() {
            self.status = match std::fs::read(self.media_path.trim_matches([' ', '\"', '\''])) {
                Ok(audio) => match enqueue_audio(&audio, &self.mixer) {
                    Ok(report) => format!(
                        "Queued {:.1}s from {} Hz / {} channel(s). The live meter counts down as Vox sends it to CABLE Input.",
                        report.seconds, report.source_rate, report.source_channels
                    ),
                    Err(error) => format!("Could not hoist media: {error}"),
                },
                Err(error) => format!("Could not read media: {error}"),
            };
        }
        ui.add_space(10.0);
        ui.label(&self.status);
        ui.add_space(18.0);
        ui.separator();
        ui.label(
            egui::RichText::new(
                "The router uses each selected Windows device's native shared-mode format. It does not codec-compress audio; it only downmixes to the microphone bus and resamples when device rates differ. Restart the router after changing Windows' default devices.",
            )
            .small(),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
}

#[cfg(windows)]
fn pick_media_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Audio", &["wav", "mp3", "m4a"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(not(windows))]
fn pick_media_file() -> Option<String> {
    None
}

fn config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.parent()
                .map(|parent| parent.join("mic-forwarder.toml"))
        })
        .unwrap_or_else(|| PathBuf::from("mic-forwarder.toml"))
}

fn load_config() -> Result<(PathBuf, RouterConfig), Box<dyn std::error::Error + Send + Sync>> {
    let path = config_path();
    if path.exists() {
        let config = toml::from_str(&std::fs::read_to_string(&path)?)?;
        return Ok((path, config));
    }
    let config = RouterConfig::default();
    std::fs::write(&path, toml::to_string_pretty(&config)?)?;
    Ok((path, config))
}

fn initialize_config(
    host: &cpal::Host,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let path = config_path();
    if path.exists() {
        println!("A configuration already exists at {}.", path.display());
        if !confirm("Replace it using the device wizard? [y/N] ")? {
            println!("Existing configuration left unchanged.");
            return Ok(path);
        }
    }

    let input_names = device_names(host.input_devices()?);
    let output_names = device_names(host.output_devices()?);
    if input_names.is_empty() {
        return Err("Windows reported no input devices".into());
    }
    if output_names.is_empty() {
        return Err("Windows reported no output devices".into());
    }
    let default_input = host.default_input_device().and_then(device_name);
    let default_output = host.default_output_device().and_then(device_name);

    println!("\nVox mixes one or more physical microphones with generated audio.");
    println!("Its OUTPUT should be the playback side of a virtual audio cable.");
    println!("Other applications then select that cable's recording side as their microphone.\n");

    let recommended_output = output_names
        .iter()
        .find(|name| is_vb_cable_standard_render(name))
        .map(String::as_str);
    let inputs = choose_inputs(&input_names, default_input.as_deref())?;
    let output = choose_one(
        "output",
        &output_names,
        default_output.as_deref(),
        recommended_output,
        "Select output device number",
    )?;
    if !looks_virtual(&output) {
        println!(
            "\nWarning: '{output}' does not look like a virtual-cable endpoint.\n\
             Vox can play/monitor through it, but Windows applications will not see the mix as a microphone."
        );
    }

    let system_outputs = output_names
        .iter()
        .filter(|name| !name.eq_ignore_ascii_case(&output) && !is_vb_cable_render(name))
        .cloned()
        .collect::<Vec<_>>();
    let (system_audio_enabled, system_audio_device) = if system_outputs.is_empty() {
        println!(
            "\nNo separate physical playback endpoint is available; the system-audio subtitle tap is disabled."
        );
        (false, default_device())
    } else {
        let physical_default = default_output
            .as_deref()
            .filter(|name| system_outputs.iter().any(|candidate| candidate == name));
        let recommended_physical = physical_default.or_else(|| {
            system_outputs
                .iter()
                .find(|name| !looks_virtual(name))
                .map(String::as_str)
        });
        let device = choose_one(
            "system-audio playback",
            &system_outputs,
            physical_default,
            recommended_physical,
            "Select physical playback device for subtitles",
        )?;
        (true, device)
    };

    let config = RouterConfig {
        inputs: inputs
            .into_iter()
            .map(|name| InputConfig { name, gain: 1.0 })
            .collect(),
        output_device: output,
        system_audio_enabled,
        system_audio_device,
        ..RouterConfig::default()
    };
    let rendered = format!(
        "# Generated by vox-mic-forwarder --init-config.\n\
         # inputs: physical microphones mixed into the bus.\n\
         # output_device: playback endpoint of a virtual cable for microphone forwarding.\n\
         # system_audio_device = \"default\" follows Windows' current playback device for subtitles.\n\n{}",
        toml::to_string_pretty(&config)?
    );
    std::fs::write(&path, rendered)?;
    println!("\nSaved selected device names; no TOML editing is required for this setup.");
    Ok(path)
}

fn device_names(devices: impl Iterator<Item = Device>) -> Vec<String> {
    devices.filter_map(|device| device.name().ok()).collect()
}

fn device_name(device: Device) -> Option<String> {
    device.name().ok()
}

fn choose_inputs(
    names: &[String],
    default: Option<&str>,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    print_choices("input", names, default, None);
    let fallback = default
        .and_then(|name| names.iter().position(|candidate| candidate == name))
        .filter(|index| !is_vb_cable_capture(&names[*index]))
        .or_else(|| names.iter().position(|name| !is_vb_cable_capture(name)))
        .unwrap_or(0);
    let answer = prompt(&format!(
        "Select input number(s), comma-separated [{}]: ",
        fallback + 1
    ))?;
    let selections = if answer.trim().is_empty() {
        vec![fallback]
    } else {
        let mut selections = Vec::new();
        for item in answer.split(',') {
            let index = parse_choice(item, names.len(), "input")?;
            if !selections.contains(&index) {
                selections.push(index);
            }
        }
        selections
    };
    let selected = selections
        .into_iter()
        .map(|index| names[index].clone())
        .collect::<Vec<_>>();
    if let Some(name) = selected.iter().find(|name| is_vb_cable_capture(name)) {
        return Err(format!(
            "'{name}' is VB-CABLE's virtual recording endpoint; choose a physical microphone to avoid a feedback loop"
        )
        .into());
    }
    Ok(selected)
}

fn choose_one(
    kind: &str,
    names: &[String],
    default: Option<&str>,
    recommended: Option<&str>,
    label: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    print_choices(kind, names, default, recommended);
    let fallback = recommended
        .and_then(|name| names.iter().position(|candidate| candidate == name))
        .or_else(|| default.and_then(|name| names.iter().position(|candidate| candidate == name)))
        .unwrap_or(0);
    let answer = prompt(&format!("{label} [{}]: ", fallback + 1))?;
    let index = if answer.trim().is_empty() {
        fallback
    } else {
        parse_choice(&answer, names.len(), kind)?
    };
    Ok(names[index].clone())
}

fn print_choices(kind: &str, names: &[String], default: Option<&str>, recommended: Option<&str>) {
    println!("{} devices:", capitalize(kind));
    for (index, name) in names.iter().enumerate() {
        let marker = match (
            Some(name.as_str()) == default,
            Some(name.as_str()) == recommended,
            is_vb_cable_capture(name),
        ) {
            (true, true, _) => " (recommended; Windows default)",
            (_, true, _) => " (recommended)",
            (true, _, true) => " (Windows default; do not use as a physical mic)",
            (true, _, _) => " (Windows default)",
            (_, _, true) if kind == "input" => " (virtual capture; do not select)",
            _ => "",
        };
        println!("  {}. {}{}", index + 1, name, marker);
    }
}

fn parse_choice(
    value: &str,
    count: usize,
    kind: &str,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let number: usize = value
        .trim()
        .parse()
        .map_err(|_| format!("'{value}' is not a valid {kind} device number"))?;
    if number == 0 || number > count {
        return Err(format!("{kind} device number must be between 1 and {count}").into());
    }
    Ok(number - 1)
}

fn prompt(message: &str) -> Result<String, std::io::Error> {
    print!("{message}");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().to_string())
}

fn confirm(message: &str) -> Result<bool, std::io::Error> {
    Ok(matches!(
        prompt(message)?.to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn looks_virtual(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["cable", "virtual", "voicemeeter", "vb-audio", "blackhole"]
        .iter()
        .any(|marker| name.contains(marker))
}

fn is_vb_cable_render(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains(VB_CABLE_VENDOR_MARKER)
        && (name.contains(VB_CABLE_RENDER_MARKER)
            || name.starts_with("cable in ")
            || name.starts_with("cable in("))
}

fn is_vb_cable_capture(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains(VB_CABLE_VENDOR_MARKER)
        && (name.contains(VB_CABLE_CAPTURE_MARKER)
            || name.starts_with("cable out ")
            || name.starts_with("cable out("))
}

fn is_vb_cable_standard_render(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains(VB_CABLE_RENDER_MARKER) && name.contains(VB_CABLE_VENDOR_MARKER)
}

fn print_devices(host: &cpal::Host) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let inputs = device_names(host.input_devices()?);
    let outputs = device_names(host.output_devices()?);
    let default_input = host.default_input_device().and_then(device_name);
    let default_output = host.default_output_device().and_then(device_name);
    print_choices("input", &inputs, default_input.as_deref(), None);
    print_choices("output", &outputs, default_output.as_deref(), None);
    println!("\nNext: run --init-config for a numbered setup wizard.");
    if !outputs.iter().any(|name| looks_virtual(name)) {
        println!(
            "No obvious virtual-cable output was detected. A user-space program cannot create a Windows audio endpoint; install or enable a virtual cable before routing this mix as a microphone."
        );
    }
    Ok(())
}

fn verify_vb_cable(host: &cpal::Host) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let capture = host
        .input_devices()?
        .find(|device| device.name().is_ok_and(|name| is_vb_cable_capture(&name)))
        .ok_or(
            "CABLE Output (VB-Audio Virtual Cable) is not present in Windows recording endpoints",
        )?;
    let render = host
        .output_devices()?
        .find(|device| {
            device
                .name()
                .is_ok_and(|name| is_vb_cable_standard_render(&name))
        })
        .ok_or(
            "CABLE Input (VB-Audio Virtual Cable) is not present in Windows playback endpoints",
        )?;

    let capture_name = capture.name()?;
    let capture_format = capture.default_input_config()?;
    let render_name = render.name()?;
    let render_format = render.default_output_config()?;
    println!("VB-CABLE is visible to the same CPAL/WASAPI enumerator used by the router:");
    println!(
        "  recording: '{}' — {} Hz / {} channel(s) / {:?}",
        capture_name,
        capture_format.sample_rate().0,
        capture_format.channels(),
        capture_format.sample_format()
    );
    println!(
        "  playback:  '{}' — {} Hz / {} channel(s) / {:?}",
        render_name,
        render_format.sample_rate().0,
        render_format.channels(),
        render_format.sample_format()
    );
    Ok(())
}

fn select_input(
    host: &cpal::Host,
    requested: &str,
) -> Result<Device, Box<dyn std::error::Error + Send + Sync>> {
    if requested.trim().is_empty() || requested.eq_ignore_ascii_case("default") {
        return host
            .default_input_device()
            .ok_or_else(|| "no default input device".into());
    }
    select_named(host.input_devices()?, requested, "input")
}

fn select_output(
    host: &cpal::Host,
    requested: &str,
) -> Result<Device, Box<dyn std::error::Error + Send + Sync>> {
    if requested.trim().is_empty() || requested.eq_ignore_ascii_case("default") {
        return host
            .default_output_device()
            .ok_or_else(|| "no default output device".into());
    }
    select_named(host.output_devices()?, requested, "output")
}

fn select_named<I>(
    devices: I,
    requested: &str,
    kind: &str,
) -> Result<Device, Box<dyn std::error::Error + Send + Sync>>
where
    I: Iterator<Item = Device>,
{
    let requested_lower = requested.to_ascii_lowercase();
    let mut partial = Vec::new();
    for device in devices {
        let name = device.name().unwrap_or_default();
        if name.eq_ignore_ascii_case(requested) {
            return Ok(device);
        }
        if name.to_ascii_lowercase().contains(&requested_lower) {
            partial.push((name, device));
        }
    }
    match partial.len() {
        1 => Ok(partial.remove(0).1),
        0 => Err(format!("{kind} device '{requested}' was not found; use --list-devices").into()),
        _ => Err(
            format!("{kind} device fragment '{requested}' is ambiguous; use its exact name").into(),
        ),
    }
}

fn build_input_stream(
    device: &Device,
    route: Option<Route>,
    output_rate: u32,
    loopback: bool,
    tap: Option<AudioTap>,
) -> Result<Stream, Box<dyn std::error::Error + Send + Sync>> {
    // CPAL's WASAPI backend transparently enables LOOPBACK when an input
    // stream is built on an output endpoint. Loopback must use that output
    // endpoint's shared-mode mix format.
    let supported = if loopback {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    let format = supported.sample_format();
    let config = supported.config();
    let channels = config.channels as usize;
    let input_rate = config.sample_rate.0;
    let error_name = route
        .as_ref()
        .map(|route| route.name.clone())
        .or_else(|| tap.as_ref().map(|tap| tap.name.clone()))
        .unwrap_or_else(|| "audio input".into());
    let error_callback = move |error| log::error!("Input stream '{error_name}' failed: {error}");

    let stream = match format {
        SampleFormat::F32 => {
            let mut resampler = LinearResampler::new(input_rate, output_rate);
            device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    route_input(
                        data,
                        channels,
                        input_rate,
                        output_rate,
                        route.as_ref().map(|route| route.samples.as_ref()),
                        tap.as_ref(),
                        &mut resampler,
                        |sample| sample,
                    )
                },
                error_callback,
                None,
            )?
        }
        SampleFormat::I16 => {
            let mut resampler = LinearResampler::new(input_rate, output_rate);
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    route_input(
                        data,
                        channels,
                        input_rate,
                        output_rate,
                        route.as_ref().map(|route| route.samples.as_ref()),
                        tap.as_ref(),
                        &mut resampler,
                        |sample| sample as f32 / 32768.0,
                    )
                },
                error_callback,
                None,
            )?
        }
        SampleFormat::U16 => {
            let mut resampler = LinearResampler::new(input_rate, output_rate);
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    route_input(
                        data,
                        channels,
                        input_rate,
                        output_rate,
                        route.as_ref().map(|route| route.samples.as_ref()),
                        tap.as_ref(),
                        &mut resampler,
                        |sample| (sample as f32 - 32768.0) / 32768.0,
                    )
                },
                error_callback,
                None,
            )?
        }
        other => return Err(format!("unsupported input sample format: {other:?}").into()),
    };
    Ok(stream)
}

fn route_input<T: Copy>(
    data: &[T],
    channels: usize,
    input_rate: u32,
    output_rate: u32,
    queue: Option<&ArrayQueue<f32>>,
    tap: Option<&AudioTap>,
    resampler: &mut LinearResampler,
    convert: impl Fn(T) -> f32,
) {
    debug_assert_eq!(
        resampler.step,
        input_rate.max(1) as f64 / output_rate.max(1) as f64
    );
    let mut tap_state = tap.map(|tap| tap.state.lock().expect("audio tap lock poisoned"));
    for frame in data.chunks_exact(channels.max(1)) {
        let mono = frame.iter().copied().map(&convert).sum::<f32>() / frame.len() as f32;
        resampler.push(mono, |sample| {
            if let Some(queue) = queue {
                push_latest(queue, sample);
            }
            if let (Some(tap), Some(state)) = (tap, tap_state.as_deref_mut()) {
                AudioTap::push_locked(state, tap.capacity, sample);
            }
        });
    }
}

fn build_output_stream(
    device: &Device,
    config: &StreamConfig,
    format: SampleFormat,
    mixer: Mixer,
) -> Result<Stream, Box<dyn std::error::Error + Send + Sync>> {
    let channels = config.channels as usize;
    let error_callback = |error| log::error!("Output stream failed: {error}");
    let stream = match format {
        SampleFormat::F32 => device.build_output_stream(
            config,
            move |data: &mut [f32], _| fill_output(data, channels, &mixer, |sample| sample),
            error_callback,
            None,
        )?,
        SampleFormat::I16 => device.build_output_stream(
            config,
            move |data: &mut [i16], _| {
                fill_output(data, channels, &mixer, |sample| {
                    (sample * i16::MAX as f32) as i16
                })
            },
            error_callback,
            None,
        )?,
        SampleFormat::U16 => device.build_output_stream(
            config,
            move |data: &mut [u16], _| {
                fill_output(data, channels, &mixer, |sample| {
                    (((sample + 1.0) * 0.5) * u16::MAX as f32) as u16
                })
            },
            error_callback,
            None,
        )?,
        other => return Err(format!("unsupported output sample format: {other:?}").into()),
    };
    Ok(stream)
}

fn fill_output<T: Copy>(
    data: &mut [T],
    channels: usize,
    mixer: &Mixer,
    convert: impl Fn(f32) -> T,
) {
    let mut microphone = mixer
        .microphone_audio
        .state
        .lock()
        .expect("microphone tap lock poisoned");
    for frame in data.chunks_exact_mut(channels.max(1)) {
        let mut physical = 0.0;
        for route in &mixer.inputs {
            physical += route.samples.pop().unwrap_or(0.0) * route.gain;
        }
        AudioTap::push_locked(
            &mut microphone,
            mixer.microphone_audio.capacity,
            physical.clamp(-1.0, 1.0),
        );
        let sample = physical + mixer.injected.pop().unwrap_or(0.0) * mixer.injected_gain;
        let sample = convert(sample.clamp(-1.0, 1.0));
        frame.fill(sample);
    }
}

fn push_latest(queue: &ArrayQueue<f32>, sample: f32) {
    if let Err(sample) = queue.push(sample) {
        let _ = queue.pop();
        let _ = queue.push(sample);
    }
}

fn serve(bind: &str, mixer: Mixer) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = Server::http(bind)?;
    for mut request in server.incoming_requests() {
        match (request.method(), request.url()) {
            (&Method::Get, "/health") => {
                let body = serde_json::json!({
                    "status": "ok",
                    "output_rate": mixer.output_rate,
                    "output_channels": mixer.output_channels,
                    "inputs": mixer.inputs.iter().map(|route| &route.name).collect::<Vec<_>>(),
                    "injected_frames_queued": mixer.injected.len(),
                    "injected_seconds_queued": mixer.injected.len() as f32 / mixer.output_rate as f32,
                    "microphone_audio_seconds_buffered": mixer.microphone_audio.buffered_seconds(),
                    "system_audio_enabled": mixer.system_audio.is_some(),
                    "system_audio_seconds_buffered": mixer.system_audio.as_ref().map(AudioTap::buffered_seconds).unwrap_or(0.0),
                })
                .to_string();
                let _ = request.respond(json_response(body, StatusCode(200)));
            }
            (&Method::Get, "/v1/devices") => {
                let host = cpal::default_host();
                let inputs = host
                    .input_devices()
                    .map(|devices| {
                        devices
                            .filter_map(|device| device.name().ok())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let outputs = host
                    .output_devices()
                    .map(|devices| {
                        devices
                            .filter_map(|device| device.name().ok())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let body = serde_json::json!({"inputs": inputs, "outputs": outputs}).to_string();
                let _ = request.respond(json_response(body, StatusCode(200)));
            }
            (&Method::Post, "/v1/forward") => {
                let mut audio = Vec::new();
                match request
                    .as_reader()
                    .take(512 * 1024 * 1024)
                    .read_to_end(&mut audio)
                {
                    Ok(_) => match enqueue_audio(&audio, &mixer) {
                        Ok(report) => {
                            let body = serde_json::json!({
                                "queued_frames": report.frames,
                                "queued_seconds": report.seconds,
                                "source_rate": report.source_rate,
                                "source_channels": report.source_channels,
                                "remaining_seconds": mixer.injected.len() as f32 / mixer.output_rate as f32,
                            }).to_string();
                            let _ = request.respond(json_response(body, StatusCode(202)));
                        }
                        Err(error) => {
                            let _ = request.respond(text_response(error, StatusCode(400)));
                        }
                    },
                    Err(error) => {
                        let _ = request.respond(text_response(error.to_string(), StatusCode(400)));
                    }
                }
            }
            (&Method::Post, "/v1/playback") => {
                let mut audio = Vec::new();
                match request
                    .as_reader()
                    .take(512 * 1024 * 1024)
                    .read_to_end(&mut audio)
                {
                    Ok(_) => match decode_audio(&audio) {
                        Ok(decoded) => {
                            let playback_mixer = mixer.clone();
                            std::thread::spawn(move || {
                                if let Err(error) = play_decoded_audio(decoded) {
                                    log::error!("Router playback failed: {error}");
                                }
                                // Playback is captured by the WASAPI loopback
                                // tap. Drop it after playback so subtitle/dub
                                // consumers never process their own output.
                                clear_audio_source(&playback_mixer, "system");
                            });
                            let _ = request.respond(json_response(
                                serde_json::json!({"status": "playing"}).to_string(),
                                StatusCode(202),
                            ));
                        }
                        Err(error) => {
                            let _ = request.respond(text_response(error, StatusCode(400)));
                        }
                    },
                    Err(error) => {
                        let _ = request.respond(text_response(error.to_string(), StatusCode(400)));
                    }
                }
            }
            (&Method::Post, "/v1/system-audio/clear") => {
                let cleared = clear_audio_source(&mixer, "system");
                let _ = request.respond(json_response(
                    serde_json::json!({"cleared_frames": cleared}).to_string(),
                    StatusCode(200),
                ));
            }
            (&Method::Get, url) if url.starts_with("/v1/system-audio/take") => {
                let minimum_seconds = query_number(url, "min_seconds")
                    .unwrap_or(3.0)
                    .clamp(0.25, 30.0);
                let latest_seconds =
                    query_number(url, "latest_seconds").map(|seconds| seconds.clamp(0.25, 30.0));
                let consumer = query_text(url, "consumer").unwrap_or("desktop");
                match take_audio_source(&mixer, "system", minimum_seconds, latest_seconds, consumer)
                {
                    Ok(Some(wav)) => {
                        let header = Header::from_bytes("Content-Type", "audio/wav").unwrap();
                        let response = Response::from_data(wav)
                            .with_status_code(StatusCode(200))
                            .with_header(header);
                        let _ = request.respond(response);
                    }
                    Ok(None) => {
                        let _ = request.respond(Response::empty(StatusCode(204)));
                    }
                    Err(error) => {
                        let _ = request.respond(text_response(error, StatusCode(409)));
                    }
                }
            }
            (&Method::Post, url) if url.starts_with("/v1/audio/clear") => {
                let source = query_text(url, "source").unwrap_or("system").to_string();
                let cleared = clear_audio_source(&mixer, &source);
                let _ = request.respond(json_response(
                    serde_json::json!({"source": source, "cleared_frames": cleared}).to_string(),
                    StatusCode(200),
                ));
            }
            (&Method::Get, url) if url.starts_with("/v1/audio/take") => {
                let source = query_text(url, "source").unwrap_or("system");
                let minimum_seconds = query_number(url, "min_seconds")
                    .unwrap_or(3.0)
                    .clamp(0.25, 30.0);
                let latest_seconds =
                    query_number(url, "latest_seconds").map(|seconds| seconds.clamp(0.25, 30.0));
                let consumer = query_text(url, "consumer").unwrap_or("desktop");
                match take_audio_source(&mixer, source, minimum_seconds, latest_seconds, consumer) {
                    Ok(Some(wav)) => {
                        let header = Header::from_bytes("Content-Type", "audio/wav").unwrap();
                        let response = Response::from_data(wav)
                            .with_status_code(StatusCode(200))
                            .with_header(header);
                        let _ = request.respond(response);
                    }
                    Ok(None) => {
                        let _ = request.respond(Response::empty(StatusCode(204)));
                    }
                    Err(error) => {
                        let _ = request.respond(text_response(error, StatusCode(409)));
                    }
                }
            }
            (&Method::Options, _) => {
                let _ = request.respond(cors(Response::empty(StatusCode(204))));
            }
            _ => {
                let _ = request.respond(text_response("not found".into(), StatusCode(404)));
            }
        }
    }
    Ok(())
}

fn query_number(url: &str, key: &str) -> Option<f32> {
    url.split_once('?')
        .map(|(_, query)| query)
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(name, value)| (name == key).then(|| value.parse().ok()).flatten())
}

fn query_text<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    url.split_once('?')
        .map(|(_, query)| query)
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(name, value)| (name == key).then_some(value))
}

fn take_audio_source(
    mixer: &Mixer,
    source: &str,
    minimum_seconds: f32,
    latest_seconds: Option<f32>,
    consumer: &str,
) -> Result<Option<Vec<u8>>, String> {
    audio_tap(mixer, source)?.take(consumer, minimum_seconds, latest_seconds)
}

fn audio_tap<'a>(mixer: &'a Mixer, source: &str) -> Result<&'a AudioTap, String> {
    match source.to_ascii_lowercase().as_str() {
        "microphone" | "mic" => Ok(&mixer.microphone_audio),
        "system" | "system-audio" | "playback" => mixer
            .system_audio
            .as_ref()
            .ok_or_else(|| "system-audio capture is disabled in mic-forwarder.toml".to_string()),
        _ => Err(format!(
            "unknown audio source '{source}'; expected microphone or system"
        )),
    }
}

fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    let mut wav = Vec::new();
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(Cursor::new(&mut wav), spec)
            .map_err(|error| format!("WAV writer failed: {error}"))?;
        for sample in samples {
            writer
                .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .map_err(|error| format!("WAV encode failed: {error}"))?;
        }
        writer
            .finalize()
            .map_err(|error| format!("WAV finalize failed: {error}"))?;
    }
    Ok(wav)
}

struct DecodedAudio {
    interleaved: Vec<f32>,
    channels: usize,
    sample_rate: u32,
}

struct EnqueueReport {
    frames: usize,
    seconds: f32,
    source_rate: u32,
    source_channels: usize,
}

fn decode_audio(audio: &[u8]) -> Result<DecodedAudio, String> {
    if audio.is_empty() {
        return Err("audio body is empty".into());
    }
    let mut hint = Hint::new();
    if audio.starts_with(b"RIFF") {
        hint.with_extension("wav");
    } else if audio.starts_with(b"ID3")
        || audio
            .get(..2)
            .is_some_and(|head| head[0] == 0xff && head[1] & 0xe0 == 0xe0)
    {
        hint.with_extension("mp3");
    } else if audio.get(4..8) == Some(b"ftyp") {
        hint.with_extension("m4a");
    }

    let stream = MediaSourceStream::new(
        Box::new(Cursor::new(audio.to_vec())),
        MediaSourceStreamOptions::default(),
    );
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("unsupported or invalid WAV/MP3/M4A: {error}"))?
        .format;
    let (track_id, codec_params) = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .map(|track| (track.id, track.codec_params.clone()))
        .ok_or_else(|| "audio contains no supported track".to_string())?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|error| format!("audio codec is unsupported: {error}"))?;
    let mut interleaved = Vec::new();
    let mut channels = 0;
    let mut sample_rate = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder = symphonia::default::get_codecs()
                    .make(&codec_params, &DecoderOptions::default())
                    .map_err(|error| format!("audio decoder reset failed: {error}"))?;
                continue;
            }
            Err(error) => return Err(format!("audio packet read failed: {error}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(error) => return Err(format!("audio decode failed: {error}")),
        };
        let spec = *decoded.spec();
        channels = spec.channels.count();
        sample_rate = spec.rate;
        let mut converted = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        converted.copy_interleaved_ref(decoded);
        interleaved.extend_from_slice(converted.samples());
    }
    if interleaved.is_empty() || channels == 0 || sample_rate == 0 {
        return Err("audio decoded to no playable samples".into());
    }
    Ok(DecodedAudio {
        interleaved,
        channels,
        sample_rate,
    })
}

fn enqueue_audio(audio: &[u8], mixer: &Mixer) -> Result<EnqueueReport, String> {
    let decoded = decode_audio(audio)?;
    let mono = decoded
        .interleaved
        .chunks(decoded.channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect::<Vec<_>>();
    let ratio = decoded.sample_rate as f64 / mixer.output_rate as f64;
    let output_len = ((mono.len() as f64) / ratio).ceil() as usize;
    if output_len
        > mixer
            .injected
            .capacity()
            .saturating_sub(mixer.injected.len())
    {
        return Err("injected-audio queue is full; wait for current speech to finish".into());
    }
    for index in 0..output_len {
        let source = index as f64 * ratio;
        let left = source.floor() as usize;
        let fraction = (source - left as f64) as f32;
        let a = mono.get(left).copied().unwrap_or(0.0);
        let b = mono.get(left + 1).copied().unwrap_or(a);
        mixer
            .injected
            .push(a + (b - a) * fraction)
            .map_err(|_| "injected-audio queue filled unexpectedly".to_string())?;
    }
    Ok(EnqueueReport {
        frames: output_len,
        seconds: output_len as f32 / mixer.output_rate as f32,
        source_rate: decoded.sample_rate,
        source_channels: decoded.channels,
    })
}

fn play_decoded_audio(audio: DecodedAudio) -> Result<(), String> {
    let (_stream, handle) = rodio::OutputStream::try_default()
        .map_err(|error| format!("could not open the Windows default playback device: {error}"))?;
    let sink = rodio::Sink::try_new(&handle)
        .map_err(|error| format!("could not create playback sink: {error}"))?;
    sink.append(rodio::buffer::SamplesBuffer::new(
        audio.channels as u16,
        audio.sample_rate,
        audio.interleaved,
    ));
    sink.sleep_until_end();
    Ok(())
}

fn clear_audio_source(mixer: &Mixer, source: &str) -> usize {
    audio_tap(mixer, source).map(AudioTap::clear).unwrap_or(0)
}

fn json_response(body: String, status: StatusCode) -> Response<Cursor<Vec<u8>>> {
    let header = Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap();
    cors(
        Response::from_data(body.into_bytes())
            .with_status_code(status)
            .with_header(header),
    )
}

fn text_response(body: String, status: StatusCode) -> Response<Cursor<Vec<u8>>> {
    cors(Response::from_data(body.into_bytes()).with_status_code(status))
}

fn cors<R: Read + Send + 'static>(response: Response<R>) -> Response<R> {
    response
        .with_header(Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap())
        .with_header(
            Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, OPTIONS").unwrap(),
        )
        .with_header(
            Header::from_bytes(
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization",
            )
            .unwrap(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resample(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
        let mut output = Vec::new();
        let mut resampler = LinearResampler::new(input_rate, output_rate);
        for sample in input {
            resampler.push(*sample, |value| output.push(value));
        }
        output
    }

    #[test]
    fn equal_rates_preserve_every_sample() {
        assert_eq!(
            resample(&[0.0, 0.5, -0.5, 1.0], 48_000, 48_000),
            [0.0, 0.5, -0.5, 1.0]
        );
    }

    #[test]
    fn upsampling_interpolates_instead_of_repeating() {
        assert_eq!(resample(&[0.0, 1.0, 0.0], 2, 4), [0.0, 0.5, 1.0, 0.5, 0.0]);
    }

    #[test]
    fn downsampling_keeps_the_correct_time_positions() {
        assert_eq!(resample(&[0.0, 1.0, 2.0, 3.0, 4.0], 4, 2), [0.0, 2.0, 4.0]);
    }

    #[test]
    fn vb_cable_endpoint_markers_tolerate_windows_name_wrappers() {
        assert!(is_vb_cable_render("CABLE Input (VB-Audio Virtual Cable)"));
        assert!(is_vb_cable_render("CABLE In 16ch (VB-Audio Virtual Cable)"));
        assert!(is_vb_cable_capture("CABLE Output (VB-Audio Virtual Cable)"));
        assert!(is_vb_cable_capture(
            "CABLE Out 16ch (VB-Audio Virtual Cable)"
        ));
        assert!(!is_vb_cable_capture("Headset (Nicholas's AirPods)"));
    }
}
