//! Non-Windows stubs. Wirl only ships for Windows today; these keep the code
//! compiling elsewhere and fail safe (text is left on the clipboard).

use std::time::Duration;
use wirl_core::insert::PasteError;
use wirl_core::pill::Rect;

#[derive(Debug, Clone)]
pub struct ClipboardSnapshot;

pub fn clipboard_snapshot() -> Option<ClipboardSnapshot> {
    None
}
pub fn clipboard_set_text(_text: &str) -> Result<(), String> {
    Err("Clipboard is only implemented on Windows".into())
}
pub fn clipboard_get_text() -> Option<String> {
    None
}
pub fn clipboard_sequence() -> u64 {
    0
}
pub fn clipboard_restore(_s: ClipboardSnapshot) -> Result<(), String> {
    Ok(())
}
pub fn can_paste() -> Result<(), PasteError> {
    Err(PasteError::Failed("not supported on this OS".into()))
}
pub fn send_ctrl_key(_key: char) -> Result<(), PasteError> {
    Err(PasteError::Failed("not supported on this OS".into()))
}
pub fn wait_modifiers_released(_timeout: Duration) -> bool {
    true
}
pub fn held_modifiers() -> Vec<&'static str> {
    Vec::new()
}
pub fn foreground_app() -> Option<crate::platform::AppInfo> {
    None
}
pub fn running_dictation_apps() -> Vec<String> {
    Vec::new()
}
pub fn select_back(_chars: usize) -> Result<(), PasteError> {
    Err(PasteError::Failed("not supported on this OS".into()))
}
pub fn collapse_selection_right() {}
pub fn active_work_area() -> Option<Rect> {
    None
}
pub fn cursor_pos() -> (f64, f64) {
    (0.0, 0.0)
}
pub fn prepare_pill_window(_w: &tauri::WebviewWindow) {}
pub fn show_pill_window(w: &tauri::WebviewWindow, x: i32, y: i32, width: u32, height: u32) {
    let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    let _ = w.set_size(tauri::PhysicalSize::new(width, height));
    let _ = w.show();
}
pub fn hide_pill_window(w: &tauri::WebviewWindow) {
    let _ = w.hide();
}

#[derive(Debug, Default)]
pub struct MediaPauseToken(pub Vec<String>);

pub fn pause_playing_media() -> MediaPauseToken {
    MediaPauseToken::default()
}
pub fn resume_media(_t: MediaPauseToken) {}
pub fn microphone_allowed() -> bool {
    true
}

/// Opens a folder in the file manager.
pub fn open_folder(path: &std::path::Path) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
