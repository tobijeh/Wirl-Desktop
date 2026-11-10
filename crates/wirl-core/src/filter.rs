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
    // SenseVoice on background noise.
    "嗯",
    "呃",
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

/// SenseVoice writes English words in capitals ("个 MEETING"). Lower-cases
/// all-capital words of two or more letters so the text reads normally even
/// with AI cleanup off; "I" and mixed-case words are left alone.
pub fn sensevoice_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.chars().count() >= 2 && word.chars().all(|c| c.is_ascii_uppercase()) {
            out.push_str(&word.to_ascii_lowercase());
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphabetic() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

#[cfg(test)]
mod sensevoice_tests {
    use super::*;

    #[test]
    fn capitals_become_lower_case() {
        assert_eq!(
            sensevoice_case("我今日有个 MEETING 之后要 SEND 个 EMAIL"),
            "我今日有个 meeting 之后要 send 个 email"
        );
        assert_eq!(sensevoice_case("I said OK to iPhone"), "I said ok to iPhone");
        assert_eq!(sensevoice_case(""), "");
    }

    #[test]
    fn lone_chinese_filler_is_no_speech() {
        assert_eq!(filter_transcript("嗯"), "");
        assert_eq!(filter_transcript("呃。"), "");
        assert_eq!(filter_transcript("嗯好啊"), "嗯好啊");
    }

    #[test]
    fn chinese_survives_the_filter() {
        assert_eq!(
            filter_transcript("呢几个字都表达唔到我想讲嘅意思"),
            "呢几个字都表达唔到我想讲嘅意思"
        );
    }
}
