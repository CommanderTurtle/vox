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
