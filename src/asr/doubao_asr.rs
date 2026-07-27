//! Volcano Engine **Doubao streaming ASR 2.0** (`volc.seedasr.sauc.duration`).
//!
//! Uses the WebSocket `bigmodel_nostream` endpoint, which is designed for the
//! "send all audio, then receive one final result" flow - a perfect fit for
//! vox's record-then-transcribe model (no partial-result handling needed).
//!
//! Auth is via `X-Api-Key` + `X-Api-Resource-Id` handshake headers (NOT
//! `Authorization: Bearer`); the key is shared with the Doubao TTS engine and
//! must come from the Doubao Speech console.
//!
//! ## Binary-frame protocol
//!
//! Every WebSocket message is a binary frame with this layout:
//!
//! ```text
//! [ 4-byte header ][ optional 4-byte sequence ][ 4-byte payload_size (BE u32) ][ payload ]
//! ```
//!
//! Header bytes:
//! - `byte0 = (version << 4) | header_size_code`  -> `0x11` (v1, code 1)
//! - `byte1 = (message_type << 4) | flags`
//! - `byte2 = (serialization << 4) | compression`
//! - `byte3 = 0x00` (reserved)
//!
//! Message types: `0x1` FULL_CLIENT_REQUEST (config), `0x2` AUDIO_ONLY,
//! `0x9` SERVER_RESPONSE, `0xF` SERVER_ERROR.
//! Flags: `0x0` no-sequence, `0x1` positive sequence, `0x2` last-no-seq,
//! `0x3` last-negative-seq.
//! Serialization: `0x0` none, `0x1` JSON. Compression: `0x0` none, `0x1` gzip.
//!
//! We gzip-compress the JSON config and audio chunks (the server accepts both,
//! but gzip matches the reference clients and reduces bandwidth).

use async_trait::async_trait;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::{SinkExt, StreamExt};
use std::io::Write;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::asr::{AsrEngine, AsrError};

/// Non-streaming endpoint: collect all audio + an empty last packet, then the
/// server returns one final result. Simpler than the bidirectional variants
/// and most accurate for the record-then-transcribe use case.
const ASR_ENDPOINT: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream";
const RESOURCE_ID: &str = "volc.seedasr.sauc.duration";

// Header byte 0: version=1 (high nibble), header_size_code=1 (low nibble).
const HDR_VERSION: u8 = 0x11;

// Message types (high nibble of byte 1).
const MSG_FULL_CLIENT_REQUEST: u8 = 0x1;
const MSG_AUDIO_ONLY: u8 = 0x2;
const MSG_SERVER_RESPONSE: u8 = 0x9;
const MSG_SERVER_ERROR: u8 = 0xF;

// Flags (low nibble of byte 1).
const FLAG_NO_SEQUENCE: u8 = 0x0;
const FLAG_LAST_NO_SEQUENCE: u8 = 0x2;

// Serialization (high nibble of byte 2).
const SER_NONE: u8 = 0x0;
const SER_JSON: u8 = 0x1;
// Compression (low nibble of byte 2).
#[allow(dead_code)]
const COMP_NONE: u8 = 0x0;
const COMP_GZIP: u8 = 0x1;

/// Audio chunk size: ~100ms of 16kHz mono 16-bit PCM = 3200 bytes. Small
/// enough to stream, large enough to avoid per-frame overhead.
const AUDIO_CHUNK_SIZE: usize = 3200;

/// Doubao streaming ASR 2.0 engine.
pub struct DoubaoAsrEngine {
    api_key: String,
}

impl DoubaoAsrEngine {
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl AsrEngine for DoubaoAsrEngine {
    fn name(&self) -> &'static str {
        "doubao-asr"
    }

    async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError> {
        if audio_wav.is_empty() {
            return Err(AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: "empty audio".into(),
            });
        }

        let request = build_handshake_request(&self.api_key)?;
        let (mut ws_stream, response) = match connect_async(request).await {
            Ok(pair) => pair,
            Err(e) => {
                // tungstenite carries the HTTP response on handshake failures
                // (e.g. 401). Extract its body so we can surface the real
                // reason instead of a bare "HTTP error: 401 Unauthorized".
                let detail = match &e {
                    tokio_tungstenite::tungstenite::Error::Http(resp) => {
                        let status = resp.status();
                        let body = resp
                            .body()
                            .as_ref()
                            .map(|b| String::from_utf8_lossy(b).to_string())
                            .unwrap_or_default();
                        format!("HTTP {} - {}", status, body.chars().take(300).collect::<String>())
                    }
                    other => other.to_string(),
                };
                return Err(AsrError::EngineError {
                    engine: "doubao-asr".into(),
                    message: format!(
                        "WebSocket handshake failed: {}. \
                         Hint: the key must be issued from the Doubao Speech console, \
                         not the Ark console (Ark keys return 401 here).",
                        detail
                    ),
                });
            }
        };
        log::debug!(
            "doubao-asr WS handshake status: {}, headers: {:?}",
            response.status(),
            response.headers()
        );

        // 1. Send the FULL_CLIENT_REQUEST (JSON config, gzip-compressed).
        let config_json = build_config_json();
        let full_request = build_frame(
            MSG_FULL_CLIENT_REQUEST,
            FLAG_NO_SEQUENCE,
            SER_JSON,
            COMP_GZIP,
            &config_json,
            true, // gzip the payload
        );
        ws_stream.send(Message::Binary(full_request)).await.map_err(|e| {
            AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: format!("send full request failed: {}", e),
            }
        })?;

        // 2. Stream the WAV bytes in AUDIO_ONLY chunks (gzip-compressed).
        let mut offset = 0;
        while offset < audio_wav.len() {
            let end = (offset + AUDIO_CHUNK_SIZE).min(audio_wav.len());
            let chunk = &audio_wav[offset..end];
            let frame = build_frame(
                MSG_AUDIO_ONLY,
                FLAG_NO_SEQUENCE,
                SER_NONE,
                COMP_GZIP,
                chunk,
                true,
            );
            ws_stream.send(Message::Binary(frame)).await.map_err(|e| {
                AsrError::EngineError {
                    engine: "doubao-asr".into(),
                    message: format!("send audio chunk failed: {}", e),
                }
            })?;
            offset = end;
        }

        // 3. Send the empty last packet to signal end-of-audio.
        let last_packet = build_frame(
            MSG_AUDIO_ONLY,
            FLAG_LAST_NO_SEQUENCE,
            SER_NONE,
            COMP_GZIP,
            &[],
            false, // empty payload, no need to gzip
        );
        ws_stream.send(Message::Binary(last_packet)).await.map_err(|e| {
            AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: format!("send last packet failed: {}", e),
            }
        })?;
        log::debug!("doubao-asr: sent {} bytes of audio in chunks", audio_wav.len());

        // 4. Read responses until we get the final result or the server closes.
        let mut full_text = String::new();
        while let Some(msg_result) = ws_stream.next().await {
            let msg = msg_result.map_err(|e| AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: format!("recv failed: {}", e),
            })?;
            match msg {
                Message::Binary(bin) => {
                    match parse_server_frame(&bin)? {
                        ServerFrame::Response { text, is_last } => {
                            if !text.is_empty() {
                                full_text = text; // nostream: final text replaces partials
                            }
                            if is_last {
                                break;
                            }
                        }
                        ServerFrame::Error { code, message } => {
                            // "last packet has been received already" is a benign
                            // race: the server auto-finalized via VAD before our
                            // explicit last packet. Treat accumulated text as success.
                            if message.contains("last packet")
                                || message.contains("already received")
                            {
                                log::warn!("doubao-asr benign race: {}", message);
                                break;
                            }
                            return Err(AsrError::EngineError {
                                engine: "doubao-asr".into(),
                                message: format!("server error {}: {}", code, message),
                            });
                        }
                    }
                }
                Message::Close(_) => {
                    log::debug!("doubao-asr: server closed connection");
                    break;
                }
                _ => {}
            }
        }

        let trimmed = full_text.trim().to_string();
        if trimmed.is_empty() {
            return Err(AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: "no transcription text in response".into(),
            });
        }
        log::info!("doubao-asr: transcribed {} chars", trimmed.len());
        Ok(trimmed)
    }
}

// ── Frame construction ───────────────────────────────────────────────

/// Build the JSON config for the FULL_CLIENT_REQUEST.
///
/// `format: "wav"` lets us send the WAV file bytes directly - the server
/// parses the WAV header itself, so no client-side header stripping is needed.
fn build_config_json() -> Vec<u8> {
    serde_json::json!({
        "user": { "uid": "vox-asr" },
        "audio": {
            "format": "wav",
            "rate": 16000,
            "bits": 16,
            "channel": 1
        },
        "request": {
            "model_name": "bigmodel",
            "enable_itn": true,    // inverse text normalization (numbers, dates)
            "enable_punc": true,   // auto punctuation
            "enable_ddc": true,    // disfluency / filler-word removal
            "show_utterances": true,
            "result_type": "full"
        }
    })
    .to_string()
    .into_bytes()
}

/// Build a client->server binary frame.
///
/// Layout: `[4B header][4B payload_size (BE u32)][payload]`.
/// `should_gzip` compresses the payload with gzip (we pass `false` only for
/// the empty last packet, where compression is pointless).
fn build_frame(
    msg_type: u8,
    flags: u8,
    serialization: u8,
    compression: u8,
    payload: &[u8],
    should_gzip: bool,
) -> Vec<u8> {
    let payload = if should_gzip && !payload.is_empty() {
        gzip_bytes(payload)
    } else {
        payload.to_vec()
    };

    let header_byte1 = (msg_type << 4) | flags;
    let header_byte2 = (serialization << 4) | compression;

    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.push(HDR_VERSION);
    frame.push(header_byte1);
    frame.push(header_byte2);
    frame.push(0x00); // reserved
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    frame
}

/// Gzip-compress a byte slice.
fn gzip_bytes(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    // GzEncoder::write only errors on the underlying writer, which is a Vec
    // and never fails; unwrap is safe here.
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

// ── Frame parsing ────────────────────────────────────────────────────

/// A parsed server->client frame.
enum ServerFrame {
    /// A normal response carrying (possibly partial) transcription text.
    Response { text: String, is_last: bool },
    /// An error frame.
    Error { code: u32, message: String },
}

/// Parse a server binary frame.
///
/// Server layout: `[4B header][optional 4B sequence][4B payload_size][payload]`.
/// The 4-byte sequence is present only when `flags & 0x1` (positive sequence)
/// or `flags & 0x3 == 0x3` (last-negative-seq).
fn parse_server_frame(data: &[u8]) -> Result<ServerFrame, AsrError> {
    if data.len() < 8 {
        return Err(AsrError::EngineError {
            engine: "doubao-asr".into(),
            message: format!("frame too short: {} bytes", data.len()),
        });
    }

    let msg_type = (data[1] >> 4) & 0x0F;
    let flags = data[1] & 0x0F;
    let serialization = (data[2] >> 4) & 0x0F;
    let compression = data[2] & 0x0F;

    // The sequence field is present when the sequence-related flag bits are set.
    let has_sequence = (flags & 0x01) != 0 || flags == 0x03;
    let mut offset = 4;
    if has_sequence {
        offset += 4; // skip the 4-byte sequence number
        if data.len() < offset + 4 {
            return Err(AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: "frame truncated at sequence field".into(),
            });
        }
    }

    // is_last when the "last packet" flag bit is set (0x2 or 0x3).
    let is_last = (flags & 0x02) != 0;

    let payload_size = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;

    let payload = if payload_size == 0 {
        &[][..]
    } else {
        if data.len() < offset + payload_size {
            return Err(AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: format!(
                    "payload truncated: declared {} bytes, have {}",
                    payload_size,
                    data.len() - offset
                ),
            });
        }
        &data[offset..offset + payload_size]
    };

    match msg_type {
        MSG_SERVER_RESPONSE => {
            let text = if payload.is_empty() {
                String::new()
            } else {
                let raw = maybe_gunzip(payload, compression)?;
                let json_str = String::from_utf8_lossy(&raw);
                extract_result_text(&json_str, serialization)
            };
            Ok(ServerFrame::Response { text, is_last })
        }
        MSG_SERVER_ERROR => {
            // Error layout: [4B header][4B error_code (BE u32)][4B msg_size][msg]
            // (no sequence field on error frames per the reference protocol.)
            if payload.len() < 4 {
                return Err(AsrError::EngineError {
                    engine: "doubao-asr".into(),
                    message: "error frame missing code".into(),
                });
            }
            let code = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let msg_bytes = &payload[4..];
            let raw = maybe_gunzip(msg_bytes, compression)?;
            let message = String::from_utf8_lossy(&raw).to_string();
            Ok(ServerFrame::Error { code, message })
        }
        other => Err(AsrError::EngineError {
            engine: "doubao-asr".into(),
            message: format!("unexpected message type: 0x{:x}", other),
        }),
    }
}

/// Decompress a payload if it was gzip-compressed; otherwise return as-is.
fn maybe_gunzip(data: &[u8], compression: u8) -> Result<Vec<u8>, AsrError> {
    if compression == COMP_GZIP {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut decoder = GzDecoder::new(data);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).map_err(|e| AsrError::EngineError {
            engine: "doubao-asr".into(),
            message: format!("gzip decompress failed: {}", e),
        })?;
        Ok(out)
    } else {
        Ok(data.to_vec())
    }
}

/// Extract the `result.text` field from a server response JSON.
///
/// The response shape is `{ "result": { "text": "...", "utterances": [...] } }`.
/// If `serialization != JSON`, the payload isn't JSON we can parse - return empty.
fn extract_result_text(json_str: &str, serialization: u8) -> String {
    if serialization != SER_JSON {
        return String::new();
    }
    #[derive(serde::Deserialize)]
    struct ResponsePayload {
        #[serde(default)]
        result: Option<ResultField>,
    }
    #[derive(serde::Deserialize)]
    struct ResultField {
        #[serde(default)]
        text: Option<String>,
    }
    serde_json::from_str::<ResponsePayload>(json_str)
        .ok()
        .and_then(|p| p.result)
        .and_then(|r| r.text)
        .unwrap_or_default()
}

// ── Handshake ────────────────────────────────────────────────────────

/// Build the WebSocket upgrade request with the Doubao auth headers.
fn build_handshake_request(api_key: &str) -> Result<Request<()>, AsrError> {
    let request_id = Uuid::new_v4().to_string();
    Request::builder()
        .method("GET")
        .uri(ASR_ENDPOINT)
        .header("Host", "openspeech.bytedance.com")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .header("X-Api-Key", api_key)
        .header("X-Api-Resource-Id", RESOURCE_ID)
        .header("X-Api-Request-Id", &request_id)
        .header("X-Api-Sequence", "-1")
        .body(())
        .map_err(|e| AsrError::EngineError {
            engine: "doubao-asr".into(),
            message: format!("failed to build handshake request: {}", e),
        })
}
