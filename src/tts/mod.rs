//! Text-to-Speech engine trait and manager.
//!
//! # Architecture
//! - [`TtsEngine`] trait: all TTS engines implement this.
//! - [`TtsManager`]: manages engine registry and switching.

pub mod mimo_tts;
pub mod edge_tts;
pub mod doubao_tts;
pub mod playback;

use async_trait::async_trait;

use crate::config::Config;

/// How TTS obtains the text to speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsInputMode {
    /// Simulate Ctrl+C to copy selected text, then read clipboard.
    Selection,
    /// Directly read current clipboard content.
    Clipboard,
}

impl TtsInputMode {
    pub fn from_config(cfg: &Config) -> Self {
        match cfg.tts.input_mode.as_str() {
            "clipboard" => TtsInputMode::Clipboard,
            _ => TtsInputMode::Selection,
        }
    }

    #[allow(dead_code)]
    pub fn toggle(&self) -> Self {
        match self {
            TtsInputMode::Selection => TtsInputMode::Clipboard,
            TtsInputMode::Clipboard => TtsInputMode::Selection,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TtsInputMode::Selection => "selection",
            TtsInputMode::Clipboard => "clipboard",
        }
    }

    #[allow(dead_code)]
    pub fn display_name(&self) -> &'static str {
        match self {
            TtsInputMode::Selection => "Selection (Ctrl+C)",
            TtsInputMode::Clipboard => "Clipboard",
        }
    }
}

/// Errors from TTS synthesis.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("TTS engine '{engine}' failed: {message}")]
    EngineError { engine: String, message: String },

    #[error("No TTS engine available")]
    NoEngineAvailable,

    #[error("TTS engine '{name}' not found")]
    EngineNotFound { name: String },

    #[error("Audio playback failed: {0}")]
    Playback(String),
}

/// A TTS engine that can synthesize text to audio.
#[async_trait]
pub trait TtsEngine: Send + Sync {
    /// Human-readable name.
    fn name(&self) -> &'static str;

    /// Synthesize text to audio bytes (container per `output_format`).
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError>;

    /// Container format of the bytes returned by `synthesize`. Defaults to
    /// WAV; engines returning MP3 override this.
    fn output_format(&self) -> playback::AudioFormat {
        playback::AudioFormat::Wav
    }
}

/// Manages multiple TTS engines.
///
/// Engines are stored in a `Vec` to preserve registration order (so the tray
/// menu lists engines deterministically). The active engine is behind a
/// `RwLock` so the manager can be shared across threads via `Arc`.
pub struct TtsManager {
    engines: Vec<(String, Box<dyn TtsEngine>)>,
    active: std::sync::RwLock<String>,
}

impl TtsManager {
    pub fn new(primary: String) -> Self {
        Self {
            engines: Vec::new(),
            active: std::sync::RwLock::new(primary),
        }
    }

    pub fn register(&mut self, engine: Box<dyn TtsEngine>) {
        let name = engine.name().to_string();
        self.engines.push((name, engine));
    }

    pub fn active_engine(&self) -> String {
        self.active.read().expect("active lock poisoned").clone()
    }

    pub fn set_active(&self, name: &str) -> Result<(), TtsError> {
        if self.engines.iter().any(|(n, _)| n == name) {
            *self.active.write().expect("active lock poisoned") = name.to_string();
            Ok(())
        } else {
            Err(TtsError::EngineNotFound { name: name.to_string() })
        }
    }

    #[allow(dead_code)]
    pub fn cycle_engine(&self) -> Option<String> {
        if self.engines.is_empty() {
            return None;
        }
        let names: Vec<&String> = self.engines.iter().map(|(n, _)| n).collect();
        let current = self.active.read().expect("active lock poisoned").clone();
        let pos = names.iter().position(|n| *n == &current);
        let next = match pos {
            Some(i) => (i + 1) % names.len(),
            None => 0,
        };
        let name = names[next].clone();
        *self.active.write().expect("active lock poisoned") = name.clone();
        Some(name)
    }

    pub fn engine_names(&self) -> Vec<String> {
        self.engines.iter().map(|(n, _)| n.clone()).collect()
    }

    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let active = self.active.read().expect("active lock poisoned").clone();
        if let Some((_, engine)) = self.engines.iter().find(|(n, _)| *n == active) {
            return engine.synthesize(text).await;
        }
        Err(TtsError::NoEngineAvailable)
    }

    /// Container format produced by the active engine.
    pub fn output_format(&self) -> playback::AudioFormat {
        let active = self.active.read().expect("active lock poisoned").clone();
        if let Some((_, engine)) = self.engines.iter().find(|(n, _)| *n == active) {
            return engine.output_format();
        }
        playback::AudioFormat::Wav
    }
}
