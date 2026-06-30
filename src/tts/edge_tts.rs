//! Microsoft Edge TTS engine — uses the free Edge "Read Aloud" WebSocket
//! endpoint. No API key, no registration required.
//!
//! Protocol (reverse-engineered, same as the `edge-tts` Python library):
//! 1. Open a WebSocket to `wss://speech.platform.bing.com/.../edge/v1`
//!    with a `Sec-MS-GEC` token (SHA-256 of Windows-file-time ticks + a
//!    fixed trusted client token) and the trusted client token.
//! 2. Send a `speech.config` message selecting the audio output format.
//! 3. Send an `ssml` message with the text to synthesize.
//! 4. Read frames until `Path:turn.end`. Binary frames carry audio data
//!    prefixed by a 2-byte big-endian header length.
//!
//! We request `riff-24khz-16bit-mono-pcm` (i.e. WAV) so the existing
//! WAV playback path works unchanged — no MP3 decoder needed.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::tts::{TtsEngine, TtsError};

/// Trusted client token used by the Edge browser (public, well-known).
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
/// GEC version string the server expects: `1-<chromium full version>`.
const SEC_MS_GEC_VERSION: &str = "1-143.0.3650.75";
/// Audio output format requested from the server.
///
/// The Edge service dropped support for several `riff-*` PCM formats, so we
/// request `audio-24khz-48kbitrate-mono-mp3` (the reference client's default)
/// and let the playback layer handle MP3 decoding via the OS player.
const AUDIO_OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";

/// Microsoft Edge TTS engine (free, no API key).
pub struct EdgeTtsEngine {
    /// Voice name, e.g. `zh-CN-XiaoxiaoNeural`, `en-US-AriaNeural`.
    voice: String,
    /// Speech rate, e.g. `+0%`, `+10%`, `-10%`.
    rate: String,
    /// Speech volume, e.g. `+0%`.
    volume: String,
    /// Speech pitch, e.g. `+0Hz`.
    pitch: String,
}

impl EdgeTtsEngine {
    pub fn new(voice: &str, rate: &str, volume: &str, pitch: &str) -> Self {
        Self {
            voice: if voice.is_empty() {
                "zh-CN-XiaoxiaoNeural".to_string()
            } else {
                voice.to_string()
            },
            rate: if rate.is_empty() { "+0%".to_string() } else { rate.to_string() },
            volume: if volume.is_empty() { "+0%".to_string() } else { volume.to_string() },
            pitch: if pitch.is_empty() { "+0Hz".to_string() } else { pitch.to_string() },
        }
    }

    /// Infer the `xml:lang` for the SSML speak element from the voice name
    /// (e.g. `zh-CN-XiaoxiaoNeural` → `zh-CN`).
    fn lang_for_voice(&self) -> String {
        self.voice
            .split('-')
            .take(2)
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Build the SSML document for the given text.
    fn build_ssml(&self, text: &str) -> String {
        // Escape XML special characters in the text body.
        let escaped = text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        format!(
            "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"{lang}\">\
             <voice name=\"{voice}\">\
             <prosody pitch=\"{pitch}\" rate=\"{rate}\" volume=\"{volume}\">{text}</prosody>\
             </voice></speak>",
            lang = self.lang_for_voice(),
            voice = self.voice,
            pitch = self.pitch,
            rate = self.rate,
            volume = self.volume,
            text = escaped,
        )
    }
}

#[async_trait]
impl TtsEngine for EdgeTtsEngine {
    fn name(&self) -> &'static str {
        "edge-tts"
    }

    fn output_format(&self) -> crate::tts::playback::AudioFormat {
        crate::tts::playback::AudioFormat::Mp3
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        if text.trim().is_empty() {
            return Err(TtsError::EngineError {
                engine: "edge-tts".into(),
                message: "empty text".into(),
            });
        }

        let request = build_connect_request()?;
        let (ws_stream, response) = connect_async(request)
            .await
            .map_err(|e| TtsError::EngineError {
                engine: "edge-tts".into(),
                message: format!("WebSocket connect failed: {}", e),
            })?;
        log::debug!("edge-tts WS handshake status: {}", response.status());

        let (mut write, mut read) = ws_stream.split();

        // 1. speech.config message — select output format.
        let timestamp = js_date_string();
        let config_msg = format!(
            "X-RequestId:{rid}\r\n\
             Content-Type:application/json; charset=utf-8\r\n\
             X-Timestamp:{ts}\r\n\
             Path:speech.config\r\n\r\n\
             {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":\
             {{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\
             \"outputFormat\":\"{fmt}\"}}}}}}}}",
            rid = connect_id(),
            ts = timestamp,
            fmt = AUDIO_OUTPUT_FORMAT,
        );
        write.send(Message::Text(config_msg)).await.map_err(|e| {
            TtsError::EngineError {
                engine: "edge-tts".into(),
                message: format!("send config failed: {}", e),
            }
        })?;

        // 2. ssml message — the text to synthesize.
        let ssml = self.build_ssml(text);
        let ssml_msg = format!(
            "X-RequestId:{rid}\r\n\
             Content-Type:application/ssml+xml\r\n\
             X-Timestamp:{ts}\r\n\
             Path:ssml\r\n\r\n\
             {ssml}",
            rid = connect_id(),
            ts = js_date_string(),
            ssml = ssml,
        );
        write.send(Message::Text(ssml_msg)).await.map_err(|e| {
            TtsError::EngineError {
                engine: "edge-tts".into(),
                message: format!("send ssml failed: {}", e),
            }
        })?;

        // 3. Read frames until turn.end, accumulating audio bytes.
        let mut audio: Vec<u8> = Vec::new();
        let mut turned_end = false;
        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| TtsError::EngineError {
                engine: "edge-tts".into(),
                message: format!("recv failed: {}", e),
            })?;
            log::debug!("edge-tts WS frame: {:?}", msg);
            match msg {
                Message::Binary(bin) => {
                    // Binary frame layout:
                    //   [2 bytes BE header length][header bytes][audio bytes]
                    if bin.len() < 2 {
                        continue;
                    }
                    let header_len =
                        u16::from_be_bytes([bin[0], bin[1]]) as usize;
                    let start = 2 + header_len;
                    if start <= bin.len() {
                        audio.extend_from_slice(&bin[start..]);
                    }
                }
                Message::Text(t) => {
                    if t.contains("Path:turn.end") {
                        turned_end = true;
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        if !turned_end && audio.is_empty() {
            return Err(TtsError::EngineError {
                engine: "edge-tts".into(),
                message: "connection closed before any audio was produced".into(),
            });
        }
        if audio.is_empty() {
            return Err(TtsError::EngineError {
                engine: "edge-tts".into(),
                message: "no audio data in response".into(),
            });
        }

        Ok(audio)
    }
}

// ── Edge TTS connection helpers ───────────────────────────────────────

/// Build the WebSocket upgrade request with the required query parameters
/// and headers (Sec-MS-GEC token, Origin, User-Agent, MUID cookie).
fn build_connect_request(
) -> Result<Request<()>, TtsError> {
    let gec = generate_sec_ms_gec();
    let url = format!(
        "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1\
         ?TrustedClientToken={token}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version={ver}",
        token = TRUSTED_CLIENT_TOKEN,
        gec = gec,
        ver = SEC_MS_GEC_VERSION,
    );

    Request::builder()
        .method("GET")
        .uri(url)
        .header("Host", "speech.platform.bing.com")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0",
        )
        .header(
            "Origin",
            "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold",
        )
        .header("Accept-Encoding", "gzip, deflate, br, zstd")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache")
        // MUID cookie — the server closes the socket shortly after the
        // handshake if this is absent.
        .header("Cookie", format!("muid={};", generate_muid()))
        .body(())
        .map_err(|e| TtsError::EngineError {
            engine: "edge-tts".into(),
            message: format!("failed to build request: {}", e),
        })
}

/// Generate a pseudo-random 32-char uppercase hex MUID (machine unique id).
///
/// The reference client uses `secrets.token_hex(16)`; the value is just an
/// opaque per-connection identifier and is not cryptographically verified by
/// the server, so a time-seeded hex string suffices. We avoid pulling in a
/// RNG crate for this.
fn generate_muid() -> String {
    use sha2::{Digest, Sha256};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(now.to_le_bytes());
    hasher.update(b"vox-muid-salt");
    let hash = hasher.finalize();
    hash.iter().take(16).map(|b| format!("{:02X}", b)).collect::<String>()
}

/// Generate the `Sec-MS-GEC` token.
///
/// Algorithm (matches the `edge-tts` reference): take the current Unix time,
/// round it **down to the nearest 5 minutes (300s)**, convert to Windows file
/// time ticks (100ns intervals since 1601-01-01), then uppercase-SHA256 of
/// `{ticks}{TRUSTED_CLIENT_TOKEN}`.
///
/// The 5-minute rounding is mandatory — the server validates against the same
/// 5-minute window, so an unrounded timestamp yields HTTP 403.
fn generate_sec_ms_gec() -> String {
    use sha2::{Digest, Sha256};

    let unix_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Round down to the nearest 5 minutes.
    let rounded = unix_secs - (unix_secs % 300);

    // Windows file time: 100ns intervals since 1601-01-01.
    let ticks = (rounded + 11_644_473_600) as u128 * 10_000_000u128;

    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", ticks, TRUSTED_CLIENT_TOKEN).as_bytes());
    let hash = hasher.finalize();
    // Uppercase hex.
    hash.iter().map(|b| format!("{:02X}", b)).collect::<String>()
}

/// A connection/request id — a UUID v4 without dashes (32 hex chars), like
/// the reference client's `connect_id()`.
fn connect_id() -> String {
    // No `uuid`/`rand` crate; derive a 128-bit value from time + counter and
    // format it as hex. Uniqueness, not cryptographic strength, is what's
    // needed here.
    use sha2::{Digest, Sha256};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(now.to_le_bytes());
    hasher.update(b"vox-rid-salt");
    let hash = hasher.finalize();
    hash.iter().take(16).map(|b| format!("{:02x}", b)).collect::<String>()
}

/// Current UTC time as a JavaScript-style date string, e.g.
/// `Tue Jun 30 2026 12:34:56 GMT+0000 (Coordinated Universal Time)`.
///
/// This matches the reference client's `date_to_string()`, which is what the
/// server expects in the `X-Timestamp` header of SSML/speech.config messages.
fn js_date_string() -> String {
    chrono::Utc::now()
        .format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}
