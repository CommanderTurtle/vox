//! Global hotkey listener using `rdev`.
//!
//! Parses hotkey strings (e.g. "Alt+`", "Ctrl+Shift+A") and monitors
//! global key events on a background thread.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rdev::{listen, EventType, Key};

/// Events sent from the hotkey listener to the main thread.
#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    /// The recording toggle hotkey was pressed
    RecordTogglePressed,
    /// The recording toggle hotkey was released
    RecordToggleReleased,
    /// Switch ASR engine hotkey was triggered
    EngineSwitch,
    /// Switch inject mode hotkey was triggered
    InjectModeSwitch,
    /// TTS trigger hotkey — read selected text and speak it
    TtsTrigger,
}

/// A parsed hotkey binding: combination of modifier keys + a main key.
#[derive(Debug, Clone)]
pub struct HotkeyBinding {
    /// Required modifier keys (alt, ctrl, shift, meta)
    pub modifiers: HashSet<Key>,
    /// The main key
    pub key: Key,
}

impl HotkeyBinding {
    /// Parse a hotkey string like "Alt+`", "Ctrl+Shift+A".
    /// Returns None if the string is invalid.
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
        if parts.is_empty() {
            return None;
        }

        let mut modifiers = HashSet::new();
        let mut main_key = None;

        for part in &parts {
            match part.to_lowercase().as_str() {
                "alt" => { modifiers.insert(Key::Alt); }
                "ctrl" | "control" => { modifiers.insert(Key::ControlLeft); }
                "shift" => { modifiers.insert(Key::ShiftLeft); }
                "meta" | "win" | "cmd" | "super" => { modifiers.insert(Key::MetaLeft); }
                other => {
                    if let Some(key) = parse_key(other) {
                        main_key = Some(key);
                    } else {
                        log::warn!("Unrecognized hotkey part: {}", other);
                        return None;
                    }
                }
            }
        }

        main_key.map(|key| HotkeyBinding { modifiers, key })
    }

    /// Check if the current key state matches this binding.
    pub fn matches(&self, pressed: &HashSet<Key>, event_key: &Key) -> bool {
        if event_key != &self.key {
            return false;
        }
        // All required modifiers must be pressed
        for required in &self.modifiers {
            if !pressed.contains(required) {
                // Accept either Left/Right variant as match
                let counterpart = match required {
                    Key::Alt => Key::AltGr,
                    Key::ControlLeft => Key::ControlRight,
                    Key::ShiftLeft => Key::ShiftRight,
                    Key::MetaLeft => Key::MetaRight,
                    _ => continue,
                };
                if !pressed.contains(&counterpart) {
                    return false;
                }
            }
        }
        true
    }
}

/// Start the global hotkey listener on a background thread.
///
/// `rx` receives events when hotkeys are triggered.
/// `stop_flag` signals the listener thread to stop.
/// `app_state` is used for recording toggle logic.
pub fn start_hotkey_listener(
    record_binding: HotkeyBinding,
    engine_switch_binding: HotkeyBinding,
    inject_switch_binding: HotkeyBinding,
    tts_binding: HotkeyBinding,
    sender: crossbeam::channel::Sender<HotkeyEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut pressed: HashSet<Key> = HashSet::new();
        let mut record_was_pressed = false;
        // Per-action debounce flags: each action only fires once per physical
        // press, suppressing the OS key-repeat that would otherwise retrigger
        // the action for as long as the keys are held down.
        let mut engine_was_pressed = false;
        let mut inject_was_pressed = false;
        let mut tts_was_pressed = false;

        // rdev's listen() blocks forever; it processes events in callback.
        if let Err(e) = listen(move |event| {
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }

            match event.event_type {
                EventType::KeyPress(key) => {
                    pressed.insert(key);

                    // Record toggle: detect press
                    if record_binding.matches(&pressed, &key) && !record_was_pressed {
                        record_was_pressed = true;
                        let _ = sender.send(HotkeyEvent::RecordTogglePressed);
                    }

                    // Engine switch (trigger once per press)
                    if engine_switch_binding.matches(&pressed, &key) && !engine_was_pressed {
                        engine_was_pressed = true;
                        let _ = sender.send(HotkeyEvent::EngineSwitch);
                    }

                    // Inject mode switch (trigger once per press)
                    if inject_switch_binding.matches(&pressed, &key) && !inject_was_pressed {
                        inject_was_pressed = true;
                        let _ = sender.send(HotkeyEvent::InjectModeSwitch);
                    }

                    // TTS trigger (trigger once per press)
                    if tts_binding.matches(&pressed, &key) && !tts_was_pressed {
                        tts_was_pressed = true;
                        let _ = sender.send(HotkeyEvent::TtsTrigger);
                    }
                }
                EventType::KeyRelease(key) => {
                    pressed.remove(&key);

                    // Record toggle: detect release
                    if (key == record_binding.key || record_binding.modifiers.contains(&key))
                        && record_was_pressed
                    {
                        record_was_pressed = false;
                        let _ = sender.send(HotkeyEvent::RecordToggleReleased);
                    }

                    // Reset debounce flags when the action's main key is released
                    if key == engine_switch_binding.key {
                        engine_was_pressed = false;
                    }
                    if key == inject_switch_binding.key {
                        inject_was_pressed = false;
                    }
                    if key == tts_binding.key {
                        tts_was_pressed = false;
                    }
                }
                _ => {}
            }
        }) {
            log::error!("Hotkey listener error: {:?}", e);
        }
    });
}

/// Parse a single key name into an rdev Key.
fn parse_key(s: &str) -> Option<Key> {
    match s.to_lowercase().as_str() {
        "`" | "backquote" => Some(Key::BackQuote),
        "-" | "minus" => Some(Key::Minus),
        "=" | "equal" => Some(Key::Equal),
        "[" => Some(Key::LeftBracket),
        "]" => Some(Key::RightBracket),
        "\\" | "backslash" => Some(Key::BackSlash),
        ";" | "semicolon" => Some(Key::SemiColon),
        "'" | "quote" => Some(Key::Quote),
        "," | "comma" => Some(Key::Comma),
        "." | "period" | "dot" => Some(Key::Dot),
        "/" | "slash" => Some(Key::Slash),
        "space" | " " => Some(Key::Space),
        "enter" | "return" => Some(Key::Return),
        "tab" => Some(Key::Tab),
        "backspace" => Some(Key::Backspace),
        "escape" | "esc" => Some(Key::Escape),
        "delete" | "del" => Some(Key::Delete),

        // Letters
        "a" => Some(Key::KeyA),
        "b" => Some(Key::KeyB),
        "c" => Some(Key::KeyC),
        "d" => Some(Key::KeyD),
        "e" => Some(Key::KeyE),
        "f" => Some(Key::KeyF),
        "g" => Some(Key::KeyG),
        "h" => Some(Key::KeyH),
        "i" => Some(Key::KeyI),
        "j" => Some(Key::KeyJ),
        "k" => Some(Key::KeyK),
        "l" => Some(Key::KeyL),
        "m" => Some(Key::KeyM),
        "n" => Some(Key::KeyN),
        "o" => Some(Key::KeyO),
        "p" => Some(Key::KeyP),
        "q" => Some(Key::KeyQ),
        "r" => Some(Key::KeyR),
        "s" => Some(Key::KeyS),
        "t" => Some(Key::KeyT),
        "u" => Some(Key::KeyU),
        "v" => Some(Key::KeyV),
        "w" => Some(Key::KeyW),
        "x" => Some(Key::KeyX),
        "y" => Some(Key::KeyY),
        "z" => Some(Key::KeyZ),

        // Numbers
        "0" => Some(Key::Num0),
        "1" => Some(Key::Num1),
        "2" => Some(Key::Num2),
        "3" => Some(Key::Num3),
        "4" => Some(Key::Num4),
        "5" => Some(Key::Num5),
        "6" => Some(Key::Num6),
        "7" => Some(Key::Num7),
        "8" => Some(Key::Num8),
        "9" => Some(Key::Num9),

        // Function keys
        "f1" => Some(Key::F1),
        "f2" => Some(Key::F2),
        "f3" => Some(Key::F3),
        "f4" => Some(Key::F4),
        "f5" => Some(Key::F5),
        "f6" => Some(Key::F6),
        "f7" => Some(Key::F7),
        "f8" => Some(Key::F8),
        "f9" => Some(Key::F9),
        "f10" => Some(Key::F10),
        "f11" => Some(Key::F11),
        "f12" => Some(Key::F12),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_parse_simple_key() {
        let binding = HotkeyBinding::parse("Alt+`").unwrap();
        assert!(binding.modifiers.contains(&Key::Alt));
        assert_eq!(binding.key, Key::BackQuote);
    }

    #[test]
    fn test_parse_combo() {
        let binding = HotkeyBinding::parse("Ctrl+Shift+A").unwrap();
        assert!(binding.modifiers.contains(&Key::ControlLeft));
        assert!(binding.modifiers.contains(&Key::ShiftLeft));
        assert_eq!(binding.key, Key::KeyA);
    }

    #[test]
    fn test_parse_letter() {
        let binding = HotkeyBinding::parse("Alt+Shift+E").unwrap();
        assert!(binding.modifiers.contains(&Key::Alt));
        assert!(binding.modifiers.contains(&Key::ShiftLeft));
        assert_eq!(binding.key, Key::KeyE);
    }

    #[test]
    fn test_matches() {
        let binding = HotkeyBinding::parse("Alt+Shift+E").unwrap();
        let mut pressed = HashSet::new();
        pressed.insert(Key::Alt);
        pressed.insert(Key::ShiftLeft);
        assert!(binding.matches(&pressed, &Key::KeyE));

        // Missing modifier
        pressed.remove(&Key::Alt);
        assert!(!binding.matches(&pressed, &Key::KeyE));
    }

    #[test]
    fn test_parse_invalid() {
        assert!(HotkeyBinding::parse("").is_none());
        assert!(HotkeyBinding::parse("++A").is_none()); // should be handled gracefully
    }
}
