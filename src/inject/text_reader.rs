//! Read text from the current selection or clipboard.
//!
//! Uses the universal approach: save clipboard → simulate Ctrl+C →
//! read clipboard → restore.
//!
//! This works in any application that supports Ctrl+C for copying.

use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use enigo::{Enigo, Key, Keyboard, Direction, Settings};

use crate::inject::InjectError;

/// Read selected text by simulating Ctrl+C and reading the clipboard.
/// Retries a few times if clipboard is empty (timing issue).
/// Returns the text, or an empty string if nothing was selected.
pub fn read_selected_text() -> Result<String, InjectError> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| {
        InjectError::Keyboard(format!("Failed to create Enigo: {:?}", e))
    })?;

    // Save current clipboard content
    let mut cb = Clipboard::new().map_err(|e| {
        InjectError::Clipboard(format!("Failed to open clipboard: {}", e))
    })?;
    let saved = cb.get_text().ok();

    // Clear clipboard first so we can detect if something was actually copied
    cb.clear().ok();
    drop(cb); // close clipboard handle so other apps can write to it

    thread::sleep(Duration::from_millis(80));

    // Simulate Ctrl+C (or Cmd+C on macOS)
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    let _ = enigo.key(modifier, Direction::Press);
    let _ = enigo.key(Key::C, Direction::Click);
    let _ = enigo.key(modifier, Direction::Release);

    // Wait for clipboard to update — retry up to 10 times
    let mut selected = String::new();
    for i in 0..10 {
        thread::sleep(Duration::from_millis(100 + i * 30));
        let mut cb2 = Clipboard::new().ok();
        if let Some(ref mut cb) = cb2 {
            let text = cb.get_text().ok().unwrap_or_default();
            if !text.is_empty() {
                selected = text;
                break;
            }
        }
    }

    // Restore saved clipboard
    if let Some(ref saved_text) = saved {
        thread::sleep(Duration::from_millis(50));
        if let Ok(mut cb) = Clipboard::new() {
            let _ = cb.set_text(saved_text);
        }
    }

    Ok(selected)
}

/// Read clipboard text directly (no simulation).
pub fn read_clipboard_text() -> Result<String, InjectError> {
    let mut cb = Clipboard::new().map_err(|e| {
        InjectError::Clipboard(format!("Failed to open clipboard: {}", e))
    })?;
    Ok(cb.get_text().ok().unwrap_or_default())
}
