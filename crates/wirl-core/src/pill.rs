//! The on-screen pill: states, timing and placement.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PillPosition {
    #[default]
    BottomCenter,
    TopCenter,
    NearMouse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PillSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl PillSize {
    /// Logical (unscaled) window size of the pill.
    pub fn dimensions(&self) -> (f64, f64) {
        match self {
            PillSize::Small => (200.0, 40.0),
            PillSize::Medium => (240.0, 48.0),
            PillSize::Large => (290.0, 58.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PillState {
    Hidden,
    /// Red ring around the selected icon, live level wave, "Esc to cancel".
    Recording {
        cancel_hint: String,
    },
    /// Spinner, "Cleaning up".
    Processing,
    /// Green check, "Inserted".
    Inserted,
    /// Green check, "Cleaned", "<hotkey> to undo".
    Cleaned {
        undo_hint: String,
    },
    /// Green check, "Undone".
    Undone,
    /// Amber, "Copied, press Ctrl+V".
    Copied,
    /// Amber with a short plain message.
    Error {
        message: String,
    },
}

impl PillState {
    pub fn visible(&self) -> bool {
        !matches!(self, PillState::Hidden)
    }

    /// How long the state stays before fading out. None = until changed.
    pub fn auto_hide_ms(&self) -> Option<u64> {
        match self {
            PillState::Hidden | PillState::Recording { .. } | PillState::Processing => None,
            PillState::Inserted | PillState::Undone => Some(1_000),
            PillState::Cleaned { .. } => Some(3_000),
            PillState::Copied => Some(4_000),
            PillState::Error { .. } => Some(3_500),
        }
    }

    pub fn recording(cancel_hotkey: &str) -> Self {
        PillState::Recording {
            cancel_hint: format!("{cancel_hotkey} to cancel"),
        }
    }

    pub fn cleaned(clean_hotkey: &str) -> Self {
        PillState::Cleaned {
            undo_hint: format!("{clean_hotkey} to undo"),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        PillState::Error { message: msg.into() }
    }

    /// Whether this is an informational success message that the
    /// "status messages" toggle may suppress. Problems are always shown.
    pub fn is_status_message(&self) -> bool {
        matches!(
            self,
            PillState::Inserted | PillState::Cleaned { .. } | PillState::Undone
        )
    }
}

/// Applies the status-message preference: with messages off, success
/// confirmations become Hidden; errors and "Copied" still show.
pub fn effective_state(state: PillState, status_messages: bool) -> PillState {
    if !status_messages && state.is_status_message() {
        PillState::Hidden
    } else {
        state
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where to put the pill (top-left, physical pixels) on the monitor that
/// holds the active window. `work_area` excludes the taskbar.
pub fn place(position: PillPosition, work_area: Rect, cursor: (f64, f64), pill_w: f64, pill_h: f64) -> (f64, f64) {
    let margin = (work_area.h * 0.06).clamp(24.0, 90.0);
    let center_x = work_area.x + (work_area.w - pill_w) / 2.0;
    let (x, y) = match position {
        PillPosition::BottomCenter => (center_x, work_area.y + work_area.h - pill_h - margin),
        PillPosition::TopCenter => (center_x, work_area.y + margin),
        PillPosition::NearMouse => {
            let below = cursor.1 + 28.0;
            let y = if below + pill_h > work_area.y + work_area.h {
                cursor.1 - 28.0 - pill_h
            } else {
                below
            };
            (cursor.0 - pill_w / 2.0, y)
        }
    };
    // Keep fully on screen.
    let x = x.clamp(work_area.x, work_area.x + (work_area.w - pill_w).max(0.0));
    let y = y.clamp(work_area.y, work_area.y + (work_area.h - pill_h).max(0.0));
    (x.round(), y.round())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_when_idle() {
        assert!(!PillState::Hidden.visible());
        assert!(PillState::Processing.visible());
        assert_eq!(PillState::Hidden.auto_hide_ms(), None);
    }

    #[test]
    fn fade_timings() {
        assert_eq!(PillState::Inserted.auto_hide_ms(), Some(1_000));
        assert_eq!(PillState::cleaned("Ctrl+Alt+C").auto_hide_ms(), Some(3_000));
        assert_eq!(PillState::recording("Esc").auto_hide_ms(), None);
        assert_eq!(PillState::Processing.auto_hide_ms(), None);
    }

    #[test]
    fn hints_use_actual_hotkeys() {
        assert_eq!(
            PillState::cleaned("Ctrl+Shift+K"),
            PillState::Cleaned {
                undo_hint: "Ctrl+Shift+K to undo".into()
            }
        );
        assert_eq!(
            PillState::recording("Esc"),
            PillState::Recording {
                cancel_hint: "Esc to cancel".into()
            }
        );
    }

    #[test]
    fn status_messages_toggle() {
        assert_eq!(effective_state(PillState::Inserted, false), PillState::Hidden);
        assert_eq!(effective_state(PillState::Copied, false), PillState::Copied);
        assert!(matches!(
            effective_state(PillState::error("x"), false),
            PillState::Error { .. }
        ));
        assert_eq!(effective_state(PillState::Inserted, true), PillState::Inserted);
    }

    #[test]
    fn serializes_for_ui() {
        let s = serde_json::to_string(&PillState::error("No internet. Inserted raw text.")).unwrap();
        assert_eq!(s, r#"{"kind":"error","message":"No internet. Inserted raw text."}"#);
        assert_eq!(
            serde_json::to_string(&PillState::Hidden).unwrap(),
            r#"{"kind":"hidden"}"#
        );
    }

    #[test]
    fn placement() {
        let wa = Rect {
            x: 1920.0,
            y: 0.0,
            w: 1920.0,
            h: 1040.0,
        };
        let (x, y) = place(PillPosition::BottomCenter, wa, (0.0, 0.0), 240.0, 48.0);
        assert_eq!(x, 1920.0 + 840.0);
        assert!(y > 900.0 && y + 48.0 <= 1040.0);
        let (_, y) = place(PillPosition::TopCenter, wa, (0.0, 0.0), 240.0, 48.0);
        assert!(y < 100.0);
        // Near mouse at the bottom edge flips above the cursor and stays on screen.
        let (x, y) = place(PillPosition::NearMouse, wa, (1925.0, 1030.0), 240.0, 48.0);
        assert_eq!(x, 1920.0);
        assert!(y + 48.0 <= 1030.0);
    }
}
