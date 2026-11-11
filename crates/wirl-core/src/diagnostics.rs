//! Diagnostics log: the last 20 entries with per-stage timings and, like the
//! Android app, the text at each step (what the speech model heard, what
//! Claude returned, what was inserted). Kept in memory only, never saved or
//! sent anywhere; "Copy report" puts it on the clipboard when the user asks.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const MAX_ENTRIES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagKind {
    Dictation,
    CleanSelection,
    Sync,
    SidecarRestart,
    Error,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub name: String,
    pub ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagEntry {
    pub at_ms: i64,
    pub kind: DiagKind,
    pub summary: String,
    pub stages: Vec<Stage>,
    pub model: Option<String>,
    pub thinking: Option<bool>,
    pub insert_method: Option<String>,
    pub error: Option<String>,
    /// What the speech model heard (after filtering), or the selected text
    /// for a clean-up.
    #[serde(default)]
    pub heard: Option<String>,
    /// What Claude returned.
    #[serde(default)]
    pub cleaned: Option<String>,
    /// What was inserted.
    #[serde(default)]
    pub inserted: Option<String>,
    /// Claude's connection: "warm" (reused, open from before) or "new".
    #[serde(default)]
    pub connection: Option<String>,
}

impl DiagEntry {
    pub fn new(kind: DiagKind, summary: impl Into<String>) -> Self {
        DiagEntry {
            at_ms: crate::dictionary::now_ms(),
            kind,
            summary: summary.into(),
            stages: vec![],
            model: None,
            thinking: None,
            insert_method: None,
            error: None,
            heard: None,
            cleaned: None,
            inserted: None,
            connection: None,
        }
    }
    pub fn stage(mut self, name: impl Into<String>, ms: u64) -> Self {
        self.stages.push(Stage { name: name.into(), ms });
        self
    }
    pub fn push_stage(&mut self, name: impl Into<String>, ms: u64) {
        self.stages.push(Stage { name: name.into(), ms });
    }
}

#[derive(Debug, Default, Clone)]
pub struct Diagnostics {
    entries: VecDeque<DiagEntry>,
    pub sidecar_restarts: u32,
}

impl Diagnostics {
    pub fn push(&mut self, e: DiagEntry) {
        if e.kind == DiagKind::SidecarRestart {
            self.sidecar_restarts += 1;
        }
        self.entries.push_front(e);
        self.entries.truncate(MAX_ENTRIES);
    }
    /// Newest first.
    pub fn entries(&self) -> Vec<DiagEntry> {
        self.entries.iter().cloned().collect()
    }
}

/// Longest text kept per field in the report.
const REPORT_TEXT_MAX: usize = 2_000;

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= REPORT_TEXT_MAX {
        s.to_string()
    } else {
        let cut: String = s.chars().take(REPORT_TEXT_MAX).collect();
        format!("{cut}… (cut)")
    }
}

/// A plain-text report for pasting into a message: `header` lines (version,
/// settings, engine status; never the API key) then each entry, newest first.
/// `time` formats an entry's timestamp.
pub fn report(header: &[String], entries: &[DiagEntry], time: impl Fn(i64) -> String) -> String {
    let mut out = String::from("Wirl for Windows diagnostics\n");
    for h in header {
        out.push_str(h);
        out.push('\n');
    }
    if entries.is_empty() {
        out.push_str("\nNo attempts yet. Dictate once, then copy the report again.\n");
        return out;
    }
    for e in entries {
        out.push_str(&format!("\n--- {} · {} ({:?})\n", time(e.at_ms), e.summary, e.kind));
        if !e.stages.is_empty() {
            let stages: Vec<String> = e.stages.iter().map(|s| format!("{}: {} ms", s.name, s.ms)).collect();
            out.push_str(&format!("Timing: {}\n", stages.join(" · ")));
        }
        if let Some(m) = &e.model {
            out.push_str(&format!("Model: {m}\n"));
        }
        if let Some(t) = e.thinking {
            out.push_str(&format!("Thinking: {}\n", if t { "on" } else { "off" }));
        }
        if let Some(c) = &e.connection {
            out.push_str(&format!("Claude connection: {c}\n"));
        }
        if let Some(i) = &e.insert_method {
            out.push_str(&format!("Insert: {i}\n"));
        }
        for (label, text) in [
            ("Heard", &e.heard),
            ("Claude returned", &e.cleaned),
            ("Inserted", &e.inserted),
        ] {
            if let Some(t) = text {
                out.push_str(&format!("{label}: {}\n", clip(t)));
            }
        }
        if let Some(err) = &e.error {
            out.push_str(&format!("Error: {err}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_lists_entries_with_text_and_timing() {
        let mut e = DiagEntry::new(DiagKind::Dictation, "3.2 s dictation, 1 chunks");
        e.at_ms = 0;
        e.push_stage("Whisper chunk 1", 410);
        e.push_stage("Stop to insert", 1_250);
        e.model = Some("English · Fast + claude-haiku-5-5".into());
        e.heard = Some("um send it tomorrow".into());
        e.cleaned = Some("Send it tomorrow.".into());
        e.inserted = Some("Send it tomorrow.".into());
        e.connection = Some("warm".into());
        let r = report(&["Version: 0.1.9".into(), "API key: saved".into()], &[e], |_| {
            "12:00:00".into()
        });
        assert!(r.starts_with("Wirl for Windows diagnostics\nVersion: 0.1.9\nAPI key: saved\n"));
        assert!(r.contains("--- 12:00:00 · 3.2 s dictation, 1 chunks (Dictation)"));
        assert!(r.contains("Timing: Whisper chunk 1: 410 ms · Stop to insert: 1250 ms"));
        assert!(
            r.contains("Heard: um send it tomorrow\nClaude returned: Send it tomorrow.\nInserted: Send it tomorrow.")
        );
        assert!(r.contains("Claude connection: warm"));
        assert!(!r.contains("Error:"));
    }

    #[test]
    fn report_without_entries_and_long_text() {
        let r = report(&[], &[], |_| String::new());
        assert!(r.contains("No attempts yet"));
        let mut e = DiagEntry::new(DiagKind::Dictation, "long");
        e.heard = Some("a".repeat(3_000));
        let r = report(&[], &[e], |_| String::new());
        assert!(r.contains("… (cut)"));
        assert!(r.len() < 2_200);
    }

    #[test]
    fn keeps_last_20_newest_first() {
        let mut d = Diagnostics::default();
        for i in 0..25 {
            d.push(DiagEntry::new(DiagKind::Info, format!("e{i}")));
        }
        let e = d.entries();
        assert_eq!(e.len(), 20);
        assert_eq!(e[0].summary, "e24");
        assert_eq!(e[19].summary, "e5");
    }

    #[test]
    fn counts_restarts() {
        let mut d = Diagnostics::default();
        d.push(DiagEntry::new(DiagKind::SidecarRestart, "crash").stage("restart", 300));
        assert_eq!(d.sidecar_restarts, 1);
    }
}
