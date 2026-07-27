//! Audio playback — decodes and plays WAV/MP3 in-process via `rodio`.
//!
//! No external player binary or shell command required, which keeps playback
//! reliable across platforms (the previous PowerShell/afplay/aplay approach
//! had platform-specific MP3 and message-pump issues).

use rodio::{OutputStream, Sink, Source};
use std::io::Cursor;

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
    let (_stream, stream_handle) = OutputStream::try_default()
        .map_err(|e| TtsError::Playback(format!("Failed to open audio output: {}", e)))?;

    let cursor = Cursor::new(data.to_vec());
    // rodio's Decoder auto-detects WAV/MP3/etc. from the bytes.
    let decoder = rodio::Decoder::new(cursor)
        .map_err(|e| TtsError::Playback(format!("Audio decode failed ({:?}): {}", fmt, e)))?;

    let sink = Sink::try_new(&stream_handle)
        .map_err(|e| TtsError::Playback(format!("Failed to create sink: {}", e)))?;
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

/// Wrap raw little-endian i16 mono PCM bytes in a WAV container.
///
/// Shared by TTS engines that receive raw PCM from cloud APIs (Mimo, Doubao).
/// `sample_rate` is the PCM's sample rate (e.g. 24000 for Doubao, 24000 for Mimo).
pub(crate) fn pcm_to_wav(pcm_data: &[u8], sample_rate: u32) -> Vec<u8> {
    use hound::{SampleFormat, WavSpec, WavWriter};
    use std::io::Cursor;

    // PCM data is i16 samples (2 bytes each), little-endian.
    let sample_count = pcm_data.len() / 2;
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut buf = Vec::new();
    {
        let mut writer = WavWriter::new(Cursor::new(&mut buf), spec).unwrap();
        for i in 0..sample_count {
            let offset = i * 2;
            if offset + 1 < pcm_data.len() {
                let sample = i16::from_le_bytes([pcm_data[offset], pcm_data[offset + 1]]);
                writer.write_sample(sample).unwrap();
            }
        }
        writer.finalize().unwrap();
    }
    buf
}
