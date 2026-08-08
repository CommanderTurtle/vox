//! LongCat local voice-cloning adapter.
//!
//! Long synthesis can be divided at sentence-ending punctuation before it
//! reaches the service. A configured character position tells Vox where to
//! begin scanning backward. Returned PCM WAV files are reassembled as one
//! valid WAV container for whichever Vox output is selected.

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
        let text = text.trim();
        if text.is_empty() {
            return Err(Self::error("text is empty"));
        }

        if !self.config.concatenate_chunks {
            log::info!("LongCat synthesis: one unsplit request");
            return self.synthesize_chunk(text).await;
        }

        let maximum = self.config.characters_per_request.max(1);
        log::info!(
            "LongCat synthesis: sentence-boundary concatenation at {} characters",
            maximum
        );
        let mut remaining = text.to_string();
        let mut audio = Vec::new();
        while !remaining.trim().is_empty() {
            let (mut request, mut tail) = take_sentence_request(&remaining, maximum);
            loop {
                let request_number = audio.len() + 1;
                log::info!(
                    "LongCat request {}: {} characters",
                    request_number,
                    request.chars().count()
                );
                match self.synthesize_chunk(&request).await {
                    Ok(wav) => {
                        audio.push(wav);
                        remaining = tail;
                        break;
                    }
                    Err(error) => {
                        let Some((shorter, deferred)) = retreat_one_sentence(&request) else {
                            return Err(error);
                        };
                        log::warn!(
                            "LongCat request {} failed; retreating one sentence and retrying: {}",
                            request_number,
                            error
                        );
                        request = shorter;
                        tail = join_text(&deferred, &tail);
                    }
                }
            }
        }

        if audio.len() == 1 {
            return Ok(audio.remove(0));
        }
        log::info!("LongCat concatenating {} successful WAVs", audio.len());
        merge_wav_chunks(&audio)
    }
}

/// Select one request by scanning backward from a character ceiling to the
/// previous sentence-ending punctuation. If no earlier terminator exists,
/// scan forward to the first one; Vox never invents a mid-sentence cut.
fn take_sentence_request(text: &str, maximum: usize) -> (String, String) {
    let text = text.trim();
    if text.chars().count() <= maximum {
        return (text.to_string(), String::new());
    }
    let limit = byte_index_after_characters(text, maximum);
    let boundaries = sentence_boundaries(text);
    let split = boundaries
        .iter()
        .copied()
        .filter(|boundary| *boundary <= limit)
        .next_back()
        .or_else(|| {
            boundaries
                .iter()
                .copied()
                .find(|boundary| *boundary > limit)
        });
    let Some(split) = split else {
        return (text.to_string(), String::new());
    };
    let request = text[..split].trim().to_string();
    let tail = text[split..].trim().to_string();
    if request.is_empty() {
        (text.to_string(), String::new())
    } else {
        (request, tail)
    }
}

/// If a request actually fails, move its final sentence back to the pending
/// text and retry the shorter prefix. Successful generations are never lost.
fn retreat_one_sentence(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let split = sentence_boundaries(text)
        .into_iter()
        .filter(|boundary| *boundary < text.len())
        .next_back()?;
    let shorter = text[..split].trim().to_string();
    let deferred = text[split..].trim().to_string();
    (!shorter.is_empty() && !deferred.is_empty()).then_some((shorter, deferred))
}

fn byte_index_after_characters(text: &str, characters: usize) -> usize {
    text.char_indices()
        .nth(characters)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn sentence_boundaries(text: &str) -> Vec<usize> {
    let mut boundaries = Vec::new();
    let mut pending = None;
    for (index, character) in text.char_indices() {
        let end = index + character.len_utf8();
        if is_sentence_terminator(character) {
            pending = Some(end);
        } else if pending.is_some() && is_sentence_closer(character) {
            pending = Some(end);
        } else if character.is_whitespace() {
            if let Some(boundary) = pending.take() {
                boundaries.push(boundary);
            }
        } else {
            pending = None;
        }
    }
    if let Some(boundary) = pending {
        boundaries.push(boundary);
    }
    boundaries
}

fn is_sentence_terminator(character: char) -> bool {
    matches!(character, '.' | '?' | '!' | '…' | '。' | '？' | '！')
}

fn is_sentence_closer(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '”' | '’' | ')' | ']' | '}' | '»' | '」' | '』'
    )
}

fn join_text(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", right) => right.to_string(),
        (left, "") => left.to_string(),
        (left, right) => format!("{left} {right}"),
    }
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
    fn scans_backward_to_previous_sentence() {
        let text = "First sentence. Second sentence? Final one!";
        let (request, tail) = take_sentence_request(text, 31);
        assert_eq!(request, "First sentence.");
        assert_eq!(tail, "Second sentence? Final one!");
    }

    #[test]
    fn never_splits_an_unpunctuated_sentence() {
        let text = "This sentence deliberately has no terminator";
        let (request, tail) = take_sentence_request(text, 8);
        assert_eq!(request, text);
        assert!(tail.is_empty());
    }

    #[test]
    fn failure_retreats_exactly_one_sentence() {
        let (request, deferred) = retreat_one_sentence("First sentence. Second sentence?").unwrap();
        assert_eq!(request, "First sentence.");
        assert_eq!(deferred, "Second sentence?");
    }

    #[test]
    fn full_width_sentence_boundaries_are_supported() {
        let (request, tail) = take_sentence_request("第一句。第二句？第三句！", 5);
        assert_eq!(request, "第一句。");
        assert_eq!(tail, "第二句？第三句！");
    }
}
