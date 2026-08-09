use reqwest::{multipart, Client};

#[derive(Debug, Clone)]
pub struct CrisperOptions {
    pub base_url: String,
    pub mode: String,
    pub language: String,
    pub chunk_duration: f32,
    pub stride: f32,
    pub context_words: u32,
    pub max_new_tokens: u32,
    pub hotwords: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CrisperError {
    #[error("invalid audio multipart: {0}")]
    Multipart(String),
    #[error("service request failed: {0}")]
    Request(String),
    #[error("service returned {status}: {body}")]
    Service { status: u16, body: String },
    #[error("invalid CrisperWhisper response: {0}")]
    InvalidResponse(String),
    #[error("CrisperWhisper returned an empty {0} transcript")]
    EmptyTranscript(String),
}

pub fn normalized_mode(mode: &str) -> &'static str {
    if mode.eq_ignore_ascii_case("literal") || mode.eq_ignore_ascii_case("verbatim") {
        "verbatim"
    } else {
        "intended"
    }
}

pub async fn transcribe(
    client: &Client,
    audio: &[u8],
    filename: &str,
    mime: &str,
    options: &CrisperOptions,
) -> Result<String, CrisperError> {
    let mode = normalized_mode(&options.mode);
    let language = if options.language.eq_ignore_ascii_case("detect")
        || options.language.eq_ignore_ascii_case("auto")
    {
        detect_language(client, audio, filename, mime, &options.base_url)
            .await?
            .0
    } else {
        options.language.clone()
    };
    let primary = transcribe_once(client, audio, filename, mime, options, mode, &language).await?;

    // A routed microphone can contain a few seconds of silence before speech.
    // If a long recording collapses to only a tiny tail fragment, compare the
    // sibling Crisper mode and retain it only when it is substantially fuller.
    // Normal short utterances remain one-pass and unchanged.
    let duration = wav_duration_seconds(audio).unwrap_or_default();
    let words = primary.split_whitespace().count();
    if duration >= 4.0 && words <= 3 {
        let sibling = if mode == "verbatim" {
            "intended"
        } else {
            "verbatim"
        };
        if let Ok(candidate) =
            transcribe_once(client, audio, filename, mime, options, sibling, &language).await
        {
            let candidate_words = candidate.split_whitespace().count();
            if candidate_words >= words.saturating_mul(2).max(words + 3) {
                return Ok(candidate);
            }
        }
    }
    Ok(primary)
}

async fn transcribe_once(
    client: &Client,
    audio: &[u8],
    filename: &str,
    mime: &str,
    options: &CrisperOptions,
    mode: &str,
    language: &str,
) -> Result<String, CrisperError> {
    let file = multipart::Part::bytes(audio.to_vec())
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|error| CrisperError::Multipart(error.to_string()))?;
    let form = multipart::Form::new()
        .part("file", file)
        .text("operation", mode.to_string())
        .text("language", language.to_string())
        .text("word_timestamps", "true")
        .text("strategy", "continuation")
        .text("chunk_duration", options.chunk_duration.to_string())
        .text("stride", options.stride.to_string())
        .text("context_words", options.context_words.to_string())
        .text("max_new_tokens", options.max_new_tokens.to_string())
        .text("hotwords", options.hotwords.clone());
    let response = client
        .post(format!(
            "{}/api/transcribe",
            options.base_url.trim_end_matches('/')
        ))
        .multipart(form)
        .send()
        .await
        .map_err(|error| CrisperError::Request(error.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| CrisperError::Request(error.to_string()))?;
    if !status.is_success() {
        return Err(CrisperError::Service {
            status: status.as_u16(),
            body: body.chars().take(400).collect(),
        });
    }
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| CrisperError::InvalidResponse(error.to_string()))?;
    value
        .get("results")
        .and_then(|results| results.get(mode))
        .and_then(|transcript| transcript.get("text"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CrisperError::EmptyTranscript(mode.to_string()))
}

pub async fn detect_language(
    client: &Client,
    audio: &[u8],
    filename: &str,
    mime: &str,
    base_url: &str,
) -> Result<(String, f32), CrisperError> {
    let file = multipart::Part::bytes(audio.to_vec())
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|error| CrisperError::Multipart(error.to_string()))?;
    let response = client
        .post(format!(
            "{}/api/detect-language",
            base_url.trim_end_matches('/')
        ))
        .multipart(multipart::Form::new().part("file", file))
        .send()
        .await
        .map_err(|error| CrisperError::Request(error.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| CrisperError::Request(error.to_string()))?;
    if !status.is_success() {
        return Err(CrisperError::Service {
            status: status.as_u16(),
            body: body.chars().take(400).collect(),
        });
    }
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| CrisperError::InvalidResponse(error.to_string()))?;
    let language = value
        .get("language")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .ok_or_else(|| CrisperError::InvalidResponse("missing language".into()))?;
    let confidence = value
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default() as f32;
    Ok((language.to_string(), confidence))
}

fn wav_duration_seconds(audio: &[u8]) -> Option<f32> {
    let reader = hound::WavReader::new(std::io::Cursor::new(audio)).ok()?;
    let spec = reader.spec();
    Some(reader.duration() as f32 / spec.sample_rate.max(1) as f32)
}

#[cfg(test)]
mod tests {
    use super::normalized_mode;

    #[test]
    fn normalizes_public_literal_name() {
        assert_eq!(normalized_mode("literal"), "verbatim");
        assert_eq!(normalized_mode("verbatim"), "verbatim");
        assert_eq!(normalized_mode("intended"), "intended");
    }
}
