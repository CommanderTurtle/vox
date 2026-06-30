//! Configuration system for vox.
//!
//! Config is loaded from a TOML file.
//! On first run, defaults are written automatically.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub asr: AsrConfig,
    pub inject: InjectConfig,
    #[serde(default)]
    pub tts: TtsConfig,
    pub general: GeneralConfig,
}

/// Hotkey bindings (stored as human-readable strings like "Alt+`").
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotkeyConfig {
    pub record_toggle: String,
    pub engine_switch: String,
    pub inject_mode_switch: String,
    #[serde(default)]
    pub tts_trigger: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AsrConfig {
    pub primary_engine: String,
    pub fallback_engines: Vec<String>,
    #[serde(rename = "whisper_local")]
    pub whisper_local: WhisperLocalConfig,
    #[serde(default)]
    pub mimo: MimoConfig,
    pub aliyun: AliyunConfig,
    pub openai: OpenaiConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhisperLocalConfig {
    pub model: String,
    pub model_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MimoConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl Default for MimoConfig {
    fn default() -> Self {
        Self {
            base_url: "https://token-plan-cn.xiaomimimo.com/v1".to_string(),
            api_key: String::new(),
            model: "mimo-v2.5-asr".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AliyunConfig {
    pub appkey: String,
    pub token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenaiConfig {
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsConfig {
    pub primary_engine: String,
    pub input_mode: String,
    pub mimo: MimoTtsConfig,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            primary_engine: "mimo-tts".to_string(),
            input_mode: "selection".to_string(),
            mimo: MimoTtsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MimoTtsConfig {
    pub model: String,
    pub voice: String,
    pub speed: f64,
}

impl Default for MimoTtsConfig {
    fn default() -> Self {
        Self {
            model: "mimo-v2.5-tts".to_string(),
            voice: "default".to_string(),
            speed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InjectConfig {
    pub mode: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GeneralConfig {
    pub autostart: bool,
    pub language: String,
}

impl Default for Config {
    fn default() -> Self {
        // Embed the defaults.toml at compile time
        let raw = include_str!("defaults.toml");
        toml::from_str(raw).expect("defaults.toml should be valid TOML")
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Manages loading, saving, and watching the config file.
pub struct ConfigManager {
    /// Absolute path to the TOML config file.
    path: PathBuf,
    /// In-memory config, protected by a read-write lock.
    config: Arc<RwLock<Config>>,
}

impl ConfigManager {
    /// Return the canonical project directory for config files.
    pub fn project_dirs() -> Option<ProjectDirs> {
        ProjectDirs::from("com", "vox", "vox")
    }

    /// Return the expected config file path.
    pub fn default_config_path() -> PathBuf {
        let dirs = Self::project_dirs().expect("could not determine config directory");
        let config_dir = dirs.config_dir();
        config_dir.join("config.toml")
    }

    /// Load config from the default platform path.
    /// If the file doesn't exist, create it with defaults.
    pub fn load_or_create() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::default_config_path();
        Self::load_from(&path)
    }

    /// Load config from an explicit path.
    /// If the file doesn't exist, write defaults to it.
    pub fn load_from(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let config = if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            toml::from_str(&raw)?
        } else {
            // Create parent directory and write defaults
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let defaults = Config::default();
            let raw = toml::to_string_pretty(&defaults)?;
            std::fs::write(path, &raw)?;
            log::info!("Created default config at {:?}", path);
            defaults
        };

        Ok(Self {
            path: path.clone(),
            config: Arc::new(RwLock::new(config)),
        })
    }

    /// Save the current in-memory config to disk.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config = self.config.read().map_err(|e| e.to_string())?;
        let raw = toml::to_string_pretty(&*config)?;
        std::fs::write(&self.path, &raw)?;
        Ok(())
    }

    /// Reload the config from disk, replacing the in-memory copy.
    ///
    /// Used after the settings window writes a new config so the running
    /// app picks up changes (API keys, engine, modes) without a restart.
    pub fn reload_from_disk(&self) -> Result<(), Box<dyn std::error::Error>> {
        let raw = std::fs::read_to_string(&self.path)?;
        let config: Config = toml::from_str(&raw)?;
        let mut guard = self.config.write().map_err(|e| e.to_string())?;
        *guard = config;
        Ok(())
    }

    /// Read-only access to the config.
    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().expect("config lock poisoned")
    }

    /// Write access to the config (caller must call `save()` afterwards).
    pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, Config> {
        self.config.write().expect("config lock poisoned")
    }

    /// Get the config path for display purposes.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Get the shared Arc for passing across threads.
    #[allow(dead_code)]
    pub fn shared_config(&self) -> Arc<RwLock<Config>> {
        self.config.clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config_is_valid() {
        let config = Config::default();
        assert_eq!(config.hotkey.record_toggle, "Alt+`");
        assert_eq!(config.asr.primary_engine, "mimo");
        assert_eq!(config.inject.mode, "keyboard");
        assert_eq!(config.hotkey.tts_trigger, "Alt+T");
        assert_eq!(config.tts.primary_engine, "mimo-tts");
    }

    #[test]
    fn test_roundtrip_config() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");

        // Load (creates with defaults) and then modify
        let mgr = ConfigManager::load_from(&config_path).unwrap();
        {
            let mut c = mgr.write();
            c.inject.mode = "clipboard".to_string();
        }
        mgr.save().unwrap();

        // Reload and check
        let mgr2 = ConfigManager::load_from(&config_path).unwrap();
        assert_eq!(mgr2.read().inject.mode, "clipboard");
    }
}
