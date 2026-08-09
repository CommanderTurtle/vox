//! CrisperWhisper 2.0 local HTTP adapter.
//!
//! Vox selects either the model's cleaned `intended` transcript or its
//! word-faithful `literal` transcript. The standalone CrisperWhisper project
//! remains untouched.

use async_trait::async_trait;
use vox_local_core::crisper::{transcribe_tagged, CrisperOptions};

use crate::asr::{AsrEngine, AsrError, AsrOutput};
use crate::config::CrisperConfig;

pub struct CrisperWhisperEngine {
    config: CrisperConfig,
    client: reqwest::Client,
}

impl CrisperWhisperEngine {
    pub fn new(config: &CrisperConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("Failed to build CrisperWhisper HTTP client");
        Self {
            config: config.clone(),
            client,
        }
    }

    fn error(message: impl Into<String>) -> AsrError {
        AsrError::EngineError {
            engine: "crisper-whisper".into(),
            message: message.into(),
        }
    }
}

#[async_trait]
impl AsrEngine for CrisperWhisperEngine {
    fn name(&self) -> &'static str {
        "crisper-whisper"
    }

    async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError> {
        self.transcribe_tagged(audio_wav)
            .await
            .map(|result| result.text)
    }

    async fn transcribe_tagged(&self, audio_wav: &[u8]) -> Result<AsrOutput, AsrError> {
        let options = CrisperOptions {
            base_url: self.config.base_url.clone(),
            mitm_url: self.config.mitm_url.clone(),
            mitm_api_key: self.config.mitm_api_key.clone(),
            mode: self.config.mode.clone(),
            language: self.config.language.clone(),
            candidate_max_new_tokens: self.config.candidate_max_new_tokens,
            chunk_duration: self.config.chunk_duration,
            stride: self.config.stride,
            context_words: self.config.context_words,
            max_new_tokens: self.config.max_new_tokens,
            hotwords: self.config.hotwords.clone(),
        };
        transcribe_tagged(
            &self.client,
            audio_wav,
            "vox-recording.wav",
            "audio/wav",
            &options,
        )
        .await
        .map(|result| AsrOutput {
            text: result.text,
            language: result.language,
        })
        .map_err(|error| Self::error(error.to_string()))
    }
}
