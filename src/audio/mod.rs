//! Audio capture: microphone PCM recording via `cpal`.
//!
//! Records from the default input device at 16 kHz, mono, i16 PCM format
//! (compatible with whisper and most ASR APIs).

pub mod capture;
pub mod utils;
