//! Audio playback — decodes and plays WAV/MP3 in-process via `rodio`.
//!
//! No external player binary or shell command required, which keeps playback
//! reliable across platforms (the previous PowerShell/afplay/aplay approach
//! had platform-specific MP3 and message-pump issues).

use std::io::Cursor;
use rodio::{OutputStream, Sink, Source};

use crate::tts::TtsError;

/// Audio container format hint for choosing the decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioFormat {
    /// Default — most cloud TTS engines return WAV.
    #[default]
    Wav,
    Mp3,
}

impl AudioFormat {
    pub fn suffix(self) -> &'static str {
        match self {
            AudioFormat::Wav => ".wav",
            AudioFormat::Mp3 => ".mp3",
        }
    }
}

/// Play audio bytes with the given format hint. Blocks until playback finishes.
pub fn play_bytes(data: &[u8], fmt: AudioFormat) -> Result<(), TtsError> {
    // OutputStream must live for the duration of playback; keep it alive
    // until the sink drains.
    let (_stream, stream_handle) = OutputStream::try_default().map_err(|e| {
        TtsError::Playback(format!("Failed to open audio output: {}", e))
    })?;

    let cursor = Cursor::new(data.to_vec());
    // rodio's Decoder auto-detects WAV/MP3/etc. from the bytes.
    let decoder = rodio::Decoder::new(cursor).map_err(|e| {
        TtsError::Playback(format!("Audio decode failed ({:?}): {}", fmt, e))
    })?;

    let sink = Sink::try_new(&stream_handle).map_err(|e| {
        TtsError::Playback(format!("Failed to create sink: {}", e))
    })?;
    sink.append(decoder.convert_samples::<f32>());
    sink.sleep_until_end();

    Ok(())
}

/// Play audio (with format hint) on a background thread. Returns immediately.
pub fn play_bytes_async(data: Vec<u8>, fmt: AudioFormat) {
    std::thread::spawn(move || {
        if let Err(e) = play_bytes(&data, fmt) {
            log::error!("Audio playback failed: {}", e);
        }
    });
}
