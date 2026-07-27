//! Mimo TTS engine — multimodal chat completion with TTS model.
//!
//! Sends the text as an assistant message and receives synthesized
//! audio as base64-encoded PCM data in the response.

use async_trait::async_trait;

use crate::tts::{TtsEngine, TtsError};

/// Mimo TTS engine.
pub struct MimoTtsEngine {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl MimoTtsEngine {
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
impl TtsEngine for MimoTtsEngine {
    fn name(&self) -> &'static str {
        "mimo-tts"
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let url = format!("{}/chat/completions", self.base_url);

        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "user", "content": "请朗读以下文字"},
                {"role": "assistant", "content": text}
            ]
        });

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .map_err(|e| TtsError::EngineError {
                engine: "mimo-tts".into(),
                message: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| TtsError::EngineError {
            engine: "mimo-tts".into(),
            message: format!("Failed to read response body: {}", e),
        })?;

        if !status.is_success() {
            return Err(TtsError::EngineError {
                engine: "mimo-tts".into(),
                message: format!("API returned {}: {}", status.as_u16(),
                    body.chars().take(300).collect::<String>()),
            });
        }

        // Parse response — extract audio.data (base64 PCM)
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
            audio: Option<AudioData>,
        }
        #[derive(serde::Deserialize)]
        struct AudioData {
            data: String,
        }

        let resp: MimoResponse = serde_json::from_str(&body).map_err(|e| {
            TtsError::EngineError {
                engine: "mimo-tts".into(),
                message: format!("Failed to parse JSON: {} — body: {}",
                    e, body.chars().take(200).collect::<String>()),
            }
        })?;

        let b64_data = resp.choices
            .first()
            .and_then(|c| c.message.audio.as_ref())
            .map(|a| &a.data)
            .ok_or_else(|| TtsError::EngineError {
                engine: "mimo-tts".into(),
                message: "No audio data in response".into(),
            })?;

        // Decode base64 → raw PCM bytes
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD;
        let pcm_bytes = engine.decode(b64_data).map_err(|e| TtsError::EngineError {
            engine: "mimo-tts".into(),
            message: format!("Base64 decode failed: {}", e),
        })?;

        // The returned data is raw PCM (probably 16-bit 24kHz mono).
        // Wrap it in a WAV header so playback systems can use it.
        let wav_bytes = crate::tts::playback::pcm_to_wav(&pcm_bytes, 24000);
        Ok(wav_bytes)
    }
}
