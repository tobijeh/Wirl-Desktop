//! First-week onboarding: how often each function has been used, and when
//! to show the one-time "try clean-up" tip and the shortcut card.

use serde::{Deserialize, Serialize};

/// Uses of a function after which the Home shortcut card disappears.
pub const LEARNED_AFTER: u32 = 3;
/// The clean-up tip can show from this many successful dictations on (the
/// tour's practice dictation counts as the first).
pub const TIP_FROM_DICTATION: u32 = 2;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Usage {
    /// Successful dictations (text inserted).
    pub dictations: u32,
    /// Successful clean-ups of a selection (not counting undo).
    pub cleanups: u32,
    /// The "try clean-up" tip has been shown. It only ever shows once.
    pub clean_tip_shown: bool,
}

impl Usage {
    /// Records a successful dictation. Returns true if the one-time clean-up
    /// tip should show now. `can_show` is false when status messages are
    /// off, so the tip is kept for later instead of being used up.
    pub fn record_dictation(&mut self, can_show: bool) -> bool {
        self.dictations = self.dictations.saturating_add(1);
        let show = can_show && !self.clean_tip_shown && self.cleanups == 0 && self.dictations >= TIP_FROM_DICTATION;
        if show {
            self.clean_tip_shown = true;
        }
        show
    }

    pub fn record_cleanup(&mut self) {
        self.cleanups = self.cleanups.saturating_add(1);
    }

    /// Show the "your two shortcuts" card on Home until both functions have
    /// been used a few times.
    pub fn show_shortcut_card(&self) -> bool {
        self.dictations < LEARNED_AFTER || self.cleanups < LEARNED_AFTER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_shows_once_on_second_dictation_if_cleanup_never_used() {
        let mut u = Usage::default();
        assert!(!u.record_dictation(true), "not on the very first dictation");
        assert!(u.record_dictation(true));
        assert!(u.clean_tip_shown);
        assert!(!u.record_dictation(true), "only once");
        assert_eq!(u.dictations, 3);
    }

    #[test]
    fn no_tip_once_cleanup_was_used() {
        let mut u = Usage::default();
        u.record_cleanup();
        assert!(!u.record_dictation(true));
        assert!(!u.record_dictation(true));
        assert!(!u.clean_tip_shown);
    }

    #[test]
    fn tip_waits_while_status_messages_are_off() {
        let mut u = Usage::default();
        u.record_dictation(false);
        assert!(!u.record_dictation(false));
        assert!(!u.clean_tip_shown, "not used up");
        assert!(u.record_dictation(true));
    }

    #[test]
    fn shortcut_card_until_both_used_three_times() {
        let mut u = Usage::default();
        assert!(u.show_shortcut_card());
        for _ in 0..3 {
            u.record_dictation(true);
        }
        assert!(u.show_shortcut_card(), "clean-up not learned yet");
        for _ in 0..2 {
            u.record_cleanup();
        }
        assert!(u.show_shortcut_card());
        u.record_cleanup();
        assert!(!u.show_shortcut_card());
    }

    #[test]
    fn missing_fields_default() {
        let u: Usage = serde_json::from_str(r#"{"dictations":4}"#).unwrap();
        assert_eq!(
            u,
            Usage {
                dictations: 4,
                ..Default::default()
            }
        );
    }
}
