//! Shared, UI-free local backend clients used by both Vox desktop and its
//! optional HTTP router. This crate deliberately contains no tray, hotkey,
//! window, audio-device, lifecycle, or gateway state.

pub mod crisper;
pub mod longcat;
