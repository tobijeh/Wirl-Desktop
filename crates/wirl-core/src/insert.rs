//! Text insertion via clipboard paste, with clipboard restore and fallback,
//! and the clean-selection undo tracker. Platform code implements
//! [`InsertPlatform`]; the logic here is tested with a mock.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteError {
    /// The target window runs elevated or otherwise rejects injected input.
    Blocked,
    /// No window to paste into.
    NoTarget,
    /// SendInput or similar failed.
    Failed(String),
}

/// Platform hooks used by [`insert_text`].
pub trait InsertPlatform {
    type Snapshot;
    /// Saves the current clipboard (all formats where possible).
    fn snapshot_clipboard(&mut self) -> Option<Self::Snapshot>;
    /// Puts text on the clipboard. Returns the clipboard sequence number after the write.
    fn set_clipboard_text(&mut self, text: &str) -> Result<u64, String>;
    /// Current clipboard sequence number (changes on every clipboard write).
    fn clipboard_sequence(&self) -> u64;
    fn restore_clipboard(&mut self, snap: Self::Snapshot) -> Result<(), String>;
    /// Checks if pasting into the focused window is possible.
    fn can_paste(&self) -> Result<(), PasteError>;
    /// Sends Ctrl+V to the focused window.
    fn send_paste(&mut self) -> Result<(), PasteError>;
    fn sleep(&mut self, d: Duration);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    /// Pasted; the previous clipboard was restored (or there was none).
    Pasted { restored: bool },
    /// Could not paste; the text is left on the clipboard.
    CopiedOnly { reason: PasteError },
    /// Could not even set the clipboard.
    Failed(String),
}

/// How long to wait after Ctrl+V before restoring the clipboard. Apps read
/// the clipboard asynchronously, so restoring too early pastes old content.
pub const RESTORE_DELAY: Duration = Duration::from_millis(400);

/// Windows text controls expect CRLF line breaks.
pub fn to_platform_newlines(text: &str) -> String {
    if cfg!(windows) {
        text.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

/// What happens to the clipboard after the paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterPaste {
    /// Put the previous clipboard back (after [`RESTORE_DELAY`]).
    Restore,
    /// Leave the pasted text on the clipboard, so a paste that didn't land
    /// can be repeated with Ctrl+V.
    Keep,
}

/// Inserts `text` and puts the previous clipboard back.
pub fn insert_text<P: InsertPlatform>(p: &mut P, text: &str) -> InsertOutcome {
    insert_text_then(p, text, AfterPaste::Restore)
}

/// Inserts `text`, then restores the clipboard or keeps the text on it.
pub fn insert_text_then<P: InsertPlatform>(p: &mut P, text: &str, after: AfterPaste) -> InsertOutcome {
    let snapshot = match after {
        AfterPaste::Restore => p.snapshot_clipboard(),
        AfterPaste::Keep => None,
    };
    let seq = match p.set_clipboard_text(&to_platform_newlines(text)) {
        Ok(s) => s,
        Err(e) => return InsertOutcome::Failed(e),
    };
    if let Err(reason) = p.can_paste() {
        return InsertOutcome::CopiedOnly { reason };
    }
    if let Err(reason) = p.send_paste() {
        return InsertOutcome::CopiedOnly { reason };
    }
    if after == AfterPaste::Keep {
        return InsertOutcome::Pasted { restored: false };
    }
    p.sleep(RESTORE_DELAY);
    let mut restored = false;
    if let Some(snap) = snapshot {
        // Only restore if nobody else changed the clipboard meanwhile.
        if p.clipboard_sequence() == seq {
            restored = p.restore_clipboard(snap).is_ok();
        }
    }
    InsertOutcome::Pasted { restored }
}

// ---------- clean-selection undo ----------

#[derive(Debug, Clone, PartialEq, Eq)]
struct LastCleanup {
    original: String,
    cleaned: String,
}

/// What to do when the clean-selection hotkey is pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanDecision {
    /// Clean this text and paste the result over the selection.
    Clean(String),
    /// Replace the (re-)selected cleaned text with the original.
    Undo { original: String },
    /// Need to select the just-pasted text first (this many characters
    /// back from the cursor), copy it, then call `after_reselect`.
    Reselect { chars: usize },
    /// Nothing selected and nothing to undo.
    NothingSelected,
}

/// Tracks the last clean-selection so a second press can undo it while the
/// text is unchanged. Any difference resets it.
#[derive(Debug, Default)]
pub struct UndoTracker {
    last: Option<LastCleanup>,
}

/// Number of caret steps to select `text` backwards (CRLF counts as one).
pub fn caret_len(text: &str) -> usize {
    text.replace("\r\n", "\n").chars().count()
}

fn same_text(a: &str, b: &str) -> bool {
    a.replace("\r\n", "\n").trim_end() == b.replace("\r\n", "\n").trim_end()
}

impl UndoTracker {
    pub fn armed(&self) -> bool {
        self.last.is_some()
    }

    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Records a completed cleanup.
    pub fn record(&mut self, original: &str, cleaned: &str) {
        self.last = Some(LastCleanup {
            original: original.to_string(),
            cleaned: cleaned.to_string(),
        });
    }

    /// Records a completed undo: the next press cleans again.
    pub fn record_undo(&mut self) {
        self.last = None;
    }

    /// Decide based on the current selection (None/empty if nothing selected).
    pub fn on_hotkey(&mut self, selection: Option<&str>) -> CleanDecision {
        let sel = selection.filter(|s| !s.trim().is_empty());
        match (sel, &self.last) {
            (Some(s), Some(last)) if same_text(s, &last.cleaned) => CleanDecision::Undo {
                original: last.original.clone(),
            },
            (Some(s), _) => {
                // Different text selected: edited or a new selection.
                self.last = None;
                CleanDecision::Clean(s.to_string())
            }
            (None, Some(last)) => CleanDecision::Reselect {
                chars: caret_len(&last.cleaned),
            },
            (None, None) => CleanDecision::NothingSelected,
        }
    }

    /// After re-selecting `chars` characters back and copying them.
    /// Returns Undo if the text is unchanged since the cleanup, else resets.
    pub fn after_reselect(&mut self, reselected: Option<&str>) -> CleanDecision {
        match (&self.last, reselected) {
            (Some(last), Some(s)) if same_text(s, &last.cleaned) => CleanDecision::Undo {
                original: last.original.clone(),
            },
            _ => {
                self.last = None;
                CleanDecision::NothingSelected
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Mock {
        clipboard: Option<String>,
        seq: u64,
        blocked: bool,
        send_fails: bool,
        pasted: Vec<String>,
        external_write_during_sleep: bool,
    }

    impl InsertPlatform for Mock {
        type Snapshot = String;
        fn snapshot_clipboard(&mut self) -> Option<String> {
            self.clipboard.clone()
        }
        fn set_clipboard_text(&mut self, text: &str) -> Result<u64, String> {
            self.clipboard = Some(text.to_string());
            self.seq += 1;
            Ok(self.seq)
        }
        fn clipboard_sequence(&self) -> u64 {
            self.seq
        }
        fn restore_clipboard(&mut self, snap: String) -> Result<(), String> {
            self.clipboard = Some(snap);
            self.seq += 1;
            Ok(())
        }
        fn can_paste(&self) -> Result<(), PasteError> {
            if self.blocked {
                Err(PasteError::Blocked)
            } else {
                Ok(())
            }
        }
        fn send_paste(&mut self) -> Result<(), PasteError> {
            if self.send_fails {
                return Err(PasteError::Failed("SendInput".into()));
            }
            self.pasted.push(self.clipboard.clone().unwrap_or_default());
            Ok(())
        }
        fn sleep(&mut self, _d: Duration) {
            if self.external_write_during_sleep {
                self.clipboard = Some("user copied this".into());
                self.seq += 1;
            }
        }
    }

    #[test]
    fn pastes_and_restores_previous_clipboard() {
        let mut m = Mock {
            clipboard: Some("old".into()),
            ..Default::default()
        };
        let out = insert_text(&mut m, "hello");
        assert_eq!(out, InsertOutcome::Pasted { restored: true });
        assert_eq!(m.pasted, vec!["hello"]);
        assert_eq!(m.clipboard.as_deref(), Some("old"));
    }

    #[test]
    fn chinese_text_goes_through_the_clipboard_unchanged() {
        // Cantonese mode: characters are pasted, never typed key by key.
        let mut m = Mock {
            clipboard: Some("old".into()),
            ..Default::default()
        };
        let text = "我今日下晝有個 meeting，之後要 send 個 email 俾老闆㗎。";
        assert_eq!(insert_text(&mut m, text), InsertOutcome::Pasted { restored: true });
        assert_eq!(m.pasted, vec![text]);
        assert_eq!(m.clipboard.as_deref(), Some("old"));
    }

    #[test]
    fn keep_leaves_the_text_on_the_clipboard() {
        let mut m = Mock {
            clipboard: Some("old".into()),
            ..Default::default()
        };
        let out = insert_text_then(&mut m, "dictated", AfterPaste::Keep);
        assert_eq!(out, InsertOutcome::Pasted { restored: false });
        assert_eq!(m.pasted, vec!["dictated"]);
        // Still there to paste again with Ctrl+V if the paste didn't land.
        assert_eq!(m.clipboard.as_deref(), Some("dictated"));
    }

    #[test]
    fn does_not_clobber_newer_clipboard() {
        let mut m = Mock {
            clipboard: Some("old".into()),
            external_write_during_sleep: true,
            ..Default::default()
        };
        let out = insert_text(&mut m, "hello");
        assert_eq!(out, InsertOutcome::Pasted { restored: false });
        assert_eq!(m.clipboard.as_deref(), Some("user copied this"));
    }

    #[test]
    fn blocked_paste_leaves_text_on_clipboard() {
        let mut m = Mock {
            clipboard: Some("old".into()),
            blocked: true,
            ..Default::default()
        };
        let out = insert_text(&mut m, "hello");
        assert_eq!(
            out,
            InsertOutcome::CopiedOnly {
                reason: PasteError::Blocked
            }
        );
        assert!(m.pasted.is_empty());
        assert!(m.clipboard.as_deref().unwrap().contains("hello"));
    }

    #[test]
    fn failed_send_leaves_text_on_clipboard() {
        let mut m = Mock {
            send_fails: true,
            ..Default::default()
        };
        let out = insert_text(&mut m, "hi");
        assert!(matches!(out, InsertOutcome::CopiedOnly { .. }));
        assert_eq!(m.clipboard.as_deref(), Some("hi"));
    }

    #[test]
    fn line_breaks_preserved() {
        let mut m = Mock::default();
        insert_text(&mut m, "a\nb\n- c");
        let expected = if cfg!(windows) { "a\r\nb\r\n- c" } else { "a\nb\n- c" };
        assert_eq!(m.pasted[0], expected);
    }

    #[test]
    fn undo_when_same_text_selected() {
        let mut u = UndoTracker::default();
        assert_eq!(u.on_hotkey(Some("teh cat")), CleanDecision::Clean("teh cat".into()));
        u.record("teh cat", "The cat.");
        assert_eq!(
            u.on_hotkey(Some("The cat.")),
            CleanDecision::Undo {
                original: "teh cat".into()
            }
        );
        u.record_undo();
        assert!(!u.armed());
    }

    #[test]
    fn undo_via_reselect_when_unchanged() {
        let mut u = UndoTracker::default();
        u.record("raw\r\ntext", "Clean\ntext.");
        assert_eq!(u.on_hotkey(None), CleanDecision::Reselect { chars: 11 });
        assert_eq!(
            u.after_reselect(Some("Clean\r\ntext.")),
            CleanDecision::Undo {
                original: "raw\r\ntext".into()
            }
        );
    }

    #[test]
    fn editing_resets_undo() {
        let mut u = UndoTracker::default();
        u.record("raw", "Clean text.");
        // Text was edited: reselecting finds something else.
        assert_eq!(u.on_hotkey(None), CleanDecision::Reselect { chars: 11 });
        assert_eq!(u.after_reselect(Some("Clean text!")), CleanDecision::NothingSelected);
        assert!(!u.armed());
        // A different selection also resets and cleans that instead.
        u.record("raw", "Clean text.");
        assert_eq!(
            u.on_hotkey(Some("other words")),
            CleanDecision::Clean("other words".into())
        );
        assert!(!u.armed());
    }

    #[test]
    fn nothing_selected() {
        let mut u = UndoTracker::default();
        assert_eq!(u.on_hotkey(Some("   ")), CleanDecision::NothingSelected);
    }
}
