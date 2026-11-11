//! OS integration. Windows is the shipping target; other platforms get
//! safe stubs so the code stays portable for a later Mac version.

use std::time::Duration;
use wirl_core::insert::{InsertPlatform, PasteError};
use wirl_core::pill::Rect;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
pub use self::other::*;

/// The window in front (diagnostics): its handle, to tell whether it's the
/// same window, and its program's file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub window: isize,
    pub program: String,
}

/// What happened around the paste, for Diagnostics.
#[derive(Debug, Clone, Default)]
pub struct PasteReport {
    /// The app in front when Ctrl+V was sent.
    pub app: Option<AppInfo>,
    /// Modifier keys still held when Ctrl+V was sent (after waiting).
    pub keys_held: Vec<&'static str>,
    /// Other dictation apps running at the time.
    pub other_apps: Vec<String>,
}

/// The real insert platform used by `wirl_core::insert::insert_text`.
#[derive(Default)]
pub struct SystemInsert {
    pub report: PasteReport,
}

impl InsertPlatform for SystemInsert {
    type Snapshot = ClipboardSnapshot;
    fn snapshot_clipboard(&mut self) -> Option<ClipboardSnapshot> {
        clipboard_snapshot()
    }
    fn set_clipboard_text(&mut self, text: &str) -> Result<u64, String> {
        clipboard_set_text(text)?;
        Ok(clipboard_sequence())
    }
    fn clipboard_sequence(&self) -> u64 {
        clipboard_sequence()
    }
    fn restore_clipboard(&mut self, snap: ClipboardSnapshot) -> Result<(), String> {
        clipboard_restore(snap)
    }
    fn can_paste(&self) -> Result<(), PasteError> {
        can_paste()
    }
    fn send_paste(&mut self) -> Result<(), PasteError> {
        if !wait_modifiers_released(Duration::from_millis(1500)) {
            self.report.keys_held = held_modifiers();
        }
        self.report.app = foreground_app();
        send_ctrl_key('V')
    }
    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Copies the current selection in the focused app (Ctrl+C) and returns
/// it, restoring the clipboard afterwards. None if nothing was selected.
pub fn copy_selection() -> Option<String> {
    let snap = clipboard_snapshot();
    let before = clipboard_sequence();
    wait_modifiers_released(Duration::from_millis(1500));
    if send_ctrl_key('C').is_err() {
        return None;
    }
    let mut text = None;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(15));
        if clipboard_sequence() != before {
            // Give the source app a moment to finish writing all formats.
            std::thread::sleep(Duration::from_millis(30));
            text = clipboard_get_text();
            break;
        }
    }
    if let Some(s) = snap {
        let _ = clipboard_restore(s);
    }
    text
}

/// Work area of the monitor holding the active window, and the cursor.
pub fn placement_context() -> (Option<Rect>, (f64, f64)) {
    (active_work_area(), cursor_pos())
}
