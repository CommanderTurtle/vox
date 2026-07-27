//! Text injection into the currently focused text field.
//!
//! Supports two modes:
//! - **Keyboard**: simulates typing each character via `enigo`
//! - **Clipboard**: writes text to clipboard, simulates Ctrl+V, then restores

pub mod clipboard;
pub mod keyboard;
pub mod text_reader;

use arboard::{Clipboard, ImageData};

use crate::config::Config;

/// Text injection error.
#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("Keyboard simulation failed: {0}")]
    Keyboard(String),
    #[error("Clipboard operation failed: {0}")]
    Clipboard(String),
}

/// A snapshot of whatever the clipboard held (text *or* image) before we
/// overwrote it, so it can be restored afterwards.
///
/// This protects the user's clipboard contents: previously the code only
/// saved/restored text, which meant an image on the clipboard was destroyed
/// and could not be recovered. We now snapshot whichever format is present.
pub enum ClipboardSnapshot {
    /// Clipboard held plain text.
    Text(String),
    /// Clipboard held an image.
    Image(ImageData<'static>),
    /// Clipboard was empty (or held a format we don't snapshot).
    Empty,
}

impl ClipboardSnapshot {
    /// Capture the current clipboard contents.
    pub fn capture() -> Result<Self, InjectError> {
        let mut cb = Clipboard::new()
            .map_err(|e| InjectError::Clipboard(format!("Failed to open clipboard: {}", e)))?;
        // Prefer text; arboard returns ContentNotAvailable when there's none.
        if let Ok(text) = cb.get_text() {
            if !text.is_empty() {
                return Ok(Self::Text(text));
            }
        }
        // No (non-empty) text — try an image so we don't destroy it.
        if let Ok(img) = cb.get_image() {
            return Ok(Self::Image(img));
        }
        Ok(Self::Empty)
    }

    /// Restore the snapshot back onto the clipboard. Best-effort: errors are
    /// ignored because we cannot do anything useful with them here.
    pub fn restore(self) {
        if let Ok(mut cb) = Clipboard::new() {
            match self {
                Self::Text(t) => {
                    let _ = cb.set_text(t);
                }
                Self::Image(img) => {
                    let _ = cb.set_image(img);
                }
                Self::Empty => {}
            }
        }
    }
}

/// The injection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectMode {
    Keyboard,
    Clipboard,
}

impl InjectMode {
    pub fn from_config(cfg: &Config) -> Self {
        match cfg.inject.mode.as_str() {
            "clipboard" => InjectMode::Clipboard,
            _ => InjectMode::Keyboard,
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            InjectMode::Keyboard => InjectMode::Clipboard,
            InjectMode::Clipboard => InjectMode::Keyboard,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            InjectMode::Keyboard => "keyboard",
            InjectMode::Clipboard => "clipboard",
        }
    }
}

/// Inject text at the current cursor position using the specified mode.
pub fn inject_text(text: &str, mode: InjectMode) -> Result<(), InjectError> {
    match mode {
        InjectMode::Keyboard => keyboard::inject_keyboard(text),
        InjectMode::Clipboard => clipboard::inject_clipboard(text),
    }
}
