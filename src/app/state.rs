//! Application state machine for vox.

/// The lifecycle state of the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    /// Idle — waiting for recording hotkey
    Idle,
    /// Currently capturing audio from microphone
    Recording,
    /// Audio captured, ASR engine is processing
    Transcribing,
}

/// How the record hotkey behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordMode {
    /// Push-to-talk: hold the hotkey to record, release to stop.
    PushToTalk,
    /// Toggle: press to start, press again to stop.
    Toggle,
}

impl RecordMode {
    /// Parse from a config string. Unknown values fall back to PushToTalk.
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "toggle" => RecordMode::Toggle,
            _ => RecordMode::PushToTalk,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RecordMode::PushToTalk => "ptt",
            RecordMode::Toggle => "toggle",
        }
    }

    #[allow(dead_code)]
    pub fn display_name(&self) -> &'static str {
        match self {
            RecordMode::PushToTalk => "Push-to-Talk (hold)",
            RecordMode::Toggle => "Toggle (press)",
        }
    }
}
