//! Native microphone router for Vox.
//!
//! Captures one or more Windows input devices, mixes them with WAV audio
//! posted by Vox, and renders the combined signal to a selected output
//! endpoint. Select the playback side of a virtual audio cable as the output;
//! applications then use that cable's capture side as their microphone.

use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use crossbeam::queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Response, Server, StatusCode};

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
    system_audio_rate: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let host = cpal::default_host();
    if std::env::args().any(|argument| argument == "--list-devices") {
        print_devices(&host)?;
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--init-config") {
        let (path, _) = load_config()?;
        println!("{}", path.display());
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
    let mixer = Mixer {
        inputs: routes,
        injected: Arc::new(ArrayQueue::new(capacity)),
        injected_gain: config.injected_gain,
        output_rate,
        output_channels,
        system_audio,
        system_audio_rate,
    };

    let output_name = output.name().unwrap_or_else(|_| "unknown output".into());
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
        streams.push(build_input_stream(&device, route, output_rate, false)?);
    }
    if let Some(samples) = &mixer.system_audio {
        let device = select_output(&host, &config.system_audio_device)?;
        let name = device
            .name()
            .unwrap_or_else(|_| config.system_audio_device.clone());
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
    loop {
        std::thread::park();
    }
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

fn print_devices(host: &cpal::Host) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Input devices:");
    for device in host.input_devices()? {
        println!("  {}", device.name().unwrap_or_else(|_| "<unknown>".into()));
    }
    println!("Output devices:");
    for device in host.output_devices()? {
        println!("  {}", device.name().unwrap_or_else(|_| "<unknown>".into()));
    }
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
            let mut phase = 0.0;
            device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    route_input(
                        data,
                        channels,
                        input_rate,
                        output_rate,
                        &route.samples,
                        &mut phase,
                        |sample| sample,
                    )
                },
                error_callback,
                None,
            )?
        }
        SampleFormat::I16 => {
            let mut phase = 0.0;
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    route_input(
                        data,
                        channels,
                        input_rate,
                        output_rate,
                        &route.samples,
                        &mut phase,
                        |sample| sample as f32 / 32768.0,
                    )
                },
                error_callback,
                None,
            )?
        }
        SampleFormat::U16 => {
            let mut phase = 0.0;
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    route_input(
                        data,
                        channels,
                        input_rate,
                        output_rate,
                        &route.samples,
                        &mut phase,
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
    phase: &mut f64,
    convert: impl Fn(T) -> f32,
) {
    let step = output_rate as f64 / input_rate as f64;
    for frame in data.chunks_exact(channels.max(1)) {
        let mono = frame.iter().copied().map(&convert).sum::<f32>() / frame.len() as f32;
        *phase += step;
        while *phase >= 1.0 {
            push_latest(queue, mono);
            *phase -= 1.0;
        }
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
                let mut wav = Vec::new();
                match request
                    .as_reader()
                    .take(512 * 1024 * 1024)
                    .read_to_end(&mut wav)
                {
                    Ok(_) => match enqueue_wav(&wav, &mixer) {
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
            (&Method::Get, url) if url.starts_with("/v1/system-audio/take") => {
                let minimum_seconds = query_number(url, "min_seconds")
                    .unwrap_or(3.0)
                    .clamp(0.25, 30.0);
                match take_system_audio(&mixer, minimum_seconds) {
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

fn take_system_audio(mixer: &Mixer, minimum_seconds: f32) -> Result<Option<Vec<u8>>, String> {
    let queue = mixer
        .system_audio
        .as_ref()
        .ok_or_else(|| "system-audio capture is disabled in mic-forwarder.toml".to_string())?;
    let minimum = (mixer.system_audio_rate as f32 * minimum_seconds) as usize;
    if queue.len() < minimum {
        return Ok(None);
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

fn enqueue_wav(wav: &[u8], mixer: &Mixer) -> Result<usize, String> {
    let mut reader =
        hound::WavReader::new(Cursor::new(wav)).map_err(|error| format!("invalid WAV: {error}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let interleaved = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("WAV decode failed: {error}"))?,
        hound::SampleFormat::Int => {
            let scale = 2_f32.powi(spec.bits_per_sample.saturating_sub(1) as i32);
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|sample| sample as f32 / scale))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("WAV decode failed: {error}"))?
        }
    };
    let mono = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect::<Vec<_>>();
    let ratio = spec.sample_rate as f64 / mixer.output_rate as f64;
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

fn json_response(body: String, status: StatusCode) -> Response<Cursor<Vec<u8>>> {
    let header = Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap();
    Response::from_data(body.into_bytes())
        .with_status_code(status)
        .with_header(header)
}

fn text_response(body: String, status: StatusCode) -> Response<Cursor<Vec<u8>>> {
    Response::from_data(body.into_bytes()).with_status_code(status)
}
