//! Diagnostics log: the last 20 entries with per-stage timings.

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

#[cfg(test)]
mod tests {
    use super::*;

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
