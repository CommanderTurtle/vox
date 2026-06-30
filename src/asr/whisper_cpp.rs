//! Local ASR via the **whisper.cpp HTTP server** (`examples/server`).
//!
//! Start the server locally, e.g.:
//!   `./server -m ggml-base.en.bin -p 4 --port 8080`
//!
//! Endpoint: `POST {base_url}/inference`
//! Body: multipart form with a `file` field (the WAV) and a
//! `response_format` query parameter. We request `json` and parse the
//! `text` field; if parsing fails we fall back to the raw body.
//!
//! This avoids any FFI / libclang build dependency — it just talks HTTP
//! to a local process, so it works on any machine that can run the
//! whisper.cpp server binary.

use async_trait::async_trait;
use reqwest::multipart;

use crate::asr::{AsrEngine, AsrError};

/// whisper.cpp HTTP-server ASR engine.
pub struct WhisperCppEngine {
    base_url: String,
    client: reqwest::Client,
}

impl WhisperCppEngine {
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        }
    }
}

#[async_trait]
impl AsrEngine for WhisperCppEngine {
    fn name(&self) -> &'static str {
        "whisper-cpp"
    }

    async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError> {
        let url = format!(
            "{}/inference?response_format=json&temperature=0.0",
            self.base_url
        );

        let file_part = multipart::Part::bytes(audio_wav.to_vec())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| AsrError::EngineError {
                engine: "whisper-cpp".into(),
                message: format!("Failed to create multipart: {}", e),
            })?;

        let form = multipart::Form::new().part("file", file_part);

        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| AsrError::EngineError {
                engine: "whisper-cpp".into(),
                message: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| AsrError::EngineError {
            engine: "whisper-cpp".into(),
            message: format!("Failed to read response body: {}", e),
        })?;

        if !status.is_success() {
            return Err(AsrError::EngineError {
                engine: "whisper-cpp".into(),
                message: format!(
                    "API returned {}: {}",
                    status.as_u16(),
                    body.chars().take(300).collect::<String>()
                ),
            });
        }

        // Prefer the JSON `{"text": "..."}` shape; fall back to raw text.
        #[derive(serde::Deserialize)]
        struct WhisperCppResponse {
            text: Option<String>,
        }

        if let Ok(resp) = serde_json::from_str::<WhisperCppResponse>(&body) {
            if let Some(t) = resp.text {
                return Ok(t.trim().to_string());
            }
        }
        Ok(body.trim().to_string())
    }
}
