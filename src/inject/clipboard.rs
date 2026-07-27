//! Clipboard-based text injection.
//!
//! Flow: snapshot current clipboard → write new text → simulate Ctrl+V →
//! restore.

use arboard::Clipboard;
use std::thread;
use std::time::Duration;

use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::inject::{ClipboardSnapshot, InjectError};

/// Inject text by writing to clipboard and simulating Ctrl+V.
///
/// The previous clipboard contents (text *or* image) are snapshotted and
/// restored after the paste, so the user's clipboard is not destroyed.
pub fn inject_clipboard(text: &str) -> Result<(), InjectError> {
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| InjectError::Keyboard(format!("Failed to create Enigo instance: {:?}", e)))?;

    // Snapshot whatever is on the clipboard (text or image) so we can restore
    // it afterwards — this avoids destroying non-text clipboard contents.
    let snapshot = ClipboardSnapshot::capture()?;

    // Write our text
    {
        let mut cb = Clipboard::new()
            .map_err(|e| InjectError::Clipboard(format!("Failed to open clipboard: {}", e)))?;
        cb.set_text(text)
            .map_err(|e| InjectError::Clipboard(format!("Failed to set clipboard text: {}", e)))?;
    }

    // Small delay for clipboard to propagate
    thread::sleep(Duration::from_millis(30));

    // Simulate Ctrl+V (or Cmd+V on macOS)
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    // Use Unicode chars so this compiles on all platforms (enigo's Key::V /
    // Key::C are not present on macOS).
    let v_key = Key::Unicode('v');
    let _ = enigo.key(modifier, Direction::Press);
    let _ = enigo.key(v_key, Direction::Click);
    let _ = enigo.key(modifier, Direction::Release);

    // Wait for paste to complete, then restore the original clipboard.
    thread::sleep(Duration::from_millis(50));
    snapshot.restore();

    Ok(())
}
