//! Volcano Engine **Doubao streaming ASR 2.0** (`volc.seedasr.sauc.duration`).
//!
//! Uses the WebSocket `bigmodel_nostream` endpoint, which is designed for the
//! "send all audio, then receive one final result" flow - a perfect fit for
//! vox's record-then-transcribe model (no partial-result handling needed).
//!
//! Auth is via `X-Api-Key` + `X-Api-Resource-Id` handshake headers (NOT
//! `Authorization: Bearer`); the key is shared with the Doubao TTS engine.
//! Uses the Agent Plan endpoint (`/api/v3/plan/sauc/...`) with the Agent Plan
//! subscription key.
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
const ASR_ENDPOINT: &str = "wss://openspeech.bytedance.com/api/v3/plan/sauc/bigmodel_nostream";
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
                        format!(
                            "HTTP {} - {}",
                            status,
                            body.chars().take(300).collect::<String>()
                        )
                    }
                    other => other.to_string(),
                };
                return Err(AsrError::EngineError {
                    engine: "doubao-asr".into(),
                    message: format!(
                        "WebSocket handshake failed: {}. \
                         Hint: verify the Agent Plan subscription key in [asr.doubao].api_key \
                         and that the plan covers the ASR resource.",
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

        // 1. Send the FULL_CLIENT_REQUEST (JSON config).
        // NOTE: gzip on the config payload triggers "unable to ungzip payload:
        // EOF" on the server side, so we send it uncompressed (compression=0).
        let config_json = build_config_json();
        let full_request = build_frame(
            MSG_FULL_CLIENT_REQUEST,
            FLAG_NO_SEQUENCE,
            SER_JSON,
            COMP_NONE,
            &config_json,
            false, // no gzip
        );
        ws_stream
            .send(Message::Binary(full_request))
            .await
            .map_err(|e| AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: format!("send full request failed: {}", e),
            })?;

        // 2. Stream the WAV bytes in AUDIO_ONLY chunks (uncompressed).
        let mut offset = 0;
        while offset < audio_wav.len() {
            let end = (offset + AUDIO_CHUNK_SIZE).min(audio_wav.len());
            let chunk = &audio_wav[offset..end];
            let frame = build_frame(
                MSG_AUDIO_ONLY,
                FLAG_NO_SEQUENCE,
                SER_NONE,
                COMP_NONE,
                chunk,
                false,
            );
            ws_stream
                .send(Message::Binary(frame))
                .await
                .map_err(|e| AsrError::EngineError {
                    engine: "doubao-asr".into(),
                    message: format!("send audio chunk failed: {}", e),
                })?;
            offset = end;
        }

        // 3. Send the empty last packet to signal end-of-audio.
        let last_packet = build_frame(
            MSG_AUDIO_ONLY,
            FLAG_LAST_NO_SEQUENCE,
            SER_NONE,
            COMP_NONE,
            &[],
            false, // empty payload, no need to gzip
        );
        ws_stream
            .send(Message::Binary(last_packet))
            .await
            .map_err(|e| AsrError::EngineError {
                engine: "doubao-asr".into(),
                message: format!("send last packet failed: {}", e),
            })?;
        log::debug!(
            "doubao-asr: sent {} bytes of audio in chunks",
            audio_wav.len()
        );

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

    // Error frames (msg_type 0xF) have a distinct layout with NO sequence
    // field and NO payload_size:  [4B header][4B error_code][4B msg_size][msg].
    // They must be parsed before the generic payload_size read below, which
    // would otherwise mistake error_code for the payload length.
    if msg_type == MSG_SERVER_ERROR {
        return parse_error_frame(data, offset, serialization, compression);
    }

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
        other => Err(AsrError::EngineError {
            engine: "doubao-asr".into(),
            message: format!("unexpected message type: 0x{:x}", other),
        }),
    }
}

/// Parse a SERVER_ERROR frame.
///
/// Layout: `[4B header][4B error_code (BE u32)][4B msg_size (BE u32)][msg]`.
/// `offset` is where parsing should start (just past the 4-byte header and
/// any sequence field, though error frames carry neither in practice).
fn parse_error_frame(
    data: &[u8],
    offset: usize,
    serialization: u8,
    compression: u8,
) -> Result<ServerFrame, AsrError> {
    if data.len() < offset + 8 {
        return Err(AsrError::EngineError {
            engine: "doubao-asr".into(),
            message: "error frame too short for code + size".into(),
        });
    }
    let code = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    let msg_size = u32::from_be_bytes([
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]) as usize;
    let msg_start = offset + 8;
    let msg_bytes = if data.len() >= msg_start + msg_size {
        &data[msg_start..msg_start + msg_size]
    } else {
        // Truncated message body - take what we have.
        &data[msg_start..]
    };
    let raw = maybe_gunzip(msg_bytes, compression)?;
    let mut message = String::from_utf8_lossy(&raw).to_string();
    // The error body is JSON like {"error":"..."} on the plan path, or a raw
    // string on the standard path. Try to extract the inner message.
    if serialization == SER_JSON {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&message) {
            if let Some(s) = v.get("error").and_then(|e| e.as_str()) {
                message = s.to_string();
            }
        }
    }
    Ok(ServerFrame::Error { code, message })
}

/// Decompress a payload if it was gzip-compressed; otherwise return as-is.
fn maybe_gunzip(data: &[u8], compression: u8) -> Result<Vec<u8>, AsrError> {
    if compression == COMP_GZIP {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut decoder = GzDecoder::new(data);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| AsrError::EngineError {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a server RESPONSE frame (msg_type 0x9) with the given flags,
    /// serialization, compression and payload, mimicking what the server sends.
    /// When flags indicate a sequence field is present (0x1 or 0x3), a 4-byte
    /// sequence number is inserted before the payload_size, matching the wire
    /// format the server actually emits.
    fn build_server_response_frame(
        flags: u8,
        serialization: u8,
        compression: u8,
        payload: &[u8],
    ) -> Vec<u8> {
        let header_byte1 = (MSG_SERVER_RESPONSE << 4) | flags;
        let header_byte2 = (serialization << 4) | compression;
        let has_sequence = (flags & 0x01) != 0 || flags == 0x03;
        let mut frame = Vec::with_capacity(8 + payload.len() + if has_sequence { 4 } else { 0 });
        frame.push(HDR_VERSION);
        frame.push(header_byte1);
        frame.push(header_byte2);
        frame.push(0x00);
        if has_sequence {
            frame.extend_from_slice(&1u32.to_be_bytes()); // sequence = 1
        }
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    /// Build a server ERROR frame (msg_type 0xF):
    /// `[4B header][4B error_code][4B msg_size][msg]`.
    fn build_server_error_frame(code: u32, message: &str) -> Vec<u8> {
        let msg_bytes = message.as_bytes();
        let header_byte1 = (MSG_SERVER_ERROR << 4) | FLAG_NO_SEQUENCE;
        let mut frame = Vec::with_capacity(12 + msg_bytes.len());
        frame.push(HDR_VERSION);
        frame.push(header_byte1);
        frame.push((SER_JSON << 4) | COMP_NONE); // serialization=JSON so inner msg is extracted
        frame.push(0x00);
        frame.extend_from_slice(&code.to_be_bytes());
        frame.extend_from_slice(&(msg_bytes.len() as u32).to_be_bytes());
        frame.extend_from_slice(msg_bytes);
        frame
    }

    #[test]
    fn test_build_frame_layout() {
        // A frame built without gzip: [4B header][4B size BE][payload].
        let payload = b"hello";
        let frame = build_frame(
            MSG_AUDIO_ONLY,
            FLAG_NO_SEQUENCE,
            SER_NONE,
            COMP_NONE,
            payload,
            false,
        );
        assert_eq!(frame.len(), 8 + payload.len());
        assert_eq!(frame[0], HDR_VERSION);
        assert_eq!((frame[1] >> 4) & 0x0F, MSG_AUDIO_ONLY);
        assert_eq!(frame[1] & 0x0F, FLAG_NO_SEQUENCE);
        let size = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]);
        assert_eq!(size as usize, payload.len());
        assert_eq!(&frame[8..], payload);
    }

    #[test]
    fn test_parse_response_frame_with_sequence() {
        // Server response with flags=0x1 (positive sequence) carries a 4-byte
        // sequence field before payload_size. This is the common case observed
        // from the real server.
        let json = br#"{"result":{"text":"hello world"}}"#;
        let frame = build_server_response_frame(0x01, SER_JSON, COMP_NONE, json);
        match parse_server_frame(&frame).unwrap() {
            ServerFrame::Response { text, is_last } => {
                assert_eq!(text, "hello world");
                assert!(!is_last);
            }
            ServerFrame::Error { .. } => panic!("expected Response, got Error"),
        }
    }

    #[test]
    fn test_parse_response_frame_is_last() {
        // flags=0x2 -> last packet marker.
        let json = br#"{"result":{"text":"final"}}"#;
        let frame = build_server_response_frame(0x02, SER_JSON, COMP_NONE, json);
        match parse_server_frame(&frame).unwrap() {
            ServerFrame::Response { is_last, .. } => assert!(is_last),
            ServerFrame::Error { .. } => panic!("expected Response, got Error"),
        }
    }

    #[test]
    fn test_parse_error_frame() {
        // Regression: error frames (msg_type 0xF) have a different layout -
        // [4B header][4B error_code][4B msg_size][msg] - and must not be parsed
        // as if error_code were a payload_size (that caused "declared 45000000
        // bytes" panics before the fix).
        let frame = build_server_error_frame(45_000_000, r#"{"error":"decode failed"}"#);
        match parse_server_frame(&frame).unwrap() {
            ServerFrame::Error { code, message } => {
                assert_eq!(code, 45_000_000);
                assert_eq!(message, "decode failed");
            }
            ServerFrame::Response { .. } => panic!("expected Error, got Response"),
        }
    }

    #[test]
    fn test_parse_frame_too_short() {
        assert!(parse_server_frame(&[0x11, 0x91]).is_err());
    }

    #[test]
    fn test_extract_result_text() {
        assert_eq!(
            extract_result_text(r#"{"result":{"text":"abc"}}"#, SER_JSON),
            "abc"
        );
        // Missing result -> empty.
        assert_eq!(extract_result_text(r#"{"foo":1}"#, SER_JSON), "");
        // Missing text field -> empty.
        assert_eq!(extract_result_text(r#"{"result":{}}"#, SER_JSON), "");
        // Non-JSON serialization -> empty.
        assert_eq!(extract_result_text("anything", SER_NONE), "");
    }

    #[test]
    fn test_build_config_json_fields() {
        let json = build_config_json();
        let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(v["audio"]["format"], "wav");
        assert_eq!(v["audio"]["rate"], 16000);
        assert_eq!(v["request"]["model_name"], "bigmodel");
        assert_eq!(v["request"]["enable_punc"], true);
    }

    #[test]
    fn test_gzip_roundtrip() {
        // gzip_bytes + maybe_gunzip should round-trip arbitrary data.
        let data = b"The quick brown fox jumps over the lazy dog. 1234567890";
        let compressed = gzip_bytes(data);
        let decompressed = maybe_gunzip(&compressed, COMP_GZIP).unwrap();
        assert_eq!(decompressed, data);
    }
}
