//! Hotkey parsing, display, conflict detection and the hold-to-talk state machine.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Normalised key name: "A".."Z", "0".."9", "F1".."F24", "Space", "Esc", ...
    pub key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Dictate,
    Cancel,
    CleanSelection,
    /// Quick switch: the next speech model ticked in Settings › Dictation.
    SwitchModel,
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Action::Dictate => "Dictate",
            Action::Cancel => "Cancel",
            Action::CleanSelection => "Clean up selection",
            Action::SwitchModel => "Switch speech model",
        }
    }
}

pub fn default_for(action: Action) -> Hotkey {
    match action {
        Action::Dictate => Hotkey::parse("Ctrl+Alt+Space").unwrap(),
        Action::Cancel => Hotkey::parse("Esc").unwrap(),
        Action::CleanSelection => Hotkey::parse("Ctrl+Alt+C").unwrap(),
        Action::SwitchModel => Hotkey::parse("Ctrl+Alt+M").unwrap(),
    }
}

/// Maps KeyboardEvent.code values and common spellings to our key names.
pub fn normalise_key(k: &str) -> Option<String> {
    let k = k.trim();
    if k.is_empty() {
        return None;
    }
    let lower = k.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("key") {
        if rest.len() == 1 && rest.chars().all(|c| c.is_ascii_alphabetic()) {
            return Some(rest.to_ascii_uppercase());
        }
    }
    if let Some(rest) = lower.strip_prefix("digit") {
        if rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit()) {
            return Some(rest.to_string());
        }
    }
    if k.len() == 1 {
        let c = k.chars().next().unwrap();
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
    }
    if let Some(n) = lower.strip_prefix('f') {
        if let Ok(n) = n.parse::<u8>() {
            if (1..=24).contains(&n) {
                return Some(format!("F{n}"));
            }
        }
    }
    let named = match lower.as_str() {
        "space" | " " | "spacebar" => "Space",
        "esc" | "escape" => "Esc",
        "enter" | "return" => "Enter",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "insert" | "ins" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" | "pgup" => "PageUp",
        "pagedown" | "pgdn" => "PageDown",
        "arrowup" | "up" => "Up",
        "arrowdown" | "down" => "Down",
        "arrowleft" | "left" => "Left",
        "arrowright" | "right" => "Right",
        "printscreen" | "prtsc" => "PrintScreen",
        "pause" => "Pause",
        "minus" | "-" => "Minus",
        "equal" | "=" => "Equal",
        "comma" | "," => "Comma",
        "period" | "." => "Period",
        "slash" | "/" => "Slash",
        "backslash" | "\\" => "Backslash",
        "semicolon" | ";" => "Semicolon",
        "quote" | "'" => "Quote",
        "backquote" | "`" => "Backquote",
        "bracketleft" | "[" => "BracketLeft",
        "bracketright" | "]" => "BracketRight",
        _ => return None,
    };
    Some(named.to_string())
}

fn is_modifier_name(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "ctrl"
            | "control"
            | "controlleft"
            | "controlright"
            | "alt"
            | "altleft"
            | "altright"
            | "option"
            | "shift"
            | "shiftleft"
            | "shiftright"
            | "win"
            | "super"
            | "meta"
            | "metaleft"
            | "metaright"
            | "cmd"
            | "command"
            | "os"
    )
}

impl Hotkey {
    pub fn parse(s: &str) -> Result<Hotkey, String> {
        let mut hk = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: String::new(),
        };
        // Split on '+', but allow a literal "+" key at the end ("Ctrl++" is not supported; use "Equal").
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hk.ctrl = true,
                "alt" | "option" => hk.alt = true,
                "shift" => hk.shift = true,
                "win" | "super" | "meta" | "cmd" | "command" => hk.win = true,
                _ => {
                    if !hk.key.is_empty() {
                        return Err(format!("\"{s}\" has more than one key"));
                    }
                    hk.key = normalise_key(part).ok_or_else(|| format!("Unknown key \"{part}\""))?;
                }
            }
        }
        if hk.key.is_empty() {
            return Err("Press a key together with the modifiers".into());
        }
        Ok(hk)
    }

    /// Builds a hotkey from what the settings page captured (modifier flags
    /// plus KeyboardEvent.code). Returns None while only modifiers are held.
    pub fn from_capture(ctrl: bool, alt: bool, shift: bool, win: bool, code: &str) -> Option<Hotkey> {
        if is_modifier_name(code) {
            return None;
        }
        Some(Hotkey {
            ctrl,
            alt,
            shift,
            win,
            key: normalise_key(code)?,
        })
    }

    pub fn has_modifier(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.win
    }

    /// Accelerator string for the global-hotkey crate.
    pub fn to_accelerator(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("control".into());
        }
        if self.alt {
            parts.push("alt".into());
        }
        if self.shift {
            parts.push("shift".into());
        }
        if self.win {
            parts.push("super".into());
        }
        let key = match self.key.as_str() {
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) => format!("Key{k}"),
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => format!("Digit{k}"),
            "Esc" => "Escape".into(),
            "Up" => "ArrowUp".into(),
            "Down" => "ArrowDown".into(),
            "Left" => "ArrowLeft".into(),
            "Right" => "ArrowRight".into(),
            k => k.to_string(),
        };
        parts.push(key);
        parts.join("+")
    }

    /// Validation for an action. Dictate and clean-up need a modifier (or
    /// a function key) so normal typing isn't swallowed.
    pub fn validate_for(&self, action: Action) -> Result<(), String> {
        let is_fn =
            self.key.starts_with('F') && self.key.len() > 1 && self.key[1..].chars().all(|c| c.is_ascii_digit());
        match action {
            Action::Cancel => Ok(()),
            _ if self.has_modifier() || is_fn || self.key == "Pause" => {
                if !self.has_modifier() || self.ctrl || self.alt || self.win || is_fn {
                    Ok(())
                } else {
                    Err("Shift alone isn't enough. Add Ctrl or Alt.".into())
                }
            }
            _ => Err("Add Ctrl, Alt or Win so normal typing isn't affected.".into()),
        }
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.win {
            parts.push("Win");
        }
        parts.push(&self.key);
        write!(f, "{}", parts.join("+"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    /// The conflicting combination, as shown to the user.
    pub combo: String,
    /// Who uses it, in plain words.
    pub used_by: String,
}

impl Conflict {
    pub fn message(&self) -> String {
        format!("{} is used by {}", self.combo, self.used_by)
    }
}

/// Shortcuts Windows reserves or that nearly every app uses.
const KNOWN: &[(&str, &str)] = &[
    ("Ctrl+Alt+Delete", "Windows (security screen)"),
    ("Ctrl+Shift+Esc", "Windows (Task Manager)"),
    ("Ctrl+Esc", "Windows (Start menu)"),
    ("Alt+Tab", "Windows (switch apps)"),
    ("Alt+Shift+Tab", "Windows (switch apps)"),
    ("Ctrl+Alt+Tab", "Windows (switch apps)"),
    ("Alt+Esc", "Windows (cycle windows)"),
    ("Alt+F4", "Windows (close window)"),
    ("Alt+Space", "Windows (window menu)"),
    ("Ctrl+F4", "most apps (close tab)"),
    ("Shift+F10", "Windows (context menu)"),
    ("PrintScreen", "Windows (screenshot)"),
    ("Alt+PrintScreen", "Windows (screenshot)"),
    ("Win+L", "Windows (lock)"),
    ("Win+D", "Windows (show desktop)"),
    ("Win+E", "Windows (File Explorer)"),
    ("Win+R", "Windows (Run)"),
    ("Win+V", "Windows (clipboard history)"),
    ("Win+H", "Windows (voice typing)"),
    ("Win+Period", "Windows (emoji panel)"),
    ("Win+Shift+S", "Windows (Snipping Tool)"),
    ("Win+Tab", "Windows (Task View)"),
    ("Win+Space", "Windows (switch keyboard layout)"),
    ("Win+A", "Windows (quick settings)"),
    ("Win+I", "Windows (Settings)"),
    ("Win+S", "Windows (search)"),
    ("Win+X", "Windows (quick link menu)"),
    ("Ctrl+Win+D", "Windows (new desktop)"),
    ("Ctrl+Win+Left", "Windows (switch desktop)"),
    ("Ctrl+Win+Right", "Windows (switch desktop)"),
    ("Ctrl+C", "most apps (copy)"),
    ("Ctrl+V", "most apps (paste)"),
    ("Ctrl+X", "most apps (cut)"),
    ("Ctrl+Z", "most apps (undo)"),
    ("Ctrl+Y", "most apps (redo)"),
    ("Ctrl+A", "most apps (select all)"),
    ("Ctrl+S", "most apps (save)"),
    ("Ctrl+F", "most apps (find)"),
    ("Ctrl+P", "most apps (print)"),
    ("Ctrl+N", "most apps (new)"),
    ("Ctrl+O", "most apps (open)"),
    ("Ctrl+W", "most apps (close tab)"),
    ("Ctrl+T", "most apps (new tab)"),
    ("Ctrl+Shift+T", "browsers (reopen tab)"),
    ("Ctrl+Space", "many apps (autocomplete / input switch)"),
    ("Ctrl+Shift+Space", "many apps (parameter hints)"),
    ("Ctrl+Alt+E", "keyboards with AltGr (€ sign)"),
    ("Ctrl+Alt+Q", "keyboards with AltGr (@ sign)"),
    ("Ctrl+Alt+2", "keyboards with AltGr (@ or ² sign)"),
    ("Ctrl+Alt+7", "keyboards with AltGr ({ bracket)"),
    ("Ctrl+Alt+8", "keyboards with AltGr ([ bracket)"),
    ("Ctrl+Alt+9", "keyboards with AltGr (] bracket)"),
    ("Ctrl+Alt+0", "keyboards with AltGr (} bracket)"),
];

/// Checks a hotkey against Windows / common shortcuts and Wirl's other
/// actions. Conflicts with other running apps are found when registering.
pub fn find_conflict(hk: &Hotkey, action: Action, others: &[(Action, Hotkey)]) -> Option<Conflict> {
    for (a, other) in others {
        if *a != action && other == hk {
            // Cancel is only active while recording, so it may share a key
            // with clean-up but never with Dictate.
            let clash = !(matches!(
                (action, *a),
                (Action::Cancel, Action::CleanSelection) | (Action::CleanSelection, Action::Cancel)
            ));
            if clash {
                return Some(Conflict {
                    combo: hk.to_string(),
                    used_by: format!("Wirl ({})", a.label()),
                });
            }
        }
    }
    if action == Action::Cancel && !hk.has_modifier() {
        // Cancel only listens while recording; plain keys are expected.
        return None;
    }
    for (combo, by) in KNOWN {
        if let Ok(k) = Hotkey::parse(combo) {
            if &k == hk {
                return Some(Conflict {
                    combo: hk.to_string(),
                    used_by: (*by).to_string(),
                });
            }
        }
    }
    if hk.win && !hk.ctrl && !hk.alt && hk.key.len() == 1 {
        return Some(Conflict {
            combo: hk.to_string(),
            used_by: "Windows (most Win+letter shortcuts are reserved)".into(),
        });
    }
    None
}

/// Conflict reported when Windows refuses to register a combination.
pub fn registration_conflict(hk: &Hotkey) -> Conflict {
    Conflict {
        combo: hk.to_string(),
        used_by: "another app".into(),
    }
}

/// State recording a new shortcut on the Hotkeys page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderState {
    Idle,
    /// Showing "Press new keys…".
    Listening(Action),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordResult {
    /// Saved. May carry a non-blocking conflict warning.
    Saved {
        action: Action,
        hotkey: Hotkey,
        warning: Option<Conflict>,
    },
    /// Still waiting (only modifiers pressed so far).
    Waiting,
    Rejected(String),
}

pub struct ShortcutRecorder {
    pub state: RecorderState,
}

impl Default for ShortcutRecorder {
    fn default() -> Self {
        ShortcutRecorder {
            state: RecorderState::Idle,
        }
    }
}

impl ShortcutRecorder {
    pub fn start(&mut self, action: Action) {
        self.state = RecorderState::Listening(action);
    }
    pub fn cancel(&mut self) {
        self.state = RecorderState::Idle;
    }
    /// Feed a key-down. The next full combination is saved.
    pub fn key_down(
        &mut self,
        ctrl: bool,
        alt: bool,
        shift: bool,
        win: bool,
        code: &str,
        others: &[(Action, Hotkey)],
    ) -> RecordResult {
        let RecorderState::Listening(action) = self.state.clone() else {
            return RecordResult::Rejected("Not recording".into());
        };
        let Some(hk) = Hotkey::from_capture(ctrl, alt, shift, win, code) else {
            return RecordResult::Waiting;
        };
        if let Err(e) = hk.validate_for(action) {
            return RecordResult::Rejected(e);
        }
        self.state = RecorderState::Idle;
        let warning = find_conflict(&hk, action, others);
        RecordResult::Saved {
            action,
            hotkey: hk,
            warning,
        }
    }
}

// ---------- hold-to-talk ----------

/// A press shorter than this is a tap (toggle); longer is hold-to-talk.
pub const HOLD_THRESHOLD_MS: u64 = 350;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerOutput {
    None,
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TState {
    Idle,
    /// Recording, key still down, not yet known if tap or hold.
    Pressed {
        at: u64,
    },
    /// Recording in toggle mode, waiting for the next press.
    Toggled,
    /// Stopped by a press; swallow the matching release.
    AwaitRelease,
}

/// Turns Dictate key presses/releases into start/stop decisions.
#[derive(Debug, Clone)]
pub struct DictateTrigger {
    pub hold_to_talk: bool,
    state: TState,
}

impl DictateTrigger {
    pub fn new(hold_to_talk: bool) -> Self {
        DictateTrigger {
            hold_to_talk,
            state: TState::Idle,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, TState::Pressed { .. } | TState::Toggled)
    }

    /// Recording ended for another reason (cancel, error, watchdog).
    pub fn reset(&mut self) {
        self.state = match self.state {
            TState::Pressed { .. } => TState::AwaitRelease,
            _ => TState::Idle,
        };
    }

    pub fn pressed(&mut self, now_ms: u64) -> TriggerOutput {
        match self.state {
            TState::Idle => {
                self.state = TState::Pressed { at: now_ms };
                TriggerOutput::Start
            }
            TState::Toggled => {
                self.state = TState::AwaitRelease;
                TriggerOutput::Stop
            }
            // Key repeat while held: ignore.
            TState::Pressed { .. } | TState::AwaitRelease => TriggerOutput::None,
        }
    }

    pub fn released(&mut self, now_ms: u64) -> TriggerOutput {
        match self.state {
            TState::Pressed { at } => {
                if self.hold_to_talk && now_ms.saturating_sub(at) >= HOLD_THRESHOLD_MS {
                    self.state = TState::Idle;
                    TriggerOutput::Stop
                } else {
                    self.state = TState::Toggled;
                    TriggerOutput::None
                }
            }
            TState::AwaitRelease => {
                self.state = TState::Idle;
                TriggerOutput::None
            }
            _ => TriggerOutput::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        let h = Hotkey::parse("ctrl+alt+space").unwrap();
        assert_eq!(h.to_string(), "Ctrl+Alt+Space");
        assert_eq!(h.to_accelerator(), "control+alt+Space");
        assert_eq!(
            Hotkey::parse("Ctrl+Alt+C").unwrap().to_accelerator(),
            "control+alt+KeyC"
        );
        assert_eq!(Hotkey::parse("Esc").unwrap().to_accelerator(), "Escape");
        assert!(Hotkey::parse("Ctrl+Alt").is_err());
        assert!(Hotkey::parse("Ctrl+A+B").is_err());
    }

    #[test]
    fn capture_from_keyboard_event_codes() {
        assert_eq!(
            Hotkey::from_capture(true, false, true, false, "KeyD")
                .unwrap()
                .to_string(),
            "Ctrl+Shift+D"
        );
        assert_eq!(
            Hotkey::from_capture(false, false, false, false, "F9")
                .unwrap()
                .to_string(),
            "F9"
        );
        assert!(Hotkey::from_capture(true, false, false, false, "ControlLeft").is_none());
        assert_eq!(
            Hotkey::from_capture(false, true, false, false, "Digit5")
                .unwrap()
                .to_string(),
            "Alt+5"
        );
    }

    #[test]
    fn recorder_waits_for_full_combo_then_saves() {
        let mut r = ShortcutRecorder::default();
        r.start(Action::Dictate);
        assert_eq!(
            r.key_down(true, false, false, false, "ControlLeft", &[]),
            RecordResult::Waiting
        );
        assert_eq!(r.state, RecorderState::Listening(Action::Dictate));
        match r.key_down(true, true, false, false, "KeyD", &[]) {
            RecordResult::Saved {
                hotkey,
                warning,
                action,
            } => {
                assert_eq!(action, Action::Dictate);
                assert_eq!(hotkey.to_string(), "Ctrl+Alt+D");
                assert!(warning.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(r.state, RecorderState::Idle);
    }

    #[test]
    fn recorder_rejects_plain_letter_for_dictate_but_allows_for_cancel() {
        let mut r = ShortcutRecorder::default();
        r.start(Action::Dictate);
        assert!(matches!(
            r.key_down(false, false, false, false, "KeyD", &[]),
            RecordResult::Rejected(_)
        ));
        r.start(Action::Cancel);
        assert!(matches!(
            r.key_down(false, false, false, false, "Backspace", &[]),
            RecordResult::Saved { .. }
        ));
    }

    #[test]
    fn detects_windows_and_app_conflicts() {
        let c = find_conflict(&Hotkey::parse("Ctrl+Shift+Esc").unwrap(), Action::Dictate, &[]).unwrap();
        assert_eq!(c.combo, "Ctrl+Shift+Esc");
        assert!(c.message().contains("Windows"));
        let c = find_conflict(&Hotkey::parse("Ctrl+C").unwrap(), Action::CleanSelection, &[]).unwrap();
        assert!(c.used_by.contains("copy"));
        assert!(find_conflict(&Hotkey::parse("Win+K").unwrap(), Action::Dictate, &[]).is_some());
        assert!(find_conflict(&Hotkey::parse("Ctrl+Alt+Space").unwrap(), Action::Dictate, &[]).is_none());
    }

    #[test]
    fn detects_conflict_with_other_wirl_action() {
        let others = vec![(Action::CleanSelection, Hotkey::parse("Ctrl+Alt+C").unwrap())];
        let c = find_conflict(&Hotkey::parse("Ctrl+Alt+C").unwrap(), Action::Dictate, &others).unwrap();
        assert!(c.used_by.contains("Clean up selection"));
        assert_eq!(
            registration_conflict(&Hotkey::parse("Ctrl+Alt+K").unwrap()).message(),
            "Ctrl+Alt+K is used by another app"
        );
    }

    #[test]
    fn hold_to_talk_records_until_release() {
        let mut t = DictateTrigger::new(true);
        assert_eq!(t.pressed(0), TriggerOutput::Start);
        assert_eq!(t.pressed(100), TriggerOutput::None); // repeat
        assert_eq!(t.released(2_000), TriggerOutput::Stop);
        assert!(!t.is_active());
    }

    #[test]
    fn quick_tap_toggles() {
        let mut t = DictateTrigger::new(true);
        assert_eq!(t.pressed(0), TriggerOutput::Start);
        assert_eq!(t.released(120), TriggerOutput::None);
        assert!(t.is_active());
        assert_eq!(t.pressed(5_000), TriggerOutput::Stop);
        assert_eq!(t.released(5_100), TriggerOutput::None);
        assert_eq!(t.pressed(6_000), TriggerOutput::Start);
    }

    #[test]
    fn hold_off_always_toggles() {
        let mut t = DictateTrigger::new(false);
        assert_eq!(t.pressed(0), TriggerOutput::Start);
        assert_eq!(t.released(3_000), TriggerOutput::None);
        assert_eq!(t.pressed(4_000), TriggerOutput::Stop);
    }

    #[test]
    fn reset_while_held_swallows_release() {
        let mut t = DictateTrigger::new(true);
        t.pressed(0);
        t.reset();
        assert_eq!(t.released(2_000), TriggerOutput::None);
        assert_eq!(t.pressed(3_000), TriggerOutput::Start);
    }
}
