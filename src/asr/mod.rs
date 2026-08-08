//! ASR engine trait and implementations.
//!
//! # Architecture
//! - [`AsrEngine`] trait: all engines implement this.
//! - [`AsrManager`]: manages engine registry, switching, and fallback.
//! - Modules: `whisper_local`, `mimo_asr`, `openai_asr`, `aliyun_asr`.

pub mod aliyun_asr;
pub mod crisper_whisper;
pub mod doubao_asr;
pub mod mimo_asr;
pub mod openai_asr;
pub mod whisper_cpp;
pub mod whisper_local;

use async_trait::async_trait;

/// Errors from ASR recognition.
#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error("ASR engine '{engine}' failed: {message}")]
    EngineError { engine: String, message: String },

    #[error("No ASR engine available (none registered)")]
    NoEngineAvailable,

    #[error("ASR engine '{name}' not found")]
    EngineNotFound { name: String },

    #[error("Audio format error: {0}")]
    #[allow(dead_code)]
    AudioFormat(String),
}

/// An ASR engine that can transcribe audio to text.
#[async_trait]
pub trait AsrEngine: Send + Sync {
    /// Human-readable name (matches config `primary_engine`).
    fn name(&self) -> &'static str;

    /// Transcribe WAV audio bytes to text.
    async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError>;
}

/// Manages multiple ASR engines with automatic fallback.
///
/// Engines are stored in a `Vec` to preserve registration order (so cycling
/// and tray-menu listing are deterministic, unlike a `HashMap`). The active
/// engine is behind a `RwLock` so the manager can be shared across threads
/// via `Arc` while still allowing the active engine to be switched.
pub struct AsrManager {
    engines: Vec<(String, Box<dyn AsrEngine>)>,
    active: std::sync::RwLock<String>,
    fallback_order: Vec<String>,
}

impl AsrManager {
    /// Create a new manager with no engines registered.
    pub fn new(primary: String, fallback: Vec<String>) -> Self {
        Self {
            engines: Vec::new(),
            active: std::sync::RwLock::new(primary),
            fallback_order: fallback,
        }
    }

    /// Register an engine.
    pub fn register(&mut self, engine: Box<dyn AsrEngine>) {
        let name = engine.name().to_string();
        self.engines.push((name, engine));
    }

    /// Get the name of the currently active engine.
    pub fn active_engine(&self) -> String {
        self.active.read().expect("active lock poisoned").clone()
    }

    /// Switch the active engine by name.
    pub fn set_active(&self, name: &str) -> Result<(), AsrError> {
        if self.engines.iter().any(|(n, _)| n == name) {
            *self.active.write().expect("active lock poisoned") = name.to_string();
            Ok(())
        } else {
            Err(AsrError::EngineNotFound {
                name: name.to_string(),
            })
        }
    }

    /// Cycle to the next available engine. Returns the new active name.
    pub fn cycle_engine(&self) -> Option<String> {
        if self.engines.is_empty() {
            return None;
        }
        let names: Vec<&String> = self.engines.iter().map(|(n, _)| n).collect();
        let current = self.active.read().expect("active lock poisoned").clone();
        let pos = names.iter().position(|n| *n == &current);
        let next_idx = match pos {
            Some(i) => (i + 1) % names.len(),
            None => 0,
        };
        let next = names[next_idx].clone();
        *self.active.write().expect("active lock poisoned") = next.clone();
        Some(next)
    }

    /// Get the list of registered engine names (in registration order).
    pub fn engine_names(&self) -> Vec<String> {
        self.engines.iter().map(|(n, _)| n.clone()).collect()
    }

    /// Transcribe audio using the active engine, with automatic fallback.
    pub async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError> {
        let active = self.active.read().expect("active lock poisoned").clone();

        // Try active engine first
        if let Some((_, engine)) = self.engines.iter().find(|(n, _)| *n == active) {
            match engine.transcribe(audio_wav).await {
                Ok(text) => return Ok(text),
                Err(e) => {
                    log::warn!("Active engine '{}' failed: {}", active, e);
                }
            }
        }

        // Try fallbacks in order
        for name in &self.fallback_order {
            if *name == active {
                continue; // already tried
            }
            if let Some((_, engine)) = self.engines.iter().find(|(n, _)| n == name) {
                match engine.transcribe(audio_wav).await {
                    Ok(text) => {
                        log::info!("Fallback engine '{}' succeeded", name);
                        return Ok(text);
                    }
                    Err(e) => {
                        log::warn!("Fallback engine '{}' failed: {}", name, e);
                    }
                }
            }
        }

        Err(AsrError::NoEngineAvailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockEngine;

    #[async_trait]
    impl AsrEngine for MockEngine {
        fn name(&self) -> &'static str {
            "mock"
        }
        async fn transcribe(&self, _audio: &[u8]) -> Result<String, AsrError> {
            Ok("mock result".to_string())
        }
    }

    struct FailingEngine;

    #[async_trait]
    impl AsrEngine for FailingEngine {
        fn name(&self) -> &'static str {
            "failing"
        }
        async fn transcribe(&self, _audio: &[u8]) -> Result<String, AsrError> {
            Err(AsrError::EngineError {
                engine: "failing".to_string(),
                message: "intentional".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn test_active_engine() {
        let mut mgr = AsrManager::new("mock".into(), vec![]);
        mgr.register(Box::new(MockEngine));
        let result = mgr.transcribe(b"fake").await.unwrap();
        assert_eq!(result, "mock result");
    }

    #[tokio::test]
    async fn test_fallback() {
        let mut mgr = AsrManager::new("failing".into(), vec!["mock".into()]);
        mgr.register(Box::new(FailingEngine));
        mgr.register(Box::new(MockEngine));
        let result = mgr.transcribe(b"fake").await.unwrap();
        assert_eq!(result, "mock result");
    }

    #[tokio::test]
    async fn test_all_fail() {
        let mut mgr = AsrManager::new("failing".into(), vec![]);
        mgr.register(Box::new(FailingEngine));
        let result = mgr.transcribe(b"fake").await;
        assert!(result.is_err());
    }

    #[test]
    fn test_cycle_engine() {
        let mut mgr = AsrManager::new("mock".into(), vec![]);
        mgr.register(Box::new(MockEngine));
        assert_eq!(mgr.active_engine(), "mock");
        mgr.cycle_engine();
        assert_eq!(mgr.active_engine(), "mock"); // only one engine
    }

    #[test]
    fn test_engine_order_preserved() {
        // Registration order should be reflected in engine_names() and cycling.
        let mut mgr = AsrManager::new("mock".into(), vec![]);
        mgr.register(Box::new(MockEngine)); // "mock"
        mgr.register(Box::new(FailingEngine)); // "failing"
        assert_eq!(
            mgr.engine_names(),
            vec!["mock".to_string(), "failing".to_string()]
        );
        assert_eq!(mgr.cycle_engine(), Some("failing".to_string()));
        assert_eq!(mgr.cycle_engine(), Some("mock".to_string()));
    }
}
