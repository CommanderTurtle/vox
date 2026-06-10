//! Text-to-Speech engine trait and manager.
//!
//! # Architecture
//! - [`TtsEngine`] trait: all TTS engines implement this.
//! - [`TtsManager`]: manages engine registry and switching.

pub mod mimo_tts;
pub mod playback;

use std::collections::HashMap;

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

    #[allow(dead_code)]
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

    /// Synthesize text to audio bytes (WAV format).
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError>;
}

/// Manages multiple TTS engines.
pub struct TtsManager {
    engines: HashMap<String, Box<dyn TtsEngine>>,
    active: String,
}

impl TtsManager {
    pub fn new(primary: String) -> Self {
        Self {
            engines: HashMap::new(),
            active: primary,
        }
    }

    pub fn register(&mut self, engine: Box<dyn TtsEngine>) {
        let name = engine.name().to_string();
        self.engines.insert(name, engine);
    }

    pub fn active_engine(&self) -> &str {
        &self.active
    }

    pub fn set_active(&mut self, name: &str) -> Result<(), TtsError> {
        if self.engines.contains_key(name) {
            self.active = name.to_string();
            Ok(())
        } else {
            Err(TtsError::EngineNotFound { name: name.to_string() })
        }
    }

    #[allow(dead_code)]
    pub fn cycle_engine(&mut self) -> Option<&str> {
        let names: Vec<&String> = self.engines.keys().collect();
        if names.is_empty() { return None; }
        let pos = names.iter().position(|n| *n == &self.active);
        let next = match pos {
            Some(i) => (i + 1) % names.len(),
            None => 0,
        };
        self.active = names[next].clone();
        Some(self.active.as_str())
    }

    pub fn engine_names(&self) -> Vec<String> {
        self.engines.keys().cloned().collect()
    }

    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        if let Some(engine) = self.engines.get(&self.active) {
            return engine.synthesize(text).await;
        }
        Err(TtsError::NoEngineAvailable)
    }
}
