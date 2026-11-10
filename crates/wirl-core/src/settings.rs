//! User settings (autosaved JSON). The API key, dictionary and pairings are
//! stored separately so "Reset to defaults" never touches them.

use crate::hotkey::{default_for, Action, Hotkey};
use crate::onboarding::Usage;
use crate::pill::{PillPosition, PillSize};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ModelId {
    /// English · Fast: Whisper base.en, quantized.
    #[default]
    Fast,
    /// English · Accurate: Whisper small.en.
    Accurate,
    /// SenseVoice (sherpa-onnx), Cantonese with English mixed in.
    Cantonese,
    /// SenseVoice (the same files), Mandarin with English mixed in.
    Mandarin,
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

    /// Settings format version (see [`SETTINGS_VERSION`] and `migrate`).
    pub version: u32,

    // Dictation. The model decides the language (models::SpeechLanguage).
    pub model: ModelId,
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
            version: SETTINGS_VERSION,
            model: ModelId::Fast,
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

    /// Reads saved settings, upgrading older formats first (see `migrate`).
    pub fn from_json(s: &str) -> Settings {
        match serde_json::from_str::<serde_json::Value>(s) {
            Ok(mut v) => {
                migrate(&mut v);
                serde_json::from_value(v).unwrap_or_default()
            }
            Err(_) => Settings::default(),
        }
    }
}

/// Current settings format.
/// - 1 (no `version`): a separate `language` setting and a Multilingual model.
/// - 2: one model per language; Multilingual and `language` removed.
pub const SETTINGS_VERSION: u32 = 2;

/// The Whisper file of the removed Multilingual model. The app deletes it
/// (and any partial download) at start-up.
pub const REMOVED_MULTILINGUAL_FILE: &str = "ggml-base.bin";

/// Upgrades saved settings to [`SETTINGS_VERSION`] in place. A model that no
/// longer exists becomes English · Fast, so nobody is left with an invalid
/// choice (which would otherwise reset every setting).
fn migrate(v: &mut serde_json::Value) {
    let Some(obj) = v.as_object_mut() else { return };
    let version = obj.get("version").and_then(|x| x.as_u64()).unwrap_or(1);
    if version < 2 {
        obj.remove("language");
        if obj.get("model").and_then(|m| m.as_str()) == Some("multilingual") {
            obj.insert("model".into(), "fast".into());
        }
    }
    // Any unknown model name (e.g. from a newer version) also falls back.
    let known = obj
        .get("model")
        .map(|m| serde_json::from_value::<ModelId>(m.clone()).is_ok())
        .unwrap_or(true);
    if !known {
        obj.insert("model".into(), "fast".into());
    }
    obj.insert("version".into(), SETTINGS_VERSION.into());
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
    fn multilingual_users_move_to_english_fast() {
        let old = r#"{"model":"multilingual","language":"de","chime":false,"holdToTalk":false}"#;
        let s = Settings::from_json(old);
        assert_eq!(s.model, ModelId::Fast);
        assert_eq!(s.version, SETTINGS_VERSION);
        // Everything else is kept.
        assert!(!s.chime && !s.hold_to_talk);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("language") && !json.contains("multilingual"));
    }

    #[test]
    fn old_settings_keep_their_model_and_unknown_models_fall_back() {
        assert_eq!(
            Settings::from_json(r#"{"model":"accurate","language":"en"}"#).model,
            ModelId::Accurate
        );
        assert_eq!(
            Settings::from_json(r#"{"model":"cantonese"}"#).model,
            ModelId::Cantonese
        );
        assert_eq!(
            Settings::from_json(r#"{"version":2,"model":"mandarin"}"#).model,
            ModelId::Mandarin
        );
        let s = Settings::from_json(r#"{"version":2,"model":"klingon","chime":false}"#);
        assert_eq!(s.model, ModelId::Fast);
        assert!(!s.chime);
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
