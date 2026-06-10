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

impl AppState {
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            AppState::Idle => "Idle",
            AppState::Recording => "Recording",
            AppState::Transcribing => "Transcribing",
        }
    }
}
