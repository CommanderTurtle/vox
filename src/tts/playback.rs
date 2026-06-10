//! Audio playback — uses OS native playback commands.
//!
//! Writes WAV data to a temp file and plays it with the system default
//! audio player (powershell on Windows, afplay on macOS, aplay on Linux).
//! This avoids complex audio library compilation issues.

use std::process::Command;
use std::io::Write;
use tempfile::NamedTempFile;

use crate::tts::TtsError;

/// Play WAV audio bytes. Blocks until playback finishes.
pub fn play_wav(wav_data: &[u8]) -> Result<(), TtsError> {
    // Write WAV data to a temp file
    let mut tmp = NamedTempFile::with_suffix(".wav")
        .map_err(|e| TtsError::Playback(format!("Failed to create temp file: {}", e)))?;

    tmp.write_all(wav_data)
        .map_err(|e| TtsError::Playback(format!("Failed to write temp file: {}", e)))?;

    let path = tmp.path().to_path_buf();

    // Play using system command
    let status = if cfg!(target_os = "windows") {
        Command::new("powershell")
            .args(["-c", &format!("(New-Object Media.SoundPlayer '{}').PlaySync()", path.display())])
            .status()
    } else if cfg!(target_os = "macos") {
        Command::new("afplay")
            .arg(&path)
            .status()
    } else {
        // Linux
        Command::new("aplay")
            .arg(&path)
            .status()
    };

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(TtsError::Playback(format!("Playback command failed: {}", s))),
        Err(e) => Err(TtsError::Playback(format!("Failed to run playback command: {}", e))),
    }
}

/// Play audio on a background thread. Returns immediately.
pub fn play_wav_async(wav_data: Vec<u8>) {
    std::thread::spawn(move || {
        if let Err(e) = play_wav(&wav_data) {
            log::error!("Audio playback failed: {}", e);
        }
    });
}
