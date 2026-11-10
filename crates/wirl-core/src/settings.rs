//! User settings (autosaved JSON). The API key, dictionary and pairings are
//! stored separately so "Reset to defaults" never touches them.

use crate::hotkey::{default_for, Action, Hotkey};
use crate::onboarding::Usage;
use crate::pill::{PillPosition, PillSize};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ModelId {
    /// base.en, quantized.
    #[default]
    Fast,
    /// small.en
    Accurate,
    /// base (multilingual)
    Multilingual,
    /// SenseVoice (sherpa-onnx), Cantonese with English mixed in.
    Cantonese,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkeys {
    pub dictate: Hotkey,
    pub cancel: Hotkey,
    pub clean_selection: Hotkey,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Hotkeys {
            dictate: default_for(Action::Dictate),
            cancel: default_for(Action::Cancel),
            clean_selection: default_for(Action::CleanSelection),
        }
    }
}

impl Hotkeys {
    pub fn get(&self, a: Action) -> &Hotkey {
        match a {
            Action::Dictate => &self.dictate,
            Action::Cancel => &self.cancel,
            Action::CleanSelection => &self.clean_selection,
        }
    }
    pub fn set(&mut self, a: Action, h: Hotkey) {
        match a {
            Action::Dictate => self.dictate = h,
            Action::Cancel => self.cancel = h,
            Action::CleanSelection => self.clean_selection = h,
        }
    }
    pub fn all(&self) -> Vec<(Action, Hotkey)> {
        vec![
            (Action::Dictate, self.dictate.clone()),
            (Action::Cancel, self.cancel.clone()),
            (Action::CleanSelection, self.clean_selection.clone()),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub start_with_windows: bool,
    pub first_run_done: bool,

    // Dictation
    pub model: ModelId,
    /// Whisper language code ("en", "auto", "de", ...).
    pub language: String,
    pub pause_media: bool,

    // AI cleanup
    pub cleanup_enabled: bool,
    pub style_notes: String,

    // Hotkeys
    pub hotkeys: Hotkeys,
    pub hold_to_talk: bool,

    // Indicator
    pub pill_position: PillPosition,
    pub pill_size: PillSize,
    /// File stem from icons.json; empty = the default from icons.json.
    pub icon: String,
    pub chime: bool,
    pub status_messages: bool,

    // Developer
    pub developer_options: bool,

    /// How often dictation and clean-up have been used (onboarding tips).
    /// Kept by "Reset to defaults".
    pub usage: Usage,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            start_with_windows: true,
            first_run_done: false,
            model: ModelId::Fast,
            language: "en".into(),
            pause_media: true,
            cleanup_enabled: true,
            style_notes: String::new(),
            hotkeys: Hotkeys::default(),
            hold_to_talk: true,
            pill_position: PillPosition::BottomCenter,
            pill_size: PillSize::Medium,
            icon: String::new(),
            chime: true,
            status_messages: true,
            developer_options: false,
            usage: Usage::default(),
        }
    }
}

impl Settings {
    /// "Reset to defaults" keeps first-run state and usage counts; the API
    /// key, dictionary and pairings live elsewhere and are untouched.
    pub fn reset_to_defaults(&self) -> Settings {
        Settings {
            first_run_done: self.first_run_done,
            usage: self.usage.clone(),
            ..Settings::default()
        }
    }

    pub fn from_json(s: &str) -> Settings {
        serde_json::from_str(s).unwrap_or_default()
    }
}

/// Languages offered in the picker: (whisper code, label).
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English (US)"),
    ("auto", "Detect automatically"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("sv", "Swedish"),
    ("da", "Danish"),
    ("no", "Norwegian"),
    ("fi", "Finnish"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("tr", "Turkish"),
    ("ar", "Arabic"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("zh", "Chinese"),
];

pub fn language_label(code: &str) -> &'static str {
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, l)| *l)
        .unwrap_or("English (US)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert!(s.enabled && s.start_with_windows && s.pause_media && s.hold_to_talk && s.chime);
        assert_eq!(s.model, ModelId::Fast);
        assert_eq!(s.hotkeys.dictate.to_string(), "Ctrl+Alt+Space");
        assert_eq!(s.hotkeys.cancel.to_string(), "Esc");
        assert_eq!(s.hotkeys.clean_selection.to_string(), "Ctrl+Alt+C");
        assert_eq!(s.pill_position, PillPosition::BottomCenter);
    }

    #[test]
    fn partial_json_fills_defaults() {
        let s = Settings::from_json(r#"{"chime":false,"pillPosition":"nearMouse"}"#);
        assert!(!s.chime);
        assert_eq!(s.pill_position, PillPosition::NearMouse);
        assert!(s.hold_to_talk);
        assert_eq!(Settings::from_json("garbage"), Settings::default());
    }

    #[test]
    fn reset_keeps_first_run_flag() {
        let mut s = Settings {
            first_run_done: true,
            chime: false,
            style_notes: "x".into(),
            ..Default::default()
        };
        s.usage.record_cleanup();
        let r = s.reset_to_defaults();
        assert!(r.first_run_done);
        assert_eq!(r.usage.cleanups, 1, "tips don't come back after a reset");
        assert!(r.chime);
        assert!(r.style_notes.is_empty());
    }
}
