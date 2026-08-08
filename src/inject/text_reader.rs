//! Read text from the current selection or clipboard.
//!
//! Uses the universal approach: snapshot clipboard → simulate Ctrl+C →
//! read clipboard → restore.
//!
//! This works in any application that supports Ctrl+C for copying.

use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::inject::{restore_clipboard_enabled, ClipboardSnapshot, InjectError};

/// Read selected text by simulating Ctrl+C and reading the clipboard.
/// Retries a few times if clipboard is empty (timing issue).
/// Returns the text, or an empty string if nothing was selected.
///
/// The previous clipboard contents (text *or* image) are snapshotted first
/// and restored afterwards, so the user's clipboard is not destroyed.
pub fn read_selected_text() -> Result<String, InjectError> {
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| InjectError::Keyboard(format!("Failed to create Enigo: {:?}", e)))?;

    // Snapshot whatever is on the clipboard (text or image) so we can restore
    // it afterwards — this avoids destroying non-text clipboard contents.
    let snapshot = ClipboardSnapshot::capture()?;

    // Clear clipboard first so we can detect if something was actually copied.
    {
        let mut cb = Clipboard::new()
            .map_err(|e| InjectError::Clipboard(format!("Failed to open clipboard: {}", e)))?;
        cb.clear().ok();
    }
    thread::sleep(Duration::from_millis(80));

    // Simulate Ctrl+C (or Cmd+C on macOS)
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    // Use Unicode chars so this compiles on all platforms (enigo's Key::C /
    // Key::V are not present on macOS).
    let c_key = Key::Unicode('c');
    let _ = enigo.key(modifier, Direction::Press);
    let _ = enigo.key(c_key, Direction::Click);
    let _ = enigo.key(modifier, Direction::Release);

    // Wait for clipboard to update — retry up to 10 times
    let mut selected = String::new();
    for i in 0..10 {
        thread::sleep(Duration::from_millis(100 + i * 30));
        if let Ok(mut cb) = Clipboard::new() {
            let text = cb.get_text().ok().unwrap_or_default();
            if !text.is_empty() {
                selected = text;
                break;
            }
        }
    }

    // Follow the same global preservation policy as clipboard injection.
    thread::sleep(Duration::from_millis(50));
    if restore_clipboard_enabled() {
        snapshot.restore();
    }

    Ok(selected)
}

/// Read clipboard text directly (no simulation).
pub fn read_clipboard_text() -> Result<String, InjectError> {
    let mut cb = Clipboard::new()
        .map_err(|e| InjectError::Clipboard(format!("Failed to open clipboard: {}", e)))?;
    Ok(cb.get_text().ok().unwrap_or_default())
}
