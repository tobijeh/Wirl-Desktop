//! Whisper output filtering: strips sound tags and known hallucinations.

use regex::Regex;
use std::sync::OnceLock;

/// Phrases Whisper invents on silence or noise. Removed only when the whole
/// transcript consists of them.
const HALLUCINATIONS: &[&str] = &[
    "thanks for watching",
    "thank you for watching",
    "thank you so much for watching",
    "thanks for watching and see you next time",
    "thank you for watching and see you next time",
    "please subscribe",
    "please like and subscribe",
    "like and subscribe",
    "dont forget to like and subscribe",
    "subscribe to my channel",
    "please subscribe to my channel",
    "see you in the next video",
    "see you next time",
    "subtitles by the amaraorg community",
    "transcribed by",
    "you",
];

fn tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // [anything], (one to four words), *one to four words*, and music notes.
    RE.get_or_init(|| {
        Regex::new(r"\[[^\]]*\]|\((?:\s*[A-Za-z'\-]+){1,4}\s*\)|\*(?:\s*[A-Za-z'\-]+){1,4}\s*\*|[♪♫]+").unwrap()
    })
}

fn normalise(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_hallucination_sentence(s: &str) -> bool {
    let n = normalise(s);
    n.is_empty() || HALLUCINATIONS.contains(&n.as_str())
}

/// Returns the cleaned transcript, or an empty string if nothing real remains.
pub fn filter_transcript(raw: &str) -> String {
    let stripped = tag_re().replace_all(raw, " ");
    // Collapse whitespace per line, keep line structure.
    let text = stripped
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim().trim_start_matches([',', ';']).trim().to_string();
    if text.is_empty() {
        return String::new();
    }
    // If every sentence is a known hallucination, there was no speech.
    let sentences: Vec<&str> = text
        .split(['.', '!', '?', '\n'])
        .filter(|s| !s.trim().is_empty())
        .collect();
    if sentences.iter().all(|s| is_hallucination_sentence(s)) {
        return String::new();
    }
    if normalise(&text).is_empty() {
        return String::new();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sound_tags() {
        assert_eq!(filter_transcript("[BLANK_AUDIO]"), "");
        assert_eq!(filter_transcript(" [beep] hello [music] world (music) "), "hello world");
        assert_eq!(filter_transcript("(silence)"), "");
        assert_eq!(filter_transcript("♪ ♪"), "");
        assert_eq!(filter_transcript("*coughs* okay let's go"), "okay let's go");
    }

    #[test]
    fn removes_lone_hallucinations() {
        assert_eq!(filter_transcript("Thanks for watching!"), "");
        assert_eq!(filter_transcript(" Thank you for watching."), "");
        assert_eq!(filter_transcript("Please subscribe."), "");
        assert_eq!(filter_transcript("Thanks for watching! Please subscribe."), "");
        assert_eq!(filter_transcript("[music] Thank you for watching. [music]"), "");
        assert_eq!(filter_transcript(" you"), "");
    }

    #[test]
    fn keeps_real_speech_containing_those_words() {
        assert_eq!(
            filter_transcript("Thanks for watching the kids last night."),
            "Thanks for watching the kids last night."
        );
        assert_eq!(
            filter_transcript("Thanks for watching! See you at dinner."),
            "Thanks for watching! See you at dinner."
        );
        assert_eq!(filter_transcript("Thank you"), "Thank you");
    }

    #[test]
    fn keeps_long_parentheticals() {
        let s = "Meet at the cafe (the one on fifth street near the park) at noon";
        assert_eq!(filter_transcript(s), s);
    }

    #[test]
    fn punctuation_only_is_empty() {
        assert_eq!(filter_transcript(" . , ..."), "");
    }
}
