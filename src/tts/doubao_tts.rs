//! Volcano Engine **Doubao TTS 2.0** (`seed-tts-2.0`) engine.
//!
//! Uses the HTTP unidirectional streaming endpoint. Auth is via the
//! `X-Api-Key` + `X-Api-Resource-Id` headers (NOT `Authorization: Bearer`).
//! Uses the Agent Plan endpoint (`/api/v3/plan/tts/...`) with the Agent Plan
//! subscription key.
//!
//! Protocol (cross-verified against the official `veadk-go` SDK and several
//! production clients):
//! 1. `POST` to `/api/v3/plan/tts/unidirectional` with the JSON payload.
//! 2. The response is a chunked stream of newline-delimited JSON (NDJSON).
//!    Each line is `{"code": <i64>, "data": "<base64 audio chunk>", "message": "..."}`.
//! 3. `code == 0` -> `data` is a base64-encoded PCM chunk; decode + append.
//!    `code == 20000000` -> stream termination marker; stop.
//!    Any other positive code -> error; surface `message`.
//! 4. We request `audio_params.format = "pcm"` (24kHz 16-bit mono little-endian)
//!    and wrap the accumulated PCM in a WAV container for playback, reusing
//!    the shared `pcm_to_wav` helper.

use async_trait::async_trait;
use futures_util::StreamExt;
use uuid::Uuid;

use crate::tts::playback::pcm_to_wav;
use crate::tts::{TtsEngine, TtsError};

/// HTTP unidirectional streaming endpoint (Agent Plan subscription path).
const TTS_ENDPOINT: &str = "https://openspeech.bytedance.com/api/v3/plan/tts/unidirectional";
/// Resource-Id selecting the seed-tts-2.0 model.
const RESOURCE_ID: &str = "seed-tts-2.0";
/// Stream-termination success code returned by the server.
const CODE_STREAM_END: i64 = 20_000_000;

/// Doubao TTS 2.0 engine.
pub struct DoubaoTtsEngine {
    api_key: String,
    speaker: String,
    speech_rate: i32,
    loudness_rate: i32,
    sample_rate: u32,
    client: reqwest::Client,
}

impl DoubaoTtsEngine {
    pub fn new(
        api_key: &str,
        speaker: &str,
        speech_rate: i32,
        loudness_rate: i32,
        sample_rate: u32,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            api_key: api_key.to_string(),
            speaker: if speaker.is_empty() {
                "zh_female_vv_uranus_bigtts".to_string()
            } else {
                speaker.to_string()
            },
            speech_rate,
            loudness_rate,
            // Clamp sample_rate to a sane non-zero default; 0 would divide-by-zero
            // in the WAV writer and is never a valid output rate.
            sample_rate: if sample_rate == 0 { 24000 } else { sample_rate },
            client,
        }
    }

    /// Build the JSON request body.
    fn build_payload(&self, text: &str) -> serde_json::Value {
        serde_json::json!({
            "user": { "uid": "vox-user" },
            "req_params": {
                "text": text,
                "speaker": self.speaker,
                "audio_params": {
                    "format": "pcm",
                    "sample_rate": self.sample_rate,
                    "speech_rate": self.speech_rate,
                    "loudness_rate": self.loudness_rate,
                }
            }
        })
    }
}

#[async_trait]
impl TtsEngine for DoubaoTtsEngine {
    fn name(&self) -> &'static str {
        "doubao-tts"
    }

    fn output_format(&self) -> crate::tts::playback::AudioFormat {
        crate::tts::playback::AudioFormat::Wav
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        if text.trim().is_empty() {
            return Err(TtsError::EngineError {
                engine: "doubao-tts".into(),
                message: "empty text".into(),
            });
        }

        let request_id = Uuid::new_v4().to_string();
        let response = self
            .client
            .post(TTS_ENDPOINT)
            .header("Content-Type", "application/json")
            .header("X-Api-Key", &self.api_key)
            .header("X-Api-Resource-Id", RESOURCE_ID)
            .header("X-Api-Request-Id", &request_id)
            .json(&self.build_payload(text))
            .send()
            .await
            .map_err(|e| TtsError::EngineError {
                engine: "doubao-tts".into(),
                message: format!("HTTP request failed: {}", e),
            })?;

        // Non-2xx: the body is a single JSON object (not NDJSON) describing
        // the error, e.g. {"header":{"code":...,"message":"Invalid X-Api-Key"}}.
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(TtsError::EngineError {
                engine: "doubao-tts".into(),
                message: format!(
                    "API returned {}: {}",
                    status.as_u16(),
                    body.chars().take(300).collect::<String>()
                ),
            });
        }

        // Stream the NDJSON body line by line. We accumulate raw bytes in a
        // buffer and split on '\n', because chunks don't always align with
        // network read boundaries.
        let mut stream = response.bytes_stream();
        let mut buf = Vec::with_capacity(8192);
        let mut pcm = Vec::new();
        let mut stream_ended = false;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| TtsError::EngineError {
                engine: "doubao-tts".into(),
                message: format!("stream read failed: {}", e),
            })?;
            buf.extend_from_slice(&chunk);

            // Process complete lines (terminated by '\n').
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let line_str = String::from_utf8_lossy(&line).trim().to_string();
                if line_str.is_empty() {
                    continue;
                }
                match Self::parse_frame(&line_str)? {
                    Frame::Audio(data) => pcm.extend_from_slice(&data),
                    Frame::End => {
                        stream_ended = true;
                        break;
                    }
                    Frame::Skip => {}
                }
            }
            if stream_ended {
                break;
            }
        }

        if !stream_ended && pcm.is_empty() {
            return Err(TtsError::EngineError {
                engine: "doubao-tts".into(),
                message: "stream ended before any audio was produced".into(),
            });
        }
        if pcm.is_empty() {
            return Err(TtsError::EngineError {
                engine: "doubao-tts".into(),
                message: "no audio data in response".into(),
            });
        }

        log::info!(
            "doubao-tts: synthesized {} bytes of PCM ({}Hz) from {} input chars",
            pcm.len(),
            self.sample_rate,
            text.len()
        );
        Ok(pcm_to_wav(&pcm, self.sample_rate))
    }
}

impl DoubaoTtsEngine {
    /// Parse one NDJSON line into a `Frame`.
    fn parse_frame(line: &str) -> Result<Frame, TtsError> {
        #[derive(serde::Deserialize)]
        struct ResponseFrame {
            code: i64,
            #[serde(default)]
            data: Option<String>,
            #[serde(default)]
            message: Option<String>,
        }

        let frame: ResponseFrame =
            serde_json::from_str(line).map_err(|e| TtsError::EngineError {
                engine: "doubao-tts".into(),
                message: format!(
                    "failed to parse NDJSON frame: {} - line: {}",
                    e,
                    line.chars().take(200).collect::<String>()
                ),
            })?;

        match frame.code {
            0 => match frame.data {
                Some(b64) if !b64.is_empty() => {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&b64)
                        .map_err(|e| TtsError::EngineError {
                            engine: "doubao-tts".into(),
                            message: format!("base64 decode failed: {}", e),
                        })?;
                    Ok(Frame::Audio(bytes))
                }
                _ => Ok(Frame::Skip),
            },
            CODE_STREAM_END => Ok(Frame::End),
            code => Err(TtsError::EngineError {
                engine: "doubao-tts".into(),
                message: format!(
                    "server error code {}: {}",
                    code,
                    frame.message.unwrap_or_default()
                ),
            }),
        }
    }
}

/// Parsed NDJSON frame.
#[derive(Debug)]
enum Frame {
    /// A decoded audio chunk to append.
    Audio(Vec<u8>),
    /// Stream-termination marker.
    End,
    /// A frame with no payload to act on (e.g. metadata-only).
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frame_audio() {
        let raw = b"hello pcm";
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        let line = format!(r#"{{"code":0,"data":"{}"}}"#, b64);
        match DoubaoTtsEngine::parse_frame(&line).unwrap() {
            Frame::Audio(bytes) => assert_eq!(bytes, raw),
            _ => panic!("expected Audio"),
        }
    }

    #[test]
    fn test_parse_frame_skip_empty_data() {
        // code=0 but data is empty -> Skip.
        let line = r#"{"code":0,"data":""}"#;
        assert!(matches!(
            DoubaoTtsEngine::parse_frame(line).unwrap(),
            Frame::Skip
        ));
        // code=0, data field absent -> Skip.
        let line = r#"{"code":0}"#;
        assert!(matches!(
            DoubaoTtsEngine::parse_frame(line).unwrap(),
            Frame::Skip
        ));
    }

    #[test]
    fn test_parse_frame_end() {
        let line = r#"{"code":20000000}"#;
        assert!(matches!(
            DoubaoTtsEngine::parse_frame(line).unwrap(),
            Frame::End
        ));
    }

    #[test]
    fn test_parse_frame_error_code() {
        let line = r#"{"code":45000010,"message":"Invalid X-Api-Key"}"#;
        let err = DoubaoTtsEngine::parse_frame(line).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("45000010"), "msg: {}", msg);
        assert!(msg.contains("Invalid X-Api-Key"), "msg: {}", msg);
    }

    #[test]
    fn test_parse_frame_invalid_json() {
        let err = DoubaoTtsEngine::parse_frame("not json").unwrap_err();
        assert!(err.to_string().contains("failed to parse NDJSON frame"));
    }
}
