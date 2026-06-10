//! Mimo ASR engine — multimodal chat completion API with input_audio.
//!
//! Instead of using /audio/transcriptions, the Mimo ASR model accepts
//! audio via the chat completions endpoint as `input_audio` content.
//!
//! Endpoint: POST {base_url}/chat/completions
//! Auth: Authorization: Bearer {api_key}

use async_trait::async_trait;

use crate::asr::{AsrEngine, AsrError};

/// Mimo ASR engine using multimodal chat completions.
pub struct MimoAsrEngine {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl MimoAsrEngine {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120)) // ASR can take a while
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
impl AsrEngine for MimoAsrEngine {
    fn name(&self) -> &'static str {
        "mimo"
    }

    async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError> {
        let url = format!("{}/chat/completions", self.base_url);

        // Encode WAV bytes as a data URL
        let b64 = base64_encode(audio_wav);
        let data_url = format!("data:audio/wav;base64,{}", b64);

        // Build the JSON payload
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_audio",
                            "input_audio": {
                                "data": data_url,
                                "format": "wav"
                            }
                        }
                    ]
                }
            ]
        });

        // Send request
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| AsrError::EngineError {
                engine: "mimo".into(),
                message: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| AsrError::EngineError {
            engine: "mimo".into(),
            message: format!("Failed to read response body: {}", e),
        })?;

        if !status.is_success() {
            return Err(AsrError::EngineError {
                engine: "mimo".into(),
                message: format!("API returned {}: {}",
                    status.as_u16(),
                    body.chars().take(300).collect::<String>()),
            });
        }

        // Parse JSON response
        #[derive(serde::Deserialize)]
        struct MimoResponse {
            choices: Vec<Choice>,
        }

        #[derive(serde::Deserialize)]
        struct Choice {
            message: Message,
        }

        #[derive(serde::Deserialize)]
        struct Message {
            content: Option<String>,
        }

        let resp: MimoResponse = serde_json::from_str(&body).map_err(|e| {
            AsrError::EngineError {
                engine: "mimo".into(),
                message: format!(
                    "Failed to parse response JSON: {} — body: {}",
                    e,
                    body.chars().take(200).collect::<String>()
                ),
            }
        })?;

        let text = resp.choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(text)
    }
}

/// Base64-encode a byte slice (standard base64, no line wrapping).
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    engine.encode(data)
}
