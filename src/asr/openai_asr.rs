//! OpenAI Whisper API ASR engine.
//!
//! Uses the standard OpenAI-compatible `/audio/transcriptions` endpoint
//! with multipart form data.
//!
//! Endpoint: POST {base_url}/audio/transcriptions
//! Auth: Authorization: Bearer {api_key}

use async_trait::async_trait;
use reqwest::multipart;

use crate::asr::{AsrEngine, AsrError};

/// OpenAI-compatible Whisper ASR engine.
pub struct OpenaiAsrEngine {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenaiAsrEngine {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            client,
        }
    }
}

#[async_trait]
impl AsrEngine for OpenaiAsrEngine {
    fn name(&self) -> &'static str {
        "openai"
    }

    async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError> {
        let url = format!("{}/audio/transcriptions", self.base_url);

        let file_part = multipart::Part::bytes(audio_wav.to_vec())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| AsrError::EngineError {
                engine: "openai".into(),
                message: format!("Failed to create multipart: {}", e),
            })?;

        let model_part = multipart::Part::text(self.model.clone());

        let form = multipart::Form::new()
            .part("file", file_part)
            .part("model", model_part);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| AsrError::EngineError {
                engine: "openai".into(),
                message: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| AsrError::EngineError {
            engine: "openai".into(),
            message: format!("Failed to read response body: {}", e),
        })?;

        if !status.is_success() {
            return Err(AsrError::EngineError {
                engine: "openai".into(),
                message: format!(
                    "API returned {}: {}",
                    status.as_u16(),
                    body.chars().take(300).collect::<String>()
                ),
            });
        }

        #[derive(serde::Deserialize)]
        struct TranscriptionResponse {
            text: String,
        }

        let resp: TranscriptionResponse =
            serde_json::from_str(&body).map_err(|e| AsrError::EngineError {
                engine: "openai".into(),
                message: format!(
                    "Failed to parse response JSON: {} — body: {}",
                    e,
                    body.chars().take(200).collect::<String>()
                ),
            })?;

        Ok(resp.text.trim().to_string())
    }
}
