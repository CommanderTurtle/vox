//! Final synthesized-audio delivery.
//!
//! Engines only create audio. This module owns the independent destination:
//! speakers, a pasteable WAV on the Windows clipboard, or the local native
//! microphone router.

use std::io::Cursor;

use rodio::Source;

use crate::config::TtsOutputConfig;
use crate::tts::playback::{self, AudioFormat};
use crate::tts::TtsError;

pub async fn deliver(
    audio: Vec<u8>,
    format: AudioFormat,
    output: TtsOutputConfig,
) -> Result<(), TtsError> {
    match output.mode.as_str() {
        "clipboard_wav" => {
            let wav = ensure_wav(&audio, format)?;
            tokio::task::spawn_blocking(move || copy_wav_file_to_clipboard(&wav))
                .await
                .map_err(|error| TtsError::Playback(format!("clipboard task failed: {error}")))??;
            Ok(())
        }
        "mic_forwarder" => {
            let wav = ensure_wav(&audio, format)?;
            let url = format!(
                "{}/v1/forward",
                output.mic_forwarder_url.trim_end_matches('/')
            );
            let response = reqwest::Client::new()
                .post(url)
                .header(reqwest::header::CONTENT_TYPE, "audio/wav")
                .body(wav)
                .send()
                .await
                .map_err(|error| {
                    TtsError::Playback(format!("microphone forwarder is unavailable: {error}"))
                })?;
            if !response.status().is_success() {
                let status = response.status();
                let detail = response.text().await.unwrap_or_default();
                return Err(TtsError::Playback(format!(
                    "microphone forwarder returned {status}: {}",
                    detail.trim()
                )));
            }
            Ok(())
        }
        _ => {
            playback::play_bytes_async(audio, format);
            Ok(())
        }
    }
}

/// Normalize any supported TTS container into a PCM WAV for clipboard and
/// router destinations. LongCat already returns WAV, so its ordinary path is
/// a zero-decode clone.
pub fn ensure_wav(audio: &[u8], format: AudioFormat) -> Result<Vec<u8>, TtsError> {
    if format == AudioFormat::Wav {
        return Ok(audio.to_vec());
    }

    let decoder = rodio::Decoder::new(Cursor::new(audio.to_vec())).map_err(|error| {
        TtsError::Playback(format!("could not decode synthesized audio: {error}"))
    })?;
    let spec = hound::WavSpec {
        channels: decoder.channels(),
        sample_rate: decoder.sample_rate(),
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = Vec::new();
    {
        let mut writer = hound::WavWriter::new(Cursor::new(&mut wav), spec)
            .map_err(|error| TtsError::Playback(format!("WAV writer failed: {error}")))?;
        for sample in decoder.convert_samples::<f32>() {
            let sample = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer
                .write_sample(sample)
                .map_err(|error| TtsError::Playback(format!("WAV encode failed: {error}")))?;
        }
        writer
            .finalize()
            .map_err(|error| TtsError::Playback(format!("WAV finalize failed: {error}")))?;
    }
    Ok(wav)
}

#[cfg(windows)]
fn copy_wav_file_to_clipboard(wav: &[u8]) -> Result<(), TtsError> {
    use clipboard_win::{formats, Clipboard, Setter};

    let path = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("vox-output.wav")))
        .unwrap_or_else(|| std::path::PathBuf::from("vox-output.wav"));
    std::fs::write(&path, wav).map_err(|error| {
        TtsError::Playback(format!("could not write {}: {error}", path.display()))
    })?;
    let absolute = path.canonicalize().unwrap_or(path);
    let file = absolute.to_string_lossy().into_owned();
    let _clipboard = Clipboard::new_attempts(10)
        .map_err(|error| TtsError::Playback(format!("could not open clipboard: {error}")))?;
    formats::FileList
        .write_clipboard(&[file])
        .map_err(|error| TtsError::Playback(format!("could not publish WAV file: {error}")))?;
    log::info!("TTS WAV copied as a pasteable file: {}", absolute.display());
    Ok(())
}

#[cfg(not(windows))]
fn copy_wav_file_to_clipboard(_wav: &[u8]) -> Result<(), TtsError> {
    Err(TtsError::Playback(
        "clipboard WAV output is currently available on Windows".into(),
    ))
}
