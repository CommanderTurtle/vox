//! Configuration system for vox.
//!
//! Config is loaded from a TOML file.
//! On first run, defaults are written automatically.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

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
    #[serde(default)]
    pub translate: TranslateConfig,
    #[serde(default)]
    pub subtitles: SubtitleConfig,
    pub general: GeneralConfig,
}

/// Local system-audio caption surface. Audio is supplied by the native
/// router's isolated WASAPI loopback tap, then sent to the already configured
/// CrisperWhisper and optional translation services.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubtitleConfig {
    #[serde(default = "default_mic_forwarder_url")]
    pub router_url: String,
    #[serde(default = "default_subtitle_chunk_seconds")]
    pub chunk_seconds: f32,
    #[serde(default = "default_subtitle_font_size")]
    pub font_size: f32,
    #[serde(default = "default_subtitle_max_lines")]
    pub max_lines: usize,
}

fn default_subtitle_chunk_seconds() -> f32 {
    1.5
}
fn default_subtitle_font_size() -> f32 {
    30.0
}
fn default_subtitle_max_lines() -> usize {
    3
}

impl Default for SubtitleConfig {
    fn default() -> Self {
        Self {
            router_url: default_mic_forwarder_url(),
            chunk_seconds: default_subtitle_chunk_seconds(),
            font_size: default_subtitle_font_size(),
            max_lines: default_subtitle_max_lines(),
        }
    }
}

/// Hotkey bindings (stored as human-readable strings like "Alt+`").
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotkeyConfig {
    pub record_toggle: String,
    pub engine_switch: String,
    pub inject_mode_switch: String,
    #[serde(default)]
    pub tts_trigger: String,
    #[serde(default = "default_tts_voice_switch")]
    pub tts_voice_switch: String,
    #[serde(default = "default_tts_seed_increment")]
    pub tts_seed_increment: String,
    #[serde(default = "default_tts_seed_decrement")]
    pub tts_seed_decrement: String,
    #[serde(default = "default_translate_text")]
    pub translate_text: String,
    #[serde(default = "default_translate_tts")]
    pub translate_tts: String,
    #[serde(default = "default_record_translate_tts")]
    pub record_translate_tts: String,
    #[serde(default = "default_record_translate_text")]
    pub record_translate_text: String,
    #[serde(default = "default_record_tts")]
    pub record_tts: String,
    #[serde(default = "default_translate_route_switch")]
    pub translate_route_switch: String,
}

fn default_tts_voice_switch() -> String {
    "Alt+Shift+T".to_string()
}
fn default_tts_seed_increment() -> String {
    "Alt+Shift+S".to_string()
}
fn default_tts_seed_decrement() -> String {
    "Alt+Shift+A".to_string()
}
fn default_translate_text() -> String {
    "Alt+R".to_string()
}
fn default_translate_tts() -> String {
    "Alt+Shift+R".to_string()
}
fn default_record_translate_tts() -> String {
    "Alt+Shift+`".to_string()
}
fn default_record_translate_text() -> String {
    "Alt+Ctrl+R".to_string()
}
fn default_record_tts() -> String {
    "Alt+Ctrl+`".to_string()
}
fn default_translate_route_switch() -> String {
    "Alt+Shift+L".to_string()
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
    #[serde(default)]
    pub crisper: CrisperConfig,
}

/// CrisperWhisper 2.0 local HTTP service with selectable intended/non-literal
/// or literal transcription.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrisperConfig {
    #[serde(default = "default_crisper_base_url")]
    pub base_url: String,
    #[serde(default = "default_crisper_mode")]
    pub mode: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_crisper_chunk_duration")]
    pub chunk_duration: f32,
    #[serde(default = "default_crisper_stride")]
    pub stride: f32,
    #[serde(default = "default_crisper_context_words")]
    pub context_words: u32,
    #[serde(default = "default_crisper_max_new_tokens")]
    pub max_new_tokens: u32,
    #[serde(default)]
    pub hotwords: String,
}

fn default_crisper_base_url() -> String {
    "http://127.0.0.1:8172".to_string()
}
fn default_crisper_mode() -> String {
    "intended".to_string()
}
fn default_language() -> String {
    "en".to_string()
}
fn default_crisper_chunk_duration() -> f32 {
    30.0
}
fn default_crisper_stride() -> f32 {
    26.0
}
fn default_crisper_context_words() -> u32 {
    12
}
fn default_crisper_max_new_tokens() -> u32 {
    256
}

impl Default for CrisperConfig {
    fn default() -> Self {
        Self {
            base_url: default_crisper_base_url(),
            mode: default_crisper_mode(),
            language: default_language(),
            chunk_duration: default_crisper_chunk_duration(),
            stride: default_crisper_stride(),
            context_words: default_crisper_context_words(),
            max_new_tokens: default_crisper_max_new_tokens(),
            hotwords: String::new(),
        }
    }
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
/// The `api_key` is shared between the ASR and TTS engines; it is the Agent
/// Plan subscription key, used against the `/api/v3/plan/...` endpoints.
/// When non-empty, both engines register and become the default primary engine.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DoubaoConfig {
    /// Agent Plan subscription API key (shared by ASR + TTS). Empty = engines skipped.
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
    pub output: TtsOutputConfig,
    #[serde(default)]
    pub mimo: MimoTtsConfig,
    #[serde(default)]
    pub edge: EdgeTtsConfig,
    #[serde(default)]
    pub doubao: DoubaoTtsConfig,
    #[serde(default)]
    pub longcat: LongCatTtsConfig,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            primary_engine: "edge-tts".to_string(),
            input_mode: "selection".to_string(),
            output: TtsOutputConfig::default(),
            mimo: MimoTtsConfig::default(),
            edge: EdgeTtsConfig::default(),
            doubao: DoubaoTtsConfig::default(),
            longcat: LongCatTtsConfig::default(),
        }
    }
}

/// Destination for synthesized audio. Playback preserves Vox's original
/// behavior; clipboard_wav publishes a pasteable WAV file; mic_forwarder
/// sends WAV to the separately built local audio router.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsOutputConfig {
    #[serde(default = "default_tts_output_mode")]
    pub mode: String,
    #[serde(default = "default_mic_forwarder_url")]
    pub mic_forwarder_url: String,
}

fn default_tts_output_mode() -> String {
    "playback".to_string()
}

fn default_mic_forwarder_url() -> String {
    "http://127.0.0.1:8182".to_string()
}

impl Default for TtsOutputConfig {
    fn default() -> Self {
        Self {
            mode: default_tts_output_mode(),
            mic_forwarder_url: default_mic_forwarder_url(),
        }
    }
}

/// LongCat local voice-cloning service. Reference audio and its verbatim
/// transcript stay client-side and are uploaded only to the configured host.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LongCatTtsConfig {
    #[serde(default = "default_longcat_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub prompt_audio_path: String,
    #[serde(default)]
    pub prompt_text: String,
    /// Named reference-audio/transcript pairs. When non-empty, the selected
    /// profile replaces the legacy prompt_audio_path/prompt_text pair above.
    #[serde(default)]
    pub voice_profiles: Vec<LongCatVoiceProfile>,
    #[serde(default)]
    pub active_voice_profile: String,
    #[serde(default = "default_longcat_steps")]
    pub steps: u32,
    #[serde(default = "default_longcat_guidance_strength")]
    pub guidance_strength: f32,
    #[serde(default = "default_longcat_guidance_method")]
    pub guidance_method: String,
    #[serde(default = "default_longcat_seed")]
    pub seed: u64,
    #[serde(default = "default_longcat_duration_scale")]
    pub duration_scale: f32,
    /// Split long synthesis into bounded requests and concatenate their WAVs.
    /// Disabling this sends the complete text as one LongCat request.
    #[serde(default = "default_true")]
    pub concatenate_chunks: bool,
    /// Character ceiling used to locate the previous sentence-ending
    /// punctuation mark when concatenation is enabled.
    #[serde(default = "default_longcat_characters_per_request")]
    pub characters_per_request: usize,
}

/// One reusable LongCat voice-conditioning pair. The transcript is kept as a
/// separate UTF-8 text file so reference audio and its verbatim words travel
/// together without copying a large paragraph into config.toml.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LongCatVoiceProfile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub audio_path: String,
    #[serde(default)]
    pub transcript_path: String,
}

impl LongCatTtsConfig {
    pub fn active_voice(&self) -> Option<&LongCatVoiceProfile> {
        if self.voice_profiles.is_empty() {
            return None;
        }
        self.voice_profiles
            .iter()
            .find(|profile| profile.name == self.active_voice_profile)
            .or_else(|| self.voice_profiles.first())
    }

    pub fn active_voice_name(&self) -> String {
        self.active_voice()
            .map(|profile| profile.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Legacy pair".to_string())
    }

    pub fn cycle_voice_profile(&mut self) -> Option<String> {
        if self.voice_profiles.is_empty() {
            return None;
        }
        let current = self
            .voice_profiles
            .iter()
            .position(|profile| profile.name == self.active_voice_profile)
            .unwrap_or(0);
        let next = (current + 1) % self.voice_profiles.len();
        let name = self.voice_profiles[next].name.clone();
        self.active_voice_profile = name.clone();
        Some(name)
    }
}

fn default_longcat_base_url() -> String {
    "http://127.0.0.1:8230".to_string()
}
fn default_longcat_steps() -> u32 {
    16
}
fn default_longcat_guidance_strength() -> f32 {
    4.0
}
fn default_longcat_guidance_method() -> String {
    "apg".to_string()
}
fn default_longcat_seed() -> u64 {
    1024
}
fn default_longcat_duration_scale() -> f32 {
    1.0
}
fn default_longcat_characters_per_request() -> usize {
    240
}

impl Default for LongCatTtsConfig {
    fn default() -> Self {
        Self {
            base_url: default_longcat_base_url(),
            prompt_audio_path: String::new(),
            prompt_text: String::new(),
            voice_profiles: Vec::new(),
            active_voice_profile: String::new(),
            steps: default_longcat_steps(),
            guidance_strength: default_longcat_guidance_strength(),
            guidance_method: default_longcat_guidance_method(),
            seed: default_longcat_seed(),
            duration_scale: default_longcat_duration_scale(),
            concatenate_chunks: true,
            characters_per_request: default_longcat_characters_per_request(),
        }
    }
}

/// Optional translation stage backed by the standalone local service or
/// another explicitly configured OpenAI-compatible endpoint. It can
/// independently transform ASR output and TTS input.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TranslateConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub asr: bool,
    #[serde(default)]
    pub tts: bool,
    #[serde(default = "default_translate_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_translate_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_source_language")]
    pub source_language: String,
    #[serde(default = "default_target_language")]
    pub target_language: String,
    #[serde(default = "default_translate_active_route")]
    pub active_route: String,
    #[serde(default = "default_inbound_route")]
    pub inbound: TranslateRouteConfig,
    #[serde(default = "default_outbound_route")]
    pub outbound: TranslateRouteConfig,
    #[serde(default = "default_translate_system_prompt")]
    pub system_prompt: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TranslateRouteConfig {
    #[serde(default = "default_source_language")]
    pub source_language: String,
    #[serde(default = "default_target_language")]
    pub target_language: String,
}

impl TranslateConfig {
    pub fn active_route(&self) -> (&str, &TranslateRouteConfig) {
        if self.active_route.eq_ignore_ascii_case("outbound") {
            ("outbound", &self.outbound)
        } else {
            ("inbound", &self.inbound)
        }
    }

    pub fn cycle_route(&mut self) -> String {
        self.active_route = if self.active_route.eq_ignore_ascii_case("outbound") {
            "inbound".to_string()
        } else {
            "outbound".to_string()
        };
        self.active_route.clone()
    }
}

fn default_true() -> bool {
    true
}
fn default_translate_base_url() -> String {
    "http://127.0.0.1:8176/v1".to_string()
}
fn default_source_language() -> String {
    "auto".to_string()
}
fn default_target_language() -> String {
    "English".to_string()
}
fn default_translate_max_tokens() -> u32 {
    256
}
fn default_translate_active_route() -> String {
    "inbound".to_string()
}
fn default_inbound_route() -> TranslateRouteConfig {
    TranslateRouteConfig {
        source_language: "auto".to_string(),
        target_language: "English".to_string(),
    }
}
fn default_outbound_route() -> TranslateRouteConfig {
    TranslateRouteConfig {
        source_language: "English".to_string(),
        target_language: "Spanish".to_string(),
    }
}
fn default_translate_system_prompt() -> String {
    "You are an expert cross-lingual translator. Identify the source language and translate it naturally into the requested target language without losing meaning or nuance. Return only the translation: no labels, commentary, or explanation. Preserve names, numbers, formatting, profanity, and tone.".to_string()
}

impl Default for TranslateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            asr: true,
            tts: false,
            base_url: default_translate_base_url(),
            api_key: String::new(),
            model: String::new(),
            max_tokens: default_translate_max_tokens(),
            source_language: default_source_language(),
            target_language: default_target_language(),
            active_route: default_translate_active_route(),
            inbound: default_inbound_route(),
            outbound: default_outbound_route(),
            system_prompt: default_translate_system_prompt(),
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
    #[serde(default = "default_restore_clipboard")]
    pub restore_clipboard: bool,
    #[serde(default)]
    pub copy_only: bool,
}

fn default_restore_clipboard() -> bool {
    true
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
    /// Keep configuration beside the executable so a Vox build and its
    /// settings remain one portable unit.
    pub fn default_config_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("config.toml")))
            .unwrap_or_else(|| PathBuf::from("config.toml"))
    }

    /// Load config from beside the running executable.
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
