//! Keyboard simulation injection: types text into the currently focused
//! application using `enigo`.

use enigo::{Enigo, Keyboard, Settings};
use std::thread;
use std::time::Duration;

use crate::inject::InjectError;

/// Inject text by simulating keystrokes.
pub fn inject_keyboard(text: &str) -> Result<(), InjectError> {
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| InjectError::Keyboard(format!("Failed to create Enigo instance: {:?}", e)))?;

    // Small delay to ensure the target app is ready
    thread::sleep(Duration::from_millis(50));

    // Text longer than 100 chars: use clipboard fallback to avoid slowness
    if text.len() > 100 {
        return super::clipboard::inject_clipboard(text);
    }

    // Use enigo.text() which handles Unicode text natively
    enigo
        .text(text)
        .map_err(|e| InjectError::Keyboard(format!("enigo.text() failed: {:?}", e)))?;

    Ok(())
}
