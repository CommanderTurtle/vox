use std::io::Cursor;

use reqwest::{multipart, Client};

#[derive(Debug, Clone)]
pub struct LongCatOptions {
    pub base_url: String,
    pub steps: u32,
    pub guidance_strength: f32,
    pub guidance_method: String,
    pub seed: u64,
    pub duration_scale: f32,
    pub concatenate: bool,
    pub characters_per_request: usize,
}

#[derive(Debug, Clone)]
pub struct LongCatReference {
    pub audio: Vec<u8>,
    pub filename: String,
    pub transcript: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LongCatError {
    #[error("text is empty")]
    EmptyText,
    #[error("reference transcript is empty")]
    EmptyReferenceTranscript,
    #[error("invalid reference audio MIME: {0}")]
    Multipart(String),
    #[error("service request failed: {0}")]
    Request(String),
    #[error("service returned {status}: {body}")]
    Service { status: u16, body: String },
    #[error("could not read synthesized WAV: {0}")]
    Response(String),
    #[error("invalid synthesized WAV: {0}")]
    Wav(String),
}

pub async fn synthesize(
    client: &Client,
    text: &str,
    options: &LongCatOptions,
    reference: Option<&LongCatReference>,
) -> Result<Vec<u8>, LongCatError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(LongCatError::EmptyText);
    }
    if !options.concatenate {
        return synthesize_request(client, text, options, reference).await;
    }

    let maximum = options.characters_per_request.max(1);
    let mut remaining = text.to_string();
    let mut wavs = Vec::new();
    while !remaining.trim().is_empty() {
        let (mut request, mut tail) = take_sentence_request(&remaining, maximum);
        loop {
            match synthesize_request(client, &request, options, reference).await {
                Ok(wav) => {
                    wavs.push(wav);
                    remaining = tail;
                    break;
                }
                Err(error) => {
                    let Some((shorter, deferred)) = retreat_one_sentence(&request) else {
                        return Err(error);
                    };
                    request = shorter;
                    tail = join_text(&deferred, &tail);
                }
            }
        }
    }
    merge_wav_chunks(&wavs)
}

async fn synthesize_request(
    client: &Client,
    text: &str,
    options: &LongCatOptions,
    reference: Option<&LongCatReference>,
) -> Result<Vec<u8>, LongCatError> {
    let mut form = multipart::Form::new()
        .text("text", text.to_string())
        .text("steps", options.steps.to_string())
        .text("guidance_strength", options.guidance_strength.to_string())
        .text("guidance_method", options.guidance_method.clone())
        .text("seed", options.seed.to_string())
        .text("duration_scale", options.duration_scale.to_string());
    if let Some(reference) = reference {
        if reference.transcript.trim().is_empty() {
            return Err(LongCatError::EmptyReferenceTranscript);
        }
        let part = multipart::Part::bytes(reference.audio.clone())
            .file_name(reference.filename.clone())
            .mime_str(audio_mime(&reference.filename))
            .map_err(|error| LongCatError::Multipart(error.to_string()))?;
        form = form
            .text("prompt_text", reference.transcript.trim().to_string())
            .part("prompt_audio", part);
    }
    let response = client
        .post(format!(
            "{}/api/synthesize",
            options.base_url.trim_end_matches('/')
        ))
        .multipart(form)
        .send()
        .await
        .map_err(|error| LongCatError::Request(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(LongCatError::Service {
            status: status.as_u16(),
            body: body.chars().take(400).collect(),
        });
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| LongCatError::Response(error.to_string()))
}

fn audio_mime(filename: &str) -> &'static str {
    match std::path::Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        _ => "audio/wav",
    }
}

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
        if matches!(character, '.' | '?' | '!' | '…' | '。' | '？' | '！') {
            pending = Some(end);
        } else if pending.is_some()
            && matches!(
                character,
                '"' | '\'' | '”' | '’' | ')' | ']' | '}' | '»' | '」' | '』'
            )
        {
            pending = Some(end);
        } else if let Some(boundary) = pending.take() {
            // A sentence may begin immediately after punctuation (common in
            // CJK text). Finalize the preceding boundary whether or not the
            // next character is whitespace.
            boundaries.push(boundary);
        }
    }
    if let Some(boundary) = pending {
        boundaries.push(boundary);
    }
    boundaries
}

fn join_text(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", right) => right.to_string(),
        (left, "") => left.to_string(),
        (left, right) => format!("{left} {right}"),
    }
}

fn merge_wav_chunks(chunks: &[Vec<u8>]) -> Result<Vec<u8>, LongCatError> {
    if chunks.is_empty() {
        return Err(LongCatError::EmptyText);
    }
    if chunks.len() == 1 {
        return Ok(chunks[0].clone());
    }
    let first = hound::WavReader::new(Cursor::new(&chunks[0]))
        .map_err(|error| LongCatError::Wav(error.to_string()))?;
    let spec = first.spec();
    drop(first);
    let mut output = Vec::new();
    {
        let mut writer = hound::WavWriter::new(Cursor::new(&mut output), spec)
            .map_err(|error| LongCatError::Wav(error.to_string()))?;
        for chunk in chunks {
            let mut reader = hound::WavReader::new(Cursor::new(chunk))
                .map_err(|error| LongCatError::Wav(error.to_string()))?;
            if reader.spec() != spec {
                return Err(LongCatError::Wav(
                    "incompatible WAV formats across requests".into(),
                ));
            }
            match spec.sample_format {
                hound::SampleFormat::Float => {
                    for sample in reader.samples::<f32>() {
                        writer
                            .write_sample(
                                sample.map_err(|error| LongCatError::Wav(error.to_string()))?,
                            )
                            .map_err(|error| LongCatError::Wav(error.to_string()))?;
                    }
                }
                hound::SampleFormat::Int => {
                    for sample in reader.samples::<i32>() {
                        writer
                            .write_sample(
                                sample.map_err(|error| LongCatError::Wav(error.to_string()))?,
                            )
                            .map_err(|error| LongCatError::Wav(error.to_string()))?;
                    }
                }
            }
        }
        writer
            .finalize()
            .map_err(|error| LongCatError::Wav(error.to_string()))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{retreat_one_sentence, take_sentence_request};

    #[test]
    fn scans_backward_to_previous_sentence() {
        let (request, tail) =
            take_sentence_request("First sentence. Second sentence? Final one!", 31);
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
    fn failure_retreats_one_sentence() {
        let (request, deferred) = retreat_one_sentence("First. Second?").unwrap();
        assert_eq!(request, "First.");
        assert_eq!(deferred, "Second?");
    }

    #[test]
    fn supports_full_width_sentence_boundaries() {
        let (request, tail) = take_sentence_request("第一句。第二句？第三句！", 5);
        assert_eq!(request, "第一句。");
        assert_eq!(tail, "第二句？第三句！");
    }
}
