//! Native microphone router for Vox.
//!
//! Captures one or more Windows input devices, mixes them with WAV audio
//! posted by Vox, and renders the combined signal to a selected output
//! endpoint. Select the playback side of a virtual audio cable as the output;
//! applications then use that cable's capture side as their microphone.

use std::io::{BufReader, Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use crossbeam::queue::ArrayQueue;
use rodio::Source;
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Response, Server, StatusCode};

const VB_CABLE_RENDER_MARKER: &str = "cable input";
const VB_CABLE_CAPTURE_MARKER: &str = "cable output";
const VB_CABLE_VENDOR_MARKER: &str = "vb-audio";

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
    system_audio: Option<Arc<ArrayQueue<f32>>>,
    /// Independent consumer lane for vox-http. Desktop captions and remote
    /// API clients can therefore run simultaneously without stealing chunks.
    system_audio_http: Option<Arc<ArrayQueue<f32>>>,
    system_audio_rate: u32,
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
    let system_audio = config.system_audio_enabled.then(|| {
        Arc::new(ArrayQueue::new(
            system_audio_rate as usize * config.queue_seconds.clamp(5, 3600),
        ))
    });
    let system_audio_http = system_audio.as_ref().map(|_| {
        Arc::new(ArrayQueue::new(
            system_audio_rate as usize * config.queue_seconds.clamp(5, 3600),
        ))
    });
    let mixer = Mixer {
        inputs: routes,
        injected: Arc::new(ArrayQueue::new(capacity)),
        injected_gain: config.injected_gain,
        output_rate,
        output_channels,
        system_audio,
        system_audio_http,
        system_audio_rate,
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
            route,
            output_rate,
            false,
            None,
        )?);
    }
    if let Some(samples) = &mixer.system_audio {
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
        let route = Route {
            name: format!("System audio ({name})"),
            gain: 1.0,
            samples: samples.clone(),
        };
        streams.push(build_input_stream(
            &device,
            route,
            mixer.system_audio_rate,
            true,
            mixer.system_audio_http.clone(),
        )?);
        log::info!(
            "System-audio subtitle tap: '{}' at {} Hz",
            name,
            mixer.system_audio_rate
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
            .with_inner_size([720.0, 350.0])
            .with_min_inner_size([520.0, 280.0]),
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
                    Ok(frames) => format!("Queued {frames} audio frames."),
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
        .find(|name| is_vb_cable_render(name))
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
        let device = choose_one(
            "system-audio playback",
            &system_outputs,
            physical_default,
            system_outputs.first().map(String::as_str),
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
    name.contains(VB_CABLE_RENDER_MARKER) && name.contains(VB_CABLE_VENDOR_MARKER)
}

fn is_vb_cable_capture(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains(VB_CABLE_CAPTURE_MARKER) && name.contains(VB_CABLE_VENDOR_MARKER)
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
        .find(|device| device.name().is_ok_and(|name| is_vb_cable_render(&name)))
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
    route: Route,
    output_rate: u32,
    loopback: bool,
    mirror: Option<Arc<ArrayQueue<f32>>>,
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
    let error_name = route.name.clone();
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
                        &route.samples,
                        mirror.as_deref(),
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
                        &route.samples,
                        mirror.as_deref(),
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
                        &route.samples,
                        mirror.as_deref(),
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
    queue: &ArrayQueue<f32>,
    mirror: Option<&ArrayQueue<f32>>,
    resampler: &mut LinearResampler,
    convert: impl Fn(T) -> f32,
) {
    debug_assert_eq!(
        resampler.step,
        input_rate.max(1) as f64 / output_rate.max(1) as f64
    );
    for frame in data.chunks_exact(channels.max(1)) {
        let mono = frame.iter().copied().map(&convert).sum::<f32>() / frame.len() as f32;
        resampler.push(mono, |sample| {
            push_latest(queue, sample);
            if let Some(mirror) = mirror {
                push_latest(mirror, sample);
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
    for frame in data.chunks_exact_mut(channels.max(1)) {
        let mut sample = mixer.injected.pop().unwrap_or(0.0) * mixer.injected_gain;
        for route in &mixer.inputs {
            sample += route.samples.pop().unwrap_or(0.0) * route.gain;
        }
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
                    "system_audio_enabled": mixer.system_audio.is_some(),
                    "system_audio_frames_buffered": mixer.system_audio.as_ref().map(|queue| queue.len()).unwrap_or(0),
                    "system_audio_http_frames_buffered": mixer.system_audio_http.as_ref().map(|queue| queue.len()).unwrap_or(0),
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
                        Ok(frames) => {
                            let body = serde_json::json!({"queued_frames": frames}).to_string();
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
                        Ok(_) => {
                            let playback_mixer = mixer.clone();
                            std::thread::spawn(move || {
                                if let Err(error) = play_audio(&audio) {
                                    log::error!("Router playback failed: {error}");
                                }
                                // Playback is captured by the WASAPI loopback
                                // tap. Drop it after playback so subtitle/dub
                                // consumers never process their own output.
                                clear_system_audio(&playback_mixer);
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
                let cleared = clear_system_audio(&mixer);
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
                match take_system_audio(&mixer, minimum_seconds, latest_seconds, consumer) {
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

fn take_system_audio(
    mixer: &Mixer,
    minimum_seconds: f32,
    latest_seconds: Option<f32>,
    consumer: &str,
) -> Result<Option<Vec<u8>>, String> {
    let queue = if consumer.eq_ignore_ascii_case("http") {
        &mixer.system_audio_http
    } else {
        &mixer.system_audio
    }
    .as_ref()
    .ok_or_else(|| "system-audio capture is disabled in mic-forwarder.toml".to_string())?;
    let minimum = (mixer.system_audio_rate as f32 * minimum_seconds) as usize;
    if queue.len() < minimum {
        return Ok(None);
    }
    if let Some(seconds) = latest_seconds {
        let keep = (mixer.system_audio_rate as f32 * seconds) as usize;
        while queue.len() > keep {
            let _ = queue.pop();
        }
    }
    let mut wav = Vec::new();
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: mixer.system_audio_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(Cursor::new(&mut wav), spec)
            .map_err(|error| format!("WAV writer failed: {error}"))?;
        while let Some(sample) = queue.pop() {
            writer
                .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .map_err(|error| format!("WAV encode failed: {error}"))?;
        }
        writer
            .finalize()
            .map_err(|error| format!("WAV finalize failed: {error}"))?;
    }
    Ok(Some(wav))
}

fn decode_audio(audio: &[u8]) -> Result<(Vec<f32>, usize, u32), String> {
    let decoder = rodio::Decoder::new(BufReader::new(Cursor::new(audio.to_vec())))
        .map_err(|error| format!("unsupported or invalid WAV/MP3/M4A: {error}"))?;
    let channels = decoder.channels().max(1) as usize;
    let sample_rate = decoder.sample_rate();
    let interleaved = decoder.convert_samples::<f32>().collect::<Vec<_>>();
    Ok((interleaved, channels, sample_rate))
}

fn enqueue_audio(audio: &[u8], mixer: &Mixer) -> Result<usize, String> {
    let (interleaved, channels, sample_rate) = decode_audio(audio)?;
    let mono = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect::<Vec<_>>();
    let ratio = sample_rate as f64 / mixer.output_rate as f64;
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
    Ok(output_len)
}

fn play_audio(audio: &[u8]) -> Result<(), String> {
    let (_stream, handle) = rodio::OutputStream::try_default()
        .map_err(|error| format!("could not open the Windows default playback device: {error}"))?;
    let sink = rodio::Sink::try_new(&handle)
        .map_err(|error| format!("could not create playback sink: {error}"))?;
    let decoder = rodio::Decoder::new(BufReader::new(Cursor::new(audio.to_vec())))
        .map_err(|error| format!("audio decode failed: {error}"))?;
    sink.append(decoder.convert_samples::<f32>());
    sink.sleep_until_end();
    Ok(())
}

fn clear_system_audio(mixer: &Mixer) -> usize {
    [&mixer.system_audio, &mixer.system_audio_http]
        .into_iter()
        .filter_map(Option::as_ref)
        .map(|queue| {
            let count = queue.len();
            while queue.pop().is_some() {}
            count
        })
        .sum()
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
        assert!(is_vb_cable_capture("CABLE Output (VB-Audio Virtual Cable)"));
        assert!(!is_vb_cable_capture("Headset (Nicholas's AirPods)"));
    }
}
