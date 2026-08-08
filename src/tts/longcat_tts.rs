//! LongCat local voice-cloning adapter.
//!
//! Text is split before it reaches the service. Every request represents at
//! most twenty estimated seconds (and normally less when reference-audio
//! conditioning can stretch delivery). Returned PCM WAV files are decoded
//! and reassembled as one valid WAV container for Vox playback.

use std::io::Cursor;
use std::path::PathBuf;

use async_trait::async_trait;
use reqwest::multipart;

use crate::config::LongCatTtsConfig;
use crate::tts::{TtsEngine, TtsError};

pub struct LongCatTtsEngine {
    config: LongCatTtsConfig,
    client: reqwest::Client,
}

impl LongCatTtsEngine {
    pub fn new(config: &LongCatTtsConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(1800))
            .build()
            .expect("Failed to build LongCat HTTP client");
        Self {
            config: config.clone(),
            client,
        }
    }

    fn error(message: impl Into<String>) -> TtsError {
        TtsError::EngineError {
            engine: "longcat".into(),
            message: message.into(),
        }
    }

    async fn synthesize_chunk(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let mut form = multipart::Form::new()
            .text("text", text.to_string())
            .text("steps", self.config.steps.to_string())
            .text(
                "guidance_strength",
                self.config.guidance_strength.to_string(),
            )
            .text("guidance_method", self.config.guidance_method.clone())
            .text("seed", self.config.seed.to_string())
            .text("duration_scale", self.config.duration_scale.to_string());

        let (prompt_audio_path, prompt_text) = if let Some(profile) = self.config.active_voice() {
            if profile.audio_path.trim().is_empty() || profile.transcript_path.trim().is_empty() {
                return Err(Self::error(
                    "the active LongCat voice profile requires both an audio file and transcript file",
                ));
            }
            let transcript_path = configured_path(&profile.transcript_path);
            let transcript =
                tokio::fs::read_to_string(&transcript_path)
                    .await
                    .map_err(|error| {
                        Self::error(format!(
                            "could not read reference transcript {}: {error}",
                            transcript_path.display()
                        ))
                    })?;
            (configured_path(&profile.audio_path), transcript)
        } else {
            (
                configured_path(&self.config.prompt_audio_path),
                self.config.prompt_text.clone(),
            )
        };

        if !prompt_audio_path.as_os_str().is_empty() {
            if prompt_text.trim().is_empty() {
                return Err(Self::error(
                    "the reference transcript is empty for the selected LongCat voice",
                ));
            }
            let path = prompt_audio_path.as_path();
            let bytes = tokio::fs::read(path).await.map_err(|error| {
                Self::error(format!(
                    "could not read reference voice {}: {error}",
                    path.display()
                ))
            })?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("reference.wav")
                .to_string();
            let mime = match path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "mp3" => "audio/mpeg",
                "m4a" => "audio/mp4",
                _ => "audio/wav",
            };
            let audio = multipart::Part::bytes(bytes)
                .file_name(name)
                .mime_str(mime)
                .map_err(|error| Self::error(format!("invalid audio MIME: {error}")))?;
            form = form
                .text("prompt_text", prompt_text.trim().to_string())
                .part("prompt_audio", audio);
        }

        let url = format!(
            "{}/api/synthesize",
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
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::error(format!(
                "service returned {}: {}",
                status.as_u16(),
                body.chars().take(400).collect::<String>()
            )));
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| Self::error(format!("could not read synthesized WAV: {error}")))
    }
}

/// Paths pasted from file dialogs, shells, or config snippets may carry one
/// pair of wrapping quotes. Windows treats those quotes as invalid filename
/// characters, so remove only a matching outer pair and preserve the path
/// itself verbatim.
fn configured_path(value: &str) -> PathBuf {
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
        .unwrap_or(value);
    PathBuf::from(value)
}

#[async_trait]
impl TtsEngine for LongCatTtsEngine {
    fn name(&self) -> &'static str {
        "longcat"
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let limit = safe_chunk_limit(&self.config);
        let chunks = split_for_duration(text, limit);
        if chunks.is_empty() {
            return Err(Self::error("text is empty"));
        }
        log::info!(
            "LongCat synthesis: {} chunk(s), {:.1}s estimated ceiling per request",
            chunks.len(),
            limit
        );
        let mut audio = Vec::with_capacity(chunks.len());
        for (index, chunk) in chunks.iter().enumerate() {
            log::info!(
                "LongCat chunk {}/{}: {} chars",
                index + 1,
                chunks.len(),
                chunk.chars().count()
            );
            audio.push(self.synthesize_chunk(chunk).await?);
        }
        merge_wav_chunks(&audio)
    }
}

/// LongCat can stretch cloned speech relative to its transcript. Reserving a
/// 1.5x conditioning margin keeps each generated request under the user's
/// hard twenty-second accuracy boundary without changing the server.
fn safe_chunk_limit(config: &LongCatTtsConfig) -> f32 {
    let requested = config.max_chunk_seconds.clamp(1.0, 20.0);
    let duration_scale = config.duration_scale.max(1.0);
    let has_voice = config.active_voice().is_some() || !config.prompt_audio_path.trim().is_empty();
    let conditioning_margin = if !has_voice { 1.0 } else { 1.5 };
    requested / duration_scale / conditioning_margin
}

fn estimated_seconds(text: &str) -> f32 {
    let mut chinese = 0usize;
    let mut latin = 0usize;
    let mut other = 0usize;
    for character in text.chars().filter(|character| !character.is_whitespace()) {
        if ('\u{4e00}'..='\u{9fff}').contains(&character) {
            chinese += 1;
        } else if character.is_alphabetic() {
            latin += 1;
        } else {
            other += 1;
        }
    }
    if chinese > latin {
        chinese += other;
    } else {
        latin += other;
    }
    chinese as f32 * 0.21 + latin as f32 * 0.082
}

fn split_for_duration(text: &str, maximum: f32) -> Vec<String> {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return Vec::new();
    }

    let mut units = Vec::new();
    for sentence in clean.split_inclusive(|character: char| {
        matches!(
            character,
            '.' | '!' | '?' | ';' | ':' | '\n' | '。' | '！' | '？' | '；'
        )
    }) {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        if estimated_seconds(sentence) <= maximum {
            units.push(sentence.to_string());
        } else if sentence.split_whitespace().count() > 1 {
            units.extend(sentence.split_whitespace().map(str::to_string));
        } else {
            units.extend(sentence.chars().map(|character| character.to_string()));
        }
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for unit in units {
        let separator = if needs_space_between(&current, &unit) {
            " "
        } else {
            ""
        };
        let candidate = format!("{current}{separator}{unit}");
        if !current.is_empty() && estimated_seconds(&candidate) > maximum {
            chunks.push(current);
            current = unit;
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn needs_space_between(left: &str, right: &str) -> bool {
    let Some(last) = left.chars().last() else {
        return false;
    };
    let Some(first) = right.chars().next() else {
        return false;
    };
    let is_cjk = |character: char| ('\u{4e00}'..='\u{9fff}').contains(&character);
    !is_cjk(last) && !is_cjk(first)
}

fn merge_wav_chunks(chunks: &[Vec<u8>]) -> Result<Vec<u8>, TtsError> {
    if chunks.len() == 1 {
        return Ok(chunks[0].clone());
    }
    let first = hound::WavReader::new(Cursor::new(&chunks[0]))
        .map_err(|error| LongCatTtsEngine::error(format!("invalid WAV response: {error}")))?;
    let spec = first.spec();
    drop(first);

    let mut output = Vec::new();
    {
        let mut writer = hound::WavWriter::new(Cursor::new(&mut output), spec)
            .map_err(|error| LongCatTtsEngine::error(format!("WAV writer failed: {error}")))?;
        for chunk in chunks {
            let mut reader = hound::WavReader::new(Cursor::new(chunk)).map_err(|error| {
                LongCatTtsEngine::error(format!("invalid LongCat WAV response: {error}"))
            })?;
            if reader.spec() != spec {
                return Err(LongCatTtsEngine::error(
                    "LongCat returned incompatible WAV formats across chunks",
                ));
            }
            match spec.sample_format {
                hound::SampleFormat::Float => {
                    for sample in reader.samples::<f32>() {
                        writer
                            .write_sample(sample.map_err(|error| {
                                LongCatTtsEngine::error(format!("WAV decode failed: {error}"))
                            })?)
                            .map_err(|error| {
                                LongCatTtsEngine::error(format!("WAV encode failed: {error}"))
                            })?;
                    }
                }
                hound::SampleFormat::Int => {
                    for sample in reader.samples::<i32>() {
                        writer
                            .write_sample(sample.map_err(|error| {
                                LongCatTtsEngine::error(format!("WAV decode failed: {error}"))
                            })?)
                            .map_err(|error| {
                                LongCatTtsEngine::error(format!("WAV encode failed: {error}"))
                            })?;
                    }
                }
            }
        }
        writer
            .finalize()
            .map_err(|error| LongCatTtsEngine::error(format!("WAV finalize failed: {error}")))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_stay_within_requested_estimate() {
        let text = "This is a deliberately long sentence with enough words to exceed a tiny duration budget, followed by another sentence.";
        let chunks = split_for_duration(text, 1.2);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| estimated_seconds(chunk) <= 1.2));
        assert_eq!(chunks.join(" ").replace("  ", " "), text);
    }

    #[test]
    fn longcat_never_accepts_more_than_twenty_seconds() {
        let mut config = LongCatTtsConfig::default();
        config.max_chunk_seconds = 99.0;
        assert_eq!(safe_chunk_limit(&config), 20.0);
        config.prompt_audio_path = "voice.wav".into();
        assert!(safe_chunk_limit(&config) < 14.0);
    }
}
