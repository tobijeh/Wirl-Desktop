//! Manual cleanup quality test. Runs the real cleanup request (same prompt
//! builder and client the app uses) against tests/quality/cases.json.
//!
//! Run it yourself with your own key; it is never run in GitHub Actions and
//! the key is only read from the environment, never saved:
//!
//!   PowerShell:  $env:ANTHROPIC_API_KEY = "sk-ant-..."; cargo run -p wirl-core --example cleanup_quality
//!   bash:        ANTHROPIC_API_KEY=sk-ant-... cargo run -p wirl-core --example cleanup_quality
//!
//! Optional argument: a substring to run only matching cases.

use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use wirl_core::claude::ClaudeClient;
use wirl_core::dictionary::{Dictionary, Entry};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DictItem {
    term: String,
    #[serde(default)]
    sounds_like: Vec<String>,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    input: String,
    #[serde(default)]
    style: String,
    #[serde(default)]
    dictionary: Vec<DictItem>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    contains_any: Vec<String>,
    #[serde(default)]
    not_contains: Vec<String>,
    #[serde(default)]
    equals: Option<String>,
    #[serde(default)]
    ends_with: Option<String>,
    #[serde(default)]
    max_count: HashMap<String, usize>,
}

fn check(c: &Case, out: &str) -> Vec<String> {
    let lower = out.to_lowercase();
    let mut fails = Vec::new();
    if let Some(eq) = &c.equals {
        if out.trim() != eq.trim() {
            fails.push(format!("expected exactly {eq:?}"));
        }
    }
    for s in &c.contains {
        if !lower.contains(&s.to_lowercase()) {
            fails.push(format!("missing {s:?}"));
        }
    }
    if !c.contains_any.is_empty() && !c.contains_any.iter().any(|s| out.contains(s.as_str())) {
        fails.push(format!("missing one of {:?}", c.contains_any));
    }
    for s in &c.not_contains {
        if lower.contains(&s.to_lowercase()) {
            fails.push(format!("should not contain {s:?}"));
        }
    }
    if let Some(e) = &c.ends_with {
        if !out.trim_end().ends_with(e.as_str()) {
            fails.push(format!("should end with {e:?}"));
        }
    }
    for (s, max) in &c.max_count {
        let n = lower.matches(&s.to_lowercase()).count();
        if n > *max {
            fails.push(format!("{s:?} appears {n} times (max {max})"));
        }
    }
    fails
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => {
            eprintln!("Set ANTHROPIC_API_KEY in your shell first (it is not saved anywhere).");
            std::process::exit(2);
        }
    };
    let filter = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/quality/cases.json");
    let cases: Vec<Case> =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read cases.json")).expect("parse cases.json");
    let client = ClaudeClient::new();
    let (mut pass, mut fail) = (0, 0);
    for c in cases.iter().filter(|c| c.name.to_lowercase().contains(&filter)) {
        let mut dict = Dictionary::default();
        for d in &c.dictionary {
            dict.add(&d.term, d.sounds_like.clone(), &d.note)
                .expect("dictionary entry");
        }
        // Same relevance filter the app uses.
        let entries: Vec<&Entry> = dict.matching(&c.input);
        match client
            .cleanup(&key, &c.input, &c.style, &entries, Duration::from_secs(30))
            .await
        {
            Ok(r) => {
                let fails = check(c, &r.text);
                if fails.is_empty() {
                    pass += 1;
                    println!("PASS  {}  ({} ms)", c.name, r.total_ms);
                } else {
                    fail += 1;
                    println!(
                        "FAIL  {}\n      in:  {:?}\n      out: {:?}\n      {}",
                        c.name,
                        c.input,
                        r.text,
                        fails.join("; ")
                    );
                }
            }
            Err(e) => {
                fail += 1;
                println!("ERROR {}: {e:?}", c.name);
            }
        }
    }
    println!("\n{pass} passed, {fail} failed");
    if fail > 0 {
        std::process::exit(1);
    }
}
