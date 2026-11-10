//! Personal dictionary: entries, matching, import/export and sync merge.
//!
//! The entry JSON format is shared with the Android app and the Wi-Fi sync
//! protocol (see docs/sync-protocol.md). Field names are camelCase on the wire.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Tombstones older than this are purged.
pub const TOMBSTONE_MAX_AGE_MS: i64 = 90 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub term: String,
    #[serde(default)]
    pub sounds_like: Vec<String>,
    #[serde(default)]
    pub note: String,
    pub updated_at: i64,
    #[serde(default)]
    pub deleted: bool,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Entry {
    pub fn new(term: &str, sounds_like: Vec<String>, note: &str) -> Self {
        Entry {
            id: uuid::Uuid::new_v4().to_string(),
            term: term.trim().to_string(),
            sounds_like: clean_list(sounds_like),
            note: note.trim().to_string(),
            updated_at: now_ms(),
            deleted: false,
        }
    }
}

fn clean_list(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        let t = s.trim();
        if !t.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    }
    out
}

/// Splits a comma-separated "sounds like" string.
pub fn split_sounds_like(s: &str) -> Vec<String> {
    clean_list(s.split(',').map(|x| x.to_string()).collect())
}

/// The whole dictionary, including tombstones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dictionary {
    pub entries: Vec<Entry>,
}

impl Dictionary {
    /// Live entries, alphabetical (case-insensitive).
    pub fn visible(&self) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self.entries.iter().filter(|e| !e.deleted).collect();
        v.sort_by(|a, b| {
            a.term
                .to_lowercase()
                .cmp(&b.term.to_lowercase())
                .then(a.term.cmp(&b.term))
        });
        v
    }

    pub fn count(&self) -> usize {
        self.entries.iter().filter(|e| !e.deleted).count()
    }

    /// Live entries whose term, sounds-like or note contains `query`.
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let q = query.trim().to_lowercase();
        self.visible()
            .into_iter()
            .filter(|e| {
                q.is_empty()
                    || e.term.to_lowercase().contains(&q)
                    || e.note.to_lowercase().contains(&q)
                    || e.sounds_like.iter().any(|s| s.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub fn add(&mut self, term: &str, sounds_like: Vec<String>, note: &str) -> Result<Entry, String> {
        let term = term.trim();
        if term.is_empty() {
            return Err("Term can't be empty".into());
        }
        if self.find_term(term).is_some() {
            return Err(format!("\"{term}\" is already in your dictionary"));
        }
        let e = Entry::new(term, sounds_like, note);
        self.entries.push(e.clone());
        Ok(e)
    }

    pub fn edit(&mut self, id: &str, term: &str, sounds_like: Vec<String>, note: &str) -> Result<(), String> {
        let term = term.trim();
        if term.is_empty() {
            return Err("Term can't be empty".into());
        }
        if let Some(other) = self.find_term(term) {
            if other.id != id {
                return Err(format!("\"{term}\" is already in your dictionary"));
            }
        }
        let e = self
            .entries
            .iter_mut()
            .find(|e| e.id == id && !e.deleted)
            .ok_or("Entry not found")?;
        e.term = term.to_string();
        e.sounds_like = clean_list(sounds_like);
        e.note = note.trim().to_string();
        e.updated_at = now_ms().max(e.updated_at + 1);
        Ok(())
    }

    /// Deleting sets a tombstone so the deletion syncs.
    pub fn delete(&mut self, id: &str) -> bool {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.deleted = true;
            e.updated_at = now_ms().max(e.updated_at + 1);
            true
        } else {
            false
        }
    }

    pub fn find_term(&self, term: &str) -> Option<&Entry> {
        let t = term.trim().to_lowercase();
        self.entries
            .iter()
            .find(|e| !e.deleted && e.term.trim().to_lowercase() == t)
    }

    /// Removes tombstones older than 90 days.
    pub fn purge_tombstones(&mut self, now: i64) {
        self.entries
            .retain(|e| !(e.deleted && now - e.updated_at > TOMBSTONE_MAX_AGE_MS));
    }

    /// Entries that plausibly match the transcript. Errs on the side of
    /// including an entry when unsure.
    pub fn matching(&self, transcript: &str) -> Vec<&Entry> {
        let words = tokenize(transcript);
        let lower = transcript.to_lowercase();
        self.visible()
            .into_iter()
            .filter(|e| entry_matches(e, &lower, &words))
            .collect()
    }

    /// Like [`matching`](Self::matching), for a transcript from the Cantonese
    /// model. That model writes simplified characters while people type their
    /// entries in traditional ones, so entries written in Chinese characters
    /// are always included (Claude checks whether they apply).
    pub fn matching_cantonese(&self, transcript: &str) -> Vec<&Entry> {
        let words = tokenize(transcript);
        let lower = transcript.to_lowercase();
        self.visible()
            .into_iter()
            .filter(|e| entry_matches(e, &lower, &words) || has_han(e))
            .collect()
    }

    /// Imports JSON (sync entry format, or simple {term, soundsLike, note}
    /// objects) and merges by term. Returns how many terms were added/updated.
    pub fn import_json(&mut self, json: &str) -> Result<ImportSummary, String> {
        let items = parse_import(json)?;
        let mut summary = ImportSummary::default();
        for item in items {
            let term = item.term.trim();
            if term.is_empty() {
                continue;
            }
            let now = now_ms();
            if let Some(existing) = self
                .entries
                .iter_mut()
                .find(|e| !e.deleted && e.term.trim().eq_ignore_ascii_case(term))
            {
                let mut changed = false;
                for s in item.sounds_like {
                    let s = s.trim();
                    if !s.is_empty() && !existing.sounds_like.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                        existing.sounds_like.push(s.to_string());
                        changed = true;
                    }
                }
                if existing.note.trim().is_empty() && !item.note.trim().is_empty() {
                    existing.note = item.note.trim().to_string();
                    changed = true;
                }
                if changed {
                    existing.updated_at = now.max(existing.updated_at + 1);
                    summary.updated += 1;
                } else {
                    summary.unchanged += 1;
                }
            } else {
                let mut e = Entry::new(term, item.sounds_like, &item.note);
                if let Some(id) = item.id.filter(|id| !self.entries.iter().any(|x| &x.id == id)) {
                    e.id = id;
                }
                self.entries.push(e);
                summary.added += 1;
            }
        }
        Ok(summary)
    }

    /// Exports live entries in the sync entry format.
    pub fn export_json(&self) -> String {
        let v: Vec<&Entry> = self.visible();
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| "[]".into())
    }

    /// Merges a remote full dictionary into this one (sync). Both sides end
    /// up identical when each applies the other's list.
    pub fn merge_remote(&mut self, remote: &[Entry]) -> MergeSummary {
        let merged = merge_entries(&self.entries, remote);
        let mut summary = MergeSummary::default();
        let before: HashMap<&str, &Entry> = self.entries.iter().map(|e| (e.id.as_str(), e)).collect();
        for e in &merged {
            match before.get(e.id.as_str()) {
                None => summary.received += 1,
                Some(old) if *old != e => summary.received += 1,
                _ => {}
            }
        }
        self.entries = merged;
        summary
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct ImportSummary {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct MergeSummary {
    pub received: usize,
}

/// Returns the winner between two versions of the same entry id.
/// Newer `updatedAt` wins; on a tie, the deleted one wins.
pub fn pick_winner<'a>(a: &'a Entry, b: &'a Entry) -> &'a Entry {
    if a.updated_at != b.updated_at {
        return if a.updated_at > b.updated_at { a } else { b };
    }
    if a.deleted != b.deleted {
        return if a.deleted { a } else { b };
    }
    // Full tie: choose deterministically so both sides agree regardless of order.
    let ka = serde_json::to_string(a).unwrap_or_default();
    let kb = serde_json::to_string(b).unwrap_or_default();
    if ka >= kb {
        a
    } else {
        b
    }
}

/// Per-id merge of two full dictionaries (including tombstones). Output is
/// sorted by id so it is identical on both sides.
pub fn merge_entries(local: &[Entry], remote: &[Entry]) -> Vec<Entry> {
    let mut map: HashMap<String, Entry> = HashMap::new();
    for e in local.iter().chain(remote.iter()) {
        match map.get(&e.id) {
            Some(existing) => {
                let w = pick_winner(existing, e).clone();
                map.insert(e.id.clone(), w);
            }
            None => {
                map.insert(e.id.clone(), e.clone());
            }
        }
    }
    let mut out: Vec<Entry> = map.into_values().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

struct ImportItem {
    id: Option<String>,
    term: String,
    sounds_like: Vec<String>,
    note: String,
}

fn parse_import(json: &str) -> Result<Vec<ImportItem>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("That file isn't valid JSON ({e})"))?;
    let arr = match &v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => match o.get("entries") {
            Some(Value::Array(a)) => a.clone(),
            _ => return Err("Expected a list of terms".into()),
        },
        _ => return Err("Expected a list of terms".into()),
    };
    let mut out = Vec::new();
    for item in arr {
        let Value::Object(o) = item else { continue };
        // Tombstones in a sync-format file are not imported.
        if o.get("deleted").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let term = o.get("term").and_then(Value::as_str).unwrap_or("").to_string();
        let sounds_like = match o.get("soundsLike").or_else(|| o.get("sounds_like")) {
            Some(Value::String(s)) => split_sounds_like(s),
            Some(Value::Array(a)) => clean_list(a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()),
            _ => vec![],
        };
        let note = o.get("note").and_then(Value::as_str).unwrap_or("").to_string();
        let id = o
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| uuid::Uuid::parse_str(s).is_ok())
            .map(|s| s.to_string());
        out.push(ImportItem {
            id,
            term,
            sounds_like,
            note,
        });
    }
    Ok(out)
}

// ---------- matching ----------

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase().replace('\'', ""))
        .collect()
}

/// A simple Soundex-style phonetic key with no length limit. The first
/// letter is coded too, so "Cara" and "Kara" share a key.
pub fn phonetic_key(word: &str) -> String {
    let w: Vec<char> = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    let mut s = String::new();
    // Digraph normalisation.
    let mut i = 0;
    let mut chars = Vec::new();
    while i < w.len() {
        let c = w[i];
        let n = w.get(i + 1).copied();
        match (c, n) {
            ('p', Some('h')) => {
                chars.push('f');
                i += 2;
            }
            ('c', Some('k')) => {
                chars.push('k');
                i += 2;
            }
            ('w', Some('r')) if i == 0 => {
                chars.push('r');
                i += 2;
            }
            ('k', Some('n')) if i == 0 => {
                chars.push('n');
                i += 2;
            }
            _ => {
                chars.push(c);
                i += 1;
            }
        }
    }
    let code = |c: char| -> Option<char> {
        match c {
            'b' | 'f' | 'p' | 'v' => Some('1'),
            'c' | 'g' | 'j' | 'k' | 'q' | 's' | 'x' | 'z' => Some('2'),
            'd' | 't' => Some('3'),
            'l' => Some('4'),
            'm' | 'n' => Some('5'),
            'r' => Some('6'),
            _ => None, // vowels, h, w, y
        }
    };
    let mut last: Option<char> = None;
    for (idx, c) in chars.iter().enumerate() {
        match code(*c) {
            Some(d) => {
                if last != Some(d) {
                    s.push(d);
                }
                last = Some(d);
            }
            None => {
                if idx == 0 {
                    s.push('0'); // leading vowel
                }
                // h/w don't separate equal codes; vowels do.
                if !matches!(c, 'h' | 'w') {
                    last = None;
                }
            }
        }
    }
    s
}

fn keys_match(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short.len() >= 2 && long.len() - short.len() <= 1 && long.starts_with(short)
}

/// Whether the entry's term or wrong spellings contain Chinese characters.
fn has_han(e: &Entry) -> bool {
    std::iter::once(&e.term)
        .chain(e.sounds_like.iter())
        .any(|p| p.chars().any(is_han))
}

/// CJK Unified Ideographs (with extension A and compatibility ideographs).
pub fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{3134F}')
}

fn entry_matches(e: &Entry, lower_text: &str, words: &[String]) -> bool {
    let phrases = std::iter::once(&e.term).chain(e.sounds_like.iter());
    for p in phrases {
        let p_low = p.trim().to_lowercase();
        if p_low.is_empty() {
            continue;
        }
        if lower_text.contains(&p_low) {
            return true;
        }
        let p_words = tokenize(&p_low);
        if p_words.is_empty() {
            continue;
        }
        let p_key: String = p_words.iter().map(|w| phonetic_key(w)).collect();
        // Compare against windows of 1..=3 transcript words, plus windows the
        // same length as the phrase.
        let max_n = 3.max(p_words.len());
        for n in 1..=max_n {
            if n > words.len() {
                break;
            }
            for win in words.windows(n) {
                let joined: String = win.concat();
                if joined == p_words.concat() {
                    return true;
                }
                let k: String = win.iter().map(|w| phonetic_key(w)).collect();
                if keys_match(&k, &p_key) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cantonese_matching_includes_chinese_entries() {
        let mut d = Dictionary::default();
        d.add("陳太", vec!["陈太".into()], "").unwrap();
        d.add("Sela", vec!["salad".into()], "").unwrap();
        d.add("Wirl", vec![], "").unwrap();
        let raw = "听日同 SALAD 去饮茶";
        let terms = |v: Vec<&Entry>| v.iter().map(|e| e.term.clone()).collect::<Vec<_>>();
        assert_eq!(terms(d.matching(raw)), ["Sela"]);
        let mut c = terms(d.matching_cantonese(raw));
        c.sort();
        assert_eq!(c, ["Sela", "陳太"]);
        // Chinese text alone never matches Latin entries by sound.
        assert!(d.matching("我哋喺度").is_empty());
    }

    fn dict(entries: &[(&str, &[&str], &str)]) -> Dictionary {
        let mut d = Dictionary::default();
        for (t, s, n) in entries {
            d.add(t, s.iter().map(|x| x.to_string()).collect(), n).unwrap();
        }
        d
    }

    #[test]
    fn matching_includes_listed_mishearing_and_phonetic() {
        let d = dict(&[
            ("Sela", &["salad"], "my daughter"),
            ("Kubernetes", &[], ""),
            ("Anthropic", &[], ""),
        ]);
        let m: Vec<&str> = d
            .matching("I'll pick up salad from school")
            .iter()
            .map(|e| e.term.as_str())
            .collect();
        assert_eq!(m, vec!["Sela"]);
        let m: Vec<&str> = d
            .matching("deploy it to cooper netties")
            .iter()
            .map(|e| e.term.as_str())
            .collect();
        assert!(m.contains(&"Kubernetes"), "{m:?}");
        let m = d.matching("the weather is nice today");
        assert!(m.is_empty(), "{:?}", m);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let d = dict(&[("iPhone", &[], "")]);
        assert_eq!(d.matching("my IPHONE broke").len(), 1);
    }

    #[test]
    fn phonetic_keys() {
        assert_eq!(phonetic_key("Cara"), phonetic_key("Kara"));
        assert_eq!(phonetic_key("Philip"), phonetic_key("Filip"));
        assert!(keys_match(&phonetic_key("salad"), &phonetic_key("Sela")));
        assert!(!keys_match(&phonetic_key("the"), &phonetic_key("Sela")));
    }

    #[test]
    fn alphabetical_and_search() {
        let d = dict(&[("zeta", &[], ""), ("Alpha", &[], "greek"), ("beta", &["bayta"], "")]);
        let terms: Vec<&str> = d.visible().iter().map(|e| e.term.as_str()).collect();
        assert_eq!(terms, vec!["Alpha", "beta", "zeta"]);
        assert_eq!(d.search("GREEK").len(), 1);
        assert_eq!(d.search("bayt").len(), 1);
        assert_eq!(d.search("").len(), 3);
    }

    #[test]
    fn delete_makes_tombstone_and_purges_after_90_days() {
        let mut d = dict(&[("Sela", &[], "")]);
        let id = d.entries[0].id.clone();
        let before = d.entries[0].updated_at;
        assert!(d.delete(&id));
        assert!(d.entries[0].deleted);
        assert!(d.entries[0].updated_at > before);
        assert_eq!(d.count(), 0);
        d.purge_tombstones(d.entries[0].updated_at + TOMBSTONE_MAX_AGE_MS - 1);
        assert_eq!(d.entries.len(), 1);
        d.purge_tombstones(d.entries[0].updated_at + TOMBSTONE_MAX_AGE_MS + 1);
        assert!(d.entries.is_empty());
    }

    #[test]
    fn import_simple_format_with_comma_string() {
        let mut d = Dictionary::default();
        let s = d
            .import_json(r#"[{"term":"Sela","soundsLike":"salad, sella","note":"daughter"},{"term":"Wirl","soundsLike":["whirl","world"]}]"#)
            .unwrap();
        assert_eq!(s.added, 2);
        let sela = d.find_term("sela").unwrap();
        assert_eq!(sela.sounds_like, vec!["salad", "sella"]);
        assert_eq!(d.find_term("Wirl").unwrap().sounds_like, vec!["whirl", "world"]);
    }

    #[test]
    fn import_sync_format_and_merge_by_term() {
        let mut d = dict(&[("Sela", &["salad"], "")]);
        let json = r#"{"entries":[
            {"id":"6f1c1a2e-3d4b-4c5d-8e9f-0a1b2c3d4e5f","term":"sela","soundsLike":["Salad","cellar"],"note":"daughter","updatedAt":1,"deleted":false},
            {"id":"7f1c1a2e-3d4b-4c5d-8e9f-0a1b2c3d4e5f","term":"Gone","soundsLike":[],"note":"","updatedAt":1,"deleted":true}
        ]}"#;
        let s = d.import_json(json).unwrap();
        assert_eq!(s.updated, 1);
        assert_eq!(s.added, 0);
        assert_eq!(d.count(), 1);
        let e = d.find_term("Sela").unwrap();
        assert_eq!(e.sounds_like, vec!["salad", "cellar"]);
        assert_eq!(e.note, "daughter");
    }

    #[test]
    fn import_rejects_garbage() {
        let mut d = Dictionary::default();
        assert!(d.import_json("not json").is_err());
        assert!(d.import_json("42").is_err());
    }

    #[test]
    fn export_roundtrip() {
        let d = dict(&[("Sela", &["salad"], "n")]);
        let mut d2 = Dictionary::default();
        d2.import_json(&d.export_json()).unwrap();
        assert_eq!(d2.visible()[0].term, "Sela");
        assert_eq!(d2.visible()[0].id, d.visible()[0].id);
    }

    fn e(id: &str, term: &str, at: i64, deleted: bool) -> Entry {
        Entry {
            id: id.into(),
            term: term.into(),
            sounds_like: vec![],
            note: String::new(),
            updated_at: at,
            deleted,
        }
    }

    #[test]
    fn merge_newer_wins_and_tie_deleted_wins() {
        let local = vec![e("a", "old", 1, false), e("b", "keep", 5, false), e("c", "x", 3, false)];
        let remote = vec![
            e("a", "new", 2, false),
            e("b", "stale", 4, false),
            e("c", "x", 3, true),
            e("d", "fresh", 1, false),
        ];
        let m = merge_entries(&local, &remote);
        let get = |id: &str| m.iter().find(|x| x.id == id).unwrap().clone();
        assert_eq!(get("a").term, "new");
        assert_eq!(get("b").term, "keep");
        assert!(get("c").deleted);
        assert_eq!(get("d").term, "fresh");
        // Symmetric: both sides end identical.
        assert_eq!(merge_entries(&remote, &local), m);
    }
}
