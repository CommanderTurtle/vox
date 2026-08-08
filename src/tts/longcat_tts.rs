//! Desktop LongCat adapter over the shared UI-free local backend core.

use std::path::PathBuf;

use async_trait::async_trait;
use vox_local_core::longcat::{synthesize as synthesize_longcat, LongCatOptions, LongCatReference};

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

    async fn reference(&self) -> Result<Option<LongCatReference>, TtsError> {
        let (audio_path, transcript) = if let Some(profile) = self.config.active_voice() {
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

        if audio_path.as_os_str().is_empty() {
            return Ok(None);
        }
        if transcript.trim().is_empty() {
            return Err(Self::error(
                "the reference transcript is empty for the selected LongCat voice",
            ));
        }
        let audio = tokio::fs::read(&audio_path).await.map_err(|error| {
            Self::error(format!(
                "could not read reference voice {}: {error}",
                audio_path.display()
            ))
        })?;
        let filename = audio_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("reference.wav")
            .to_string();
        Ok(Some(LongCatReference {
            audio,
            filename,
            transcript,
        }))
    }
}

#[async_trait]
impl TtsEngine for LongCatTtsEngine {
    fn name(&self) -> &'static str {
        "longcat"
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let reference = self.reference().await?;
        let options = LongCatOptions {
            base_url: self.config.base_url.clone(),
            steps: self.config.steps,
            guidance_strength: self.config.guidance_strength,
            guidance_method: self.config.guidance_method.clone(),
            seed: self.config.seed,
            duration_scale: self.config.duration_scale,
            concatenate: self.config.concatenate_chunks,
            characters_per_request: self.config.characters_per_request,
        };
        synthesize_longcat(&self.client, text, &options, reference.as_ref())
            .await
            .map_err(|error| Self::error(error.to_string()))
    }
}

/// File-dialog and pasted paths may carry one pair of wrapping quotes.
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
