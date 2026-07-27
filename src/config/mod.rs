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
    #[serde(rename = "whisper_local", default)]
    pub whisper_local: WhisperLocalConfig,
    #[serde(default)]
    pub whisper_cpp: WhisperCppConfig,
    #[serde(default)]
    pub mimo: MimoConfig,
    #[serde(default)]
    pub aliyun: AliyunConfig,
    #[serde(default)]
    pub openai: OpenaiConfig,
    #[serde(default)]
    pub doubao: DoubaoConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhisperLocalConfig {
    pub model: String,
    pub model_path: String,
}

impl Default for WhisperLocalConfig {
    fn default() -> Self {
        Self {
            model: "base".to_string(),
            model_path: String::new(),
        }
    }
}

/// Local ASR via the whisper.cpp HTTP server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhisperCppConfig {
    pub base_url: String,
}

impl Default for WhisperCppConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8080".to_string(),
        }
    }
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AliyunConfig {
    #[serde(default)]
    pub appkey: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenaiConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
}

impl Default for OpenaiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "whisper-1".to_string(),
        }
    }
}

/// Volcano Engine Doubao voice models config (shared by ASR + TTS).
///
/// The `api_key` is shared between the ASR and TTS engines; it must be issued
/// from the **Doubao Speech console** (not the Ark console - Ark keys return
/// HTTP 401 against the speech endpoints). When non-empty, both engines
/// register and become the default primary engine.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DoubaoConfig {
    /// Doubao Speech API key (shared by ASR + TTS). Empty = engines skipped.
    #[serde(default)]
    pub api_key: String,
}

/// Doubao TTS (seed-tts-2.0) tuning parameters.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DoubaoTtsConfig {
    /// Speaker ID, e.g. `zh_female_vv_uranus_bigtts`.
    #[serde(default = "default_doubao_speaker")]
    pub speaker: String,
    /// Speech rate, [-50, 100]; 0 = normal, 100 = 2x, -50 = 0.5x.
    #[serde(default)]
    pub speech_rate: i32,
    /// Loudness rate, [-50, 100]; 0 = normal.
    #[serde(default)]
    pub loudness_rate: i32,
    /// PCM sample rate, default 24000.
    #[serde(default = "default_doubao_sample_rate")]
    pub sample_rate: u32,
}

fn default_doubao_speaker() -> String {
    "zh_female_vv_uranus_bigtts".to_string()
}
fn default_doubao_sample_rate() -> u32 {
    24000
}

impl Default for DoubaoTtsConfig {
    fn default() -> Self {
        Self {
            speaker: default_doubao_speaker(),
            speech_rate: 0,
            loudness_rate: 0,
            sample_rate: default_doubao_sample_rate(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsConfig {
    pub primary_engine: String,
    #[serde(default)]
    pub input_mode: String,
    #[serde(default)]
    pub mimo: MimoTtsConfig,
    #[serde(default)]
    pub edge: EdgeTtsConfig,
    #[serde(default)]
    pub doubao: DoubaoTtsConfig,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            primary_engine: "edge-tts".to_string(),
            input_mode: "selection".to_string(),
            mimo: MimoTtsConfig::default(),
            edge: EdgeTtsConfig::default(),
            doubao: DoubaoTtsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MimoTtsConfig {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub voice: String,
    #[serde(default)]
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

/// Microsoft Edge TTS (free, no API key).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EdgeTtsConfig {
    /// Voice name, e.g. `zh-CN-XiaoxiaoNeural`, `en-US-AriaNeural`.
    #[serde(default = "default_edge_voice")]
    pub voice: String,
    /// Speech rate, e.g. `+0%`, `+10%`.
    #[serde(default = "default_edge_zero_pct")]
    pub rate: String,
    /// Speech volume, e.g. `+0%`.
    #[serde(default = "default_edge_zero_pct")]
    pub volume: String,
    /// Speech pitch, e.g. `+0Hz`.
    #[serde(default = "default_edge_zero_hz")]
    pub pitch: String,
}

fn default_edge_voice() -> String {
    "zh-CN-XiaoxiaoNeural".to_string()
}
fn default_edge_zero_pct() -> String {
    "+0%".to_string()
}
fn default_edge_zero_hz() -> String {
    "+0Hz".to_string()
}

impl Default for EdgeTtsConfig {
    fn default() -> Self {
        Self {
            voice: default_edge_voice(),
            rate: default_edge_zero_pct(),
            volume: default_edge_zero_pct(),
            pitch: default_edge_zero_hz(),
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
    /// Voice input mode: `"ptt"` (push-to-talk: hold to record, release to
    /// stop) or `"toggle"` (press to start, press again to stop).
    #[serde(default = "default_record_mode")]
    pub record_mode: String,
}

fn default_record_mode() -> String {
    "ptt".to_string()
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
        assert_eq!(config.asr.primary_engine, "whisper-cpp");
        assert_eq!(config.inject.mode, "keyboard");
        assert_eq!(config.hotkey.tts_trigger, "Alt+T");
        assert_eq!(config.tts.primary_engine, "edge-tts");
        assert_eq!(config.tts.edge.voice, "zh-CN-XiaoxiaoNeural");
        assert_eq!(config.asr.whisper_cpp.base_url, "http://127.0.0.1:8080");
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
