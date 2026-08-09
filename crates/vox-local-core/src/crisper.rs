use reqwest::{multipart, Client};

#[derive(Debug, Clone)]
pub struct CrisperOptions {
    pub base_url: String,
    pub mitm_url: String,
    pub mitm_api_key: String,
    pub mode: String,
    pub language: String,
    pub candidate_max_new_tokens: u32,
    pub chunk_duration: f32,
    pub stride: f32,
    pub context_words: u32,
    pub max_new_tokens: u32,
    pub hotwords: String,
}

#[derive(Debug, Clone)]
pub struct CrisperTranscript {
    pub text: String,
    pub language: Option<String>,
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
    Ok(transcribe_tagged(client, audio, filename, mime, options)
        .await?
        .text)
}

pub async fn transcribe_tagged(
    client: &Client,
    audio: &[u8],
    filename: &str,
    mime: &str,
    options: &CrisperOptions,
) -> Result<CrisperTranscript, CrisperError> {
    let mode = normalized_mode(&options.mode);
    let language = if options.language.eq_ignore_ascii_case("detect")
        || options.language.eq_ignore_ascii_case("auto")
    {
        detect_with_candidates(client, audio, filename, mime, options, mode).await?
    } else {
        options.language.clone()
    };
    let text = transcribe_once(client, audio, filename, mime, options, mode, &language).await?;
    Ok(CrisperTranscript {
        text,
        language: Some(language),
    })
}

async fn detect_with_candidates(
    client: &Client,
    audio: &[u8],
    filename: &str,
    mime: &str,
    options: &CrisperOptions,
    mode: &str,
) -> Result<String, CrisperError> {
    let file = multipart::Part::bytes(audio.to_vec())
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|error| CrisperError::Multipart(error.to_string()))?;
    let form = multipart::Form::new()
        .part("file", file)
        .text("operation", mode.to_string())
        .text(
            "max_new_tokens",
            options.candidate_max_new_tokens.clamp(1, 128).to_string(),
        );
    let response = client
        .post(format!(
            "{}/api/transcribe-candidates",
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
    let candidates: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| CrisperError::InvalidResponse(error.to_string()))?;
    let rows = candidates
        .get("candidates")
        .cloned()
        .ok_or_else(|| CrisperError::InvalidResponse("missing candidates".into()))?;
    let mut request = client
        .post(format!(
            "{}/arbitrate",
            options.mitm_url.trim_end_matches('/')
        ))
        .json(&serde_json::json!({"candidates": rows}));
    if !options.mitm_api_key.is_empty() {
        request = request.bearer_auth(&options.mitm_api_key);
    }
    let response = request
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
        .get("language")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CrisperError::InvalidResponse("arbiter returned no language".into()))
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
