//! LongCat local voice-cloning adapter.
//!
//! Long synthesis can be split by an explicit word-unit budget before it
//! reaches the service. Returned PCM WAV files are decoded and reassembled
//! as one valid WAV container for whichever Vox output is selected.

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
        let chunks = if self.config.concatenate_chunks {
            split_by_word_units(text, self.config.words_per_chunk.max(1))
        } else {
            let text = text.trim();
            (!text.is_empty())
                .then(|| vec![text.to_string()])
                .unwrap_or_default()
        };
        if chunks.is_empty() {
            return Err(Self::error("text is empty"));
        }
        if self.config.concatenate_chunks {
            log::info!(
                "LongCat synthesis: {} word-bounded chunk(s), {} units maximum per request",
                chunks.len(),
                self.config.words_per_chunk.max(1)
            );
        } else {
            log::info!("LongCat synthesis: one unsplit request");
        }
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

/// Split on explicit word-like units rather than guessing spoken duration.
/// Space-delimited words are one unit. Each CJK character is one unit so a
/// sentence without spaces is still bounded and can use the same control.
fn split_by_word_units(text: &str, maximum: usize) -> Vec<String> {
    let maximum = maximum.max(1);
    let mut units: Vec<(String, bool)> = Vec::new();
    for token in text.split_whitespace() {
        let mut token_units = Vec::new();
        let mut segment = String::new();
        for character in token.chars() {
            if is_cjk(character) {
                if !segment.is_empty() {
                    token_units.push(std::mem::take(&mut segment));
                }
                token_units.push(character.to_string());
            } else {
                segment.push(character);
            }
        }
        if !segment.is_empty() {
            token_units.push(segment);
        }
        for (index, unit) in token_units.into_iter().enumerate() {
            units.push((unit, index > 0));
        }
    }
    if units.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for (unit, same_token) in units {
        if count == maximum {
            chunks.push(std::mem::take(&mut current));
            count = 0;
        }
        if !current.is_empty() && !same_token {
            current.push(' ');
        }
        current.push_str(&unit);
        count += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn is_cjk(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{ac00}'..='\u{d7af}'
    )
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
    fn chunks_by_explicit_word_budget() {
        let text = "This is a deliberate six word sentence.";
        let chunks = split_by_word_units(text, 3);
        assert_eq!(chunks, ["This is a", "deliberate six word", "sentence."]);
    }

    #[test]
    fn cjk_without_spaces_is_bounded() {
        let chunks = split_by_word_units("你好世界测试", 2);
        assert_eq!(chunks, ["你好", "世界", "测试"]);
    }
}
