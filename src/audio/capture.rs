//! Cross-platform microphone audio capture via `cpal`.
//!
//! Records from the default input device.
//! The capture runs on a dedicated thread and accumulates PCM i16 samples
//! until told to stop via an atomic flag.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Handle to an ongoing audio capture session.
pub struct AudioCapture {
    /// Shared stop signal — set to true to stop recording.
    stop_flag: Arc<AtomicBool>,
    /// Collected PCM i16 samples (16 kHz, mono).
    samples: Arc<std::sync::Mutex<Vec<i16>>>,
    /// The sample rate of the captured audio.
    #[allow(dead_code)]
    sample_rate: u32,
}

impl AudioCapture {
    /// Start recording from the default input device.
    ///
    /// Returns immediately; recording happens on a background thread.
    pub fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(std::sync::Mutex::new(Vec::new()));

        let stop_flag_clone = stop_flag.clone();
        let stop_flag_for_wait = stop_flag.clone(); // for the blocking loop below
        let samples_clone = samples.clone();

        std::thread::spawn(move || {
            let host = cpal::default_host();
            let device = match host.default_input_device() {
                Some(d) => d,
                None => {
                    log::error!("No default input device found.");
                    return;
                }
            };

            log::info!("Using input device: {}", device.name().unwrap_or_default());

            let config = match device.default_input_config() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to get default input config: {}", e);
                    return;
                }
            };

            log::info!("Input config: {:?}", config);

            let _sample_rate = config.sample_rate().0;
            let channels = config.channels() as usize;

            let err_fn = |err| {
                log::error!("Audio capture stream error: {}", err);
            };

            let stream_result = match config.sample_format() {
                cpal::SampleFormat::I16 => {
                    device.build_input_stream(
                        &config.config(),
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if stop_flag_clone.load(Ordering::Relaxed) {
                                return;
                            }
                            let mut buf = samples_clone.lock().unwrap();
                            if channels > 1 {
                                for (i, &sample) in data.iter().enumerate() {
                                    if i % channels == 0 {
                                        buf.push(sample);
                                    }
                                }
                            } else {
                                buf.extend_from_slice(data);
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::F32 => {
                    device.build_input_stream(
                        &config.config(),
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if stop_flag_clone.load(Ordering::Relaxed) {
                                return;
                            }
                            let mut buf = samples_clone.lock().unwrap();
                            if channels > 1 {
                                for (i, &sample) in data.iter().enumerate() {
                                    if i % channels == 0 {
                                        buf.push((sample * 32767.0) as i16);
                                    }
                                }
                            } else {
                                for &sample in data {
                                    buf.push((sample * 32767.0) as i16);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                fmt => {
                    log::error!("Unsupported sample format: {:?}", fmt);
                    return;
                }
            };

            let stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to build input stream: {}", e);
                    return;
                }
            };

            if let Err(e) = stream.play() {
                log::error!("Failed to play audio stream: {}", e);
                return;
            }

            // Block until stop flag is set
            while !stop_flag_for_wait.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            // Drop stream to stop capture
            drop(stream);

            log::info!("Audio capture stopped.");
        });

        Ok(AudioCapture {
            stop_flag,
            samples,
            sample_rate: 16000,
        })
    }

    /// Stop recording and return the accumulated PCM i16 samples.
    pub fn stop(&self) -> Vec<i16> {
        self.stop_flag.store(true, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let guard = self.samples.lock().unwrap();
        guard.clone()
    }

    /// Get the sample rate of the captured audio.
    #[allow(dead_code)]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
