//! CrisperWhisper 2.0 local HTTP adapter.
//!
//! Vox selects either the model's cleaned `intended` transcript or its
//! word-faithful `literal` transcript. The standalone CrisperWhisper project
//! remains untouched.

use async_trait::async_trait;
use reqwest::multipart;

use crate::asr::{AsrEngine, AsrError};
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
        let file = multipart::Part::bytes(audio_wav.to_vec())
            .file_name("vox-recording.wav")
            .mime_str("audio/wav")
            .map_err(|error| Self::error(format!("invalid WAV multipart: {error}")))?;
        // Crisper's public API calls its literal transcript "verbatim".
        // Vox keeps the friendlier tray label while mapping to the exact
        // operation/result key expected by the backend.
        let mode = if self.config.mode.eq_ignore_ascii_case("literal")
            || self.config.mode.eq_ignore_ascii_case("verbatim")
        {
            "verbatim"
        } else {
            "intended"
        };
        let form = multipart::Form::new()
            .part("file", file)
            .text("operation", mode)
            .text("language", self.config.language.clone())
            .text("word_timestamps", "true")
            .text("strategy", "continuation")
            .text("chunk_duration", self.config.chunk_duration.to_string())
            .text("stride", self.config.stride.to_string())
            .text("context_words", self.config.context_words.to_string())
            .text("max_new_tokens", self.config.max_new_tokens.to_string())
            .text("hotwords", self.config.hotwords.clone());

        let url = format!(
            "{}/api/transcribe",
            self.config.base_url.trim_end_matches('/')
        );
        let response = self
            .client
            .post(url)
            .multipart(form)
            .send()
            .await
            .map_err(|error| Self::error(format!("service request failed: {error}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| Self::error(format!("could not read service response: {error}")))?;
        if !status.is_success() {
            return Err(Self::error(format!(
                "service returned {}: {}",
                status.as_u16(),
                body.chars().take(400).collect::<String>()
            )));
        }

        let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
            Self::error(format!(
                "invalid CrisperWhisper response: {error}; body: {}",
                body.chars().take(300).collect::<String>()
            ))
        })?;
        let text = parsed
            .get("results")
            .and_then(|results| results.get(mode))
            .and_then(|transcript| transcript.get("text"))
            .and_then(|text| text.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(Self::error(format!(
                "CrisperWhisper returned an empty {mode} transcript"
            )));
        }
        Ok(text)
    }
}
