//! Clipboard-based text injection.
//!
//! Flow: save current clipboard → write new text → simulate Ctrl+V → restore.

use arboard::Clipboard;
use std::thread;
use std::time::Duration;

use enigo::{Enigo, Key, Keyboard, Direction, Settings};

use crate::inject::InjectError;

/// Inject text by writing to clipboard and simulating Ctrl+V.
pub fn inject_clipboard(text: &str) -> Result<(), InjectError> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| {
        InjectError::Keyboard(format!("Failed to create Enigo instance: {:?}", e))
    })?;

    // Save current clipboard content
    let saved = {
        let mut cb = Clipboard::new().map_err(|e| {
            InjectError::Clipboard(format!("Failed to open clipboard: {}", e))
        })?;
        let saved = cb.get_text().ok();
        // Write our text
        cb.set_text(text).map_err(|e| {
            InjectError::Clipboard(format!("Failed to set clipboard text: {}", e))
        })?;
        saved
    };

    // Small delay for clipboard to propagate
    thread::sleep(Duration::from_millis(30));

    // Simulate Ctrl+V (or Cmd+V on macOS)
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    let _ = enigo.key(modifier, Direction::Press);
    let _ = enigo.key(Key::V, Direction::Click);
    let _ = enigo.key(modifier, Direction::Release);

    // Wait for paste to complete
    thread::sleep(Duration::from_millis(50));

    // Restore saved clipboard content
    if let Some(text) = saved {
        thread::sleep(Duration::from_millis(50));
        let mut cb = Clipboard::new().ok();
        if let Some(ref mut cb) = cb {
            let _ = cb.set_text(&text);
        }
    }

    Ok(())
}
