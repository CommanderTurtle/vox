//! Local Whisper ASR engine via `whisper.cpp` (whisper-rs binding).
//!
//! This module is only compiled when the `whisper-local` feature is enabled.
//! When disabled, a stub engine is provided that returns an error.

use crate::asr::{AsrEngine, AsrError};
use async_trait::async_trait;

// ── Feature-gated implementation ─────────────────────────────────────

/// Whisper local ASR engine using whisper.cpp.
#[cfg(feature = "whisper-local")]
pub struct WhisperLocalEngine {
    /// The whisper context (holds the loaded model).
    ctx: whisper_rs::WhisperContext,
    /// Model parameters.
    params: whisper_rs::WhisperParams,
}

#[cfg(feature = "whisper-local")]
impl WhisperLocalEngine {
    /// Create a new WhisperLocalEngine by loading a model from disk.
    ///
    /// `model_path`: path to a GGML model file (e.g. `ggml-tiny.bin`).
    pub fn new(model_path: &str) -> Result<Self, AsrError> {
        let ctx =
            whisper_rs::WhisperContext::new(model_path).map_err(|e| AsrError::EngineError {
                engine: "whisper-local".into(),
                message: format!("Failed to load model '{}': {}", model_path, e),
            })?;

        let mut params = whisper_rs::WhisperParams::new(whisper_rs::SAMPLING_GREEDY);
        params.set_n_threads(4);
        params.set_language(Some("auto"));
        params.set_translate(false);

        Ok(Self { ctx, params })
    }
}

#[cfg(feature = "whisper-local")]
#[async_trait]
impl AsrEngine for WhisperLocalEngine {
    fn name(&self) -> &'static str {
        "whisper-local"
    }

    async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError> {
        // Decode WAV to get PCM samples (16kHz, mono, i16)
        let mut reader =
            hound::WavReader::new(audio_wav).map_err(|e| AsrError::AudioFormat(e.to_string()))?;

        let spec = reader.spec();
        if spec.channels != 1 {
            return Err(AsrError::AudioFormat(format!(
                "Expected mono audio, got {} channels",
                spec.channels
            )));
        }

        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 32768.0)
                .collect(),
            hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        };

        let samples = whisper_rs::convert_stereo_to_mono_audio(&samples)
            .map_err(|e| AsrError::AudioFormat(e.to_string()))?;

        // Run inference (this is CPU-bound, ideally should be in a blocking task)
        let ctx = &self.ctx;
        let params = self.params.clone();

        // whisper-rs requires the audio to be in f32 at 16kHz
        let result = tokio::task::spawn_blocking(move || {
            let mut state = ctx
                .create_state()
                .map_err(|e| format!("Failed to create state: {}", e))?;

            state
                .full(params, &samples[..])
                .map_err(|e| format!("Inference failed: {}", e))?;

            let n_segments = state.full_n_segments();
            let mut text = String::new();

            for i in 0..n_segments {
                let segment = state
                    .full_get_segment_text(i)
                    .map_err(|e| format!("Failed to get segment {}: {}", i, e))?;
                text.push_str(&segment);
                text.push(' ');
            }

            Ok::<_, String>(text.trim().to_string())
        })
        .await
        .map_err(|e| AsrError::EngineError {
            engine: "whisper-local".into(),
            message: format!("Task join failed: {}", e),
        })?;

        result.map_err(|e| AsrError::EngineError {
            engine: "whisper-local".into(),
            message: e,
        })
    }
}

// ── Stub (whisper-local feature disabled) ────────────────────────────

/// Stub engine: returns an error when whisper-local feature is not enabled.
#[cfg(not(feature = "whisper-local"))]
pub struct WhisperLocalEngine;

#[cfg(not(feature = "whisper-local"))]
impl WhisperLocalEngine {
    pub fn new(_model_path: &str) -> Result<Self, AsrError> {
        Err(AsrError::EngineError {
            engine: "whisper-local".into(),
            message: "whisper-local feature not enabled at compile time".into(),
        })
    }
}

#[cfg(not(feature = "whisper-local"))]
#[async_trait]
impl AsrEngine for WhisperLocalEngine {
    fn name(&self) -> &'static str {
        "whisper-local"
    }

    async fn transcribe(&self, _audio_wav: &[u8]) -> Result<String, AsrError> {
        Err(AsrError::EngineError {
            engine: "whisper-local".into(),
            message: "whisper-local feature not enabled at compile time".into(),
        })
    }
}
