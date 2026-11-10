//! The Claude cleanup prompt and request body.
//!
//! One shared builder is used by dictation, clean-selection, and the manual
//! quality-test script so all three send exactly the same request shape.

use crate::dictionary::Entry;
use serde_json::{json, Value};

/// Claude model used for cleanup.
pub const CLEANUP_MODEL: &str = "claude-haiku-5-5";
/// Messages API version header.
pub const ANTHROPIC_VERSION: &str = "2026-10-21";
/// Output cap. Dictations are short; this leaves plenty of room.
pub const CLEANUP_MAX_TOKENS: u32 = 4096;

/// Built-in core prompt. Not user-editable. Must match the product spec exactly.
pub const CORE_PROMPT: &str = "You are an expert editor turning spoken dictation into the message the speaker intended to write. The text came from speech recognition, which often produces correctly spelled words that are the wrong words.

Work in three steps:

1. Understand. Read the entire dictation and work out what the speaker is trying to say: the topic, the point, the audience, and the context.

2. Resolve the speaker's intent:
- Self-corrections: when the speaker corrects themselves ('no, actually,' 'I mean,' 'wait,' 'sorry,' 'not that, this'), keep only the final intended version and remove the discarded one.
- Restarts and repetition: remove false starts and repeated phrases.
- Redundancy: if the same point is made twice in different words, keep it once, using the speaker's clearest phrasing.
- Spoken commands: if the speaker gives an instruction about the text itself ('new paragraph,' 'new line,' 'scratch that,' 'delete that last part,' 'make that a list,' 'put that in quotes,' 'question mark,' 'all caps'), carry it out and remove the command words.
- Only treat words as a command when they're clearly directed at the text. If they're part of the message ('tell him no, actually it's fine'), keep them as content.

3. Polish, checking every word and phrase against the overall meaning:
- If a word or phrase doesn't fit the logic of the message, replace it with what the speaker most likely said, choosing words that sound similar. Example: 'Is it still a pro, juicing that spacebar thing' becomes 'Is it still producing that spacebar thing.'
- Watch for real words that are the wrong word (their/there, accept/except, and sound-alikes that make no sense in context).
- Fix punctuation, capitalization, and grammar.
- Remove filler words (um, uh, like, you know).
- Use the personal dictionary to correct names and special terms.

Rules:
- Keep the speaker's meaning, tone, and voice. A casual text should still sound casual.
- Don't add new ideas, facts, or opinions.
- Don't rephrase sentences that are already clear.
- If you can't tell what was meant, keep the original wording.
- Never describe sounds or noises. If there are no intended words, return nothing.
- Return only the final text, with no preamble, notes, or quotes.";

pub const STYLE_HEADER: &str = "Speaker's style preferences:";
pub const DICTIONARY_HEADER: &str = "Always spell these terms exactly as written. If a word sounds like a term or matches a listed mishearing, and the context fits the note, replace it with the term:";

/// Builds the full system prompt: core prompt, then optional style notes,
/// then the dictionary entries that plausibly match this dictation.
pub fn build_system_prompt(style_notes: &str, entries: &[&Entry]) -> String {
    let mut s = String::from(CORE_PROMPT);
    let style = style_notes.trim();
    if !style.is_empty() {
        s.push_str("\n\n");
        s.push_str(STYLE_HEADER);
        s.push('\n');
        s.push_str(style);
    }
    if !entries.is_empty() {
        s.push_str("\n\n");
        s.push_str(DICTIONARY_HEADER);
        for e in entries {
            s.push('\n');
            s.push_str(&format_entry(e));
        }
    }
    s
}

/// One dictionary line: term, the wrong words it gets typed as, and note.
pub fn format_entry(e: &Entry) -> String {
    let mut line = format!("- {}", e.term.trim());
    let sounds: Vec<&str> = e
        .sounds_like
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if !sounds.is_empty() {
        line.push_str(&format!(" (misheard as: {})", sounds.join(", ")));
    }
    let note = e.note.trim();
    if !note.is_empty() {
        line.push_str(&format!(" — note: {note}"));
    }
    line
}

/// Wraps the text so Claude edits it rather than answering it.
pub fn wrap_user_text(text: &str) -> String {
    format!("<dictation>\n{}\n</dictation>", text.trim())
}

/// Builds the Messages API request body. Extended thinking is explicitly off.
pub fn build_request(system_prompt: &str, text: &str, stream: bool) -> Value {
    json!({
        "model": CLEANUP_MODEL,
        "max_tokens": CLEANUP_MAX_TOKENS,
        "stream": stream,
        "thinking": { "type": "disabled" },
        "output_config": { "effort": "low" },
        "system": system_prompt,
        "messages": [
            { "role": "user", "content": wrap_user_text(text) }
        ]
    })
}

/// Cleans up what Claude returned: removes any echoed wrapper tags and
/// surrounding quotes, keeps internal line breaks and lists intact.
pub fn postprocess_output(out: &str) -> String {
    let mut s = out.trim().to_string();
    for tag in ["<dictation>", "</dictation>"] {
        s = s.replace(tag, "");
    }
    let s = s.trim();
    // Normalise line endings but keep every line break and list marker.
    s.replace("\r\n", "\n").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::Entry;

    #[test]
    fn request_has_thinking_off_and_exact_core_prompt() {
        let sys = build_system_prompt("", &[]);
        assert_eq!(sys, CORE_PROMPT);
        let req = build_request(&sys, "hello there", true);
        assert_eq!(req["model"], "claude-haiku-5-5");
        assert_eq!(req["thinking"]["type"], "disabled");
        assert_eq!(req["system"].as_str().unwrap(), CORE_PROMPT);
        assert!(req.get("temperature").is_none());
        assert_eq!(req["messages"][0]["role"], "user");
        assert!(req["messages"][0]["content"].as_str().unwrap().contains("hello there"));
    }

    #[test]
    fn core_prompt_matches_spec_markers() {
        assert!(CORE_PROMPT.starts_with("You are an expert editor turning spoken dictation"));
        assert!(CORE_PROMPT.ends_with("Return only the final text, with no preamble, notes, or quotes."));
        assert!(CORE_PROMPT.contains(
            "'Is it still a pro, juicing that spacebar thing' becomes 'Is it still producing that spacebar thing.'"
        ));
        assert_eq!(CORE_PROMPT.matches("\n\n").count(), 5);
    }

    #[test]
    fn style_and_dictionary_appended_in_order() {
        let e = Entry::new("Sela", vec!["salad".into(), "sella".into()], "my daughter's name");
        let sys = build_system_prompt("  Use British spelling.  ", &[&e]);
        let style_at = sys.find(STYLE_HEADER).unwrap();
        let dict_at = sys.find(DICTIONARY_HEADER).unwrap();
        assert!(style_at > CORE_PROMPT.len() - 1);
        assert!(dict_at > style_at);
        assert!(sys.contains("Use British spelling."));
        assert!(sys.contains("- Sela (misheard as: salad, sella) — note: my daughter's name"));
    }

    #[test]
    fn empty_style_is_omitted() {
        let sys = build_system_prompt("   \n ", &[]);
        assert!(!sys.contains(STYLE_HEADER));
    }

    #[test]
    fn postprocess_keeps_lists_and_breaks() {
        let out = postprocess_output("<dictation>\nShopping:\n- eggs\n- milk\r\n\nThanks\n</dictation>");
        assert_eq!(out, "Shopping:\n- eggs\n- milk\n\nThanks");
    }
}
