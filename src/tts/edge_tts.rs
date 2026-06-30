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
/// GEC version string the server expects.
const SEC_MS_GEC_VERSION: &str = "1-130.0.2849.68";
/// Audio output format: 24 kHz 16-bit mono WAV (RIFF).
const AUDIO_OUTPUT_FORMAT: &str = "riff-24khz-16bit-mono-pcm";

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

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        if text.trim().is_empty() {
            return Err(TtsError::EngineError {
                engine: "edge-tts".into(),
                message: "empty text".into(),
            });
        }

        let request = build_connect_request()?;
        let (ws_stream, _response) = connect_async(request)
            .await
            .map_err(|e| TtsError::EngineError {
                engine: "edge-tts".into(),
                message: format!("WebSocket connect failed: {}", e),
            })?;

        let (mut write, mut read) = ws_stream.split();

        // 1. speech.config message — select output format.
        let timestamp = iso8601_now();
        let config_msg = format!(
            "X-Timestamp:{ts}\r\n\
             Content-Type:application/json; charset=utf-8\r\n\
             Path:speech.config\r\n\r\n\
             {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":\
             {{\"sentenceBoundaryEnabled\":false,\"wordBoundaryEnabled\":false}},\
             \"outputFormat\":\"{fmt}\"}}}}}}}}",
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
            "X-Timestamp:{ts}\r\n\
             Content-Type:application/ssml+xml;charset=UTF-8\r\n\
             Path:ssml\r\n\r\n\
             {ssml}",
            ts = iso8601_now(),
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
/// and headers (Sec-MS-GEC token, Origin, User-Agent).
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
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .header("Origin", "https://edge.microsoft.com")
        .header(
            "Accept-Language",
            "en-US,en;q=0.9",
        )
        .body(())
        .map_err(|e| TtsError::EngineError {
            engine: "edge-tts".into(),
            message: format!("failed to build request: {}", e),
        })
}

/// Generate the `Sec-MS-GEC` token: uppercased SHA-256 of
/// `{windows_file_time_ticks}{TRUSTED_CLIENT_TOKEN}`.
fn generate_sec_ms_gec() -> String {
    use sha2::{Digest, Sha256};

    let unix_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Convert Unix seconds → Windows file time ticks (100ns since 1601-01-01).
    let ticks = (unix_secs + 11_644_473_600) * 10_000_000;

    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", ticks, TRUSTED_CLIENT_TOKEN).as_bytes());
    let hash = hasher.finalize();
    // Uppercase hex.
    hash.iter().map(|b| format!("{:02X}", b)).collect::<String>()
}

/// Current UTC time as an ISO-8601 string with milliseconds and a `Z`
/// suffix, e.g. `2026-06-30T12:34:56.789Z`.
fn iso8601_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}
