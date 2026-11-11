//! Decisions in the dictation pipeline that don't depend on the OS.

use crate::claude::CleanupError;
use crate::filter::filter_transcript;

/// Recordings shorter than this are discarded silently.
pub const MIN_RECORDING_MS: usize = 1_000;

/// Timeouts. On timeout the stage is cancelled and Wirl returns to idle.
pub const MODEL_LOAD_TIMEOUT_MS: u64 = 60_000;
pub const CHUNK_TRANSCRIBE_TIMEOUT_MS: u64 = 20_000;
pub const CLEANUP_TIMEOUT_MS: u64 = 15_000;

/// Whether a stopped recording should be dropped without any message,
/// before transcription.
pub fn discard_before_transcription(duration_ms: usize, any_speech: bool) -> bool {
    duration_ms < MIN_RECORDING_MS || !any_speech
}

/// Filters the joined Whisper output. None means "no speech".
pub fn transcript_or_none(joined: &str) -> Option<String> {
    let t = filter_transcript(joined);
    if t.trim().is_empty() {
        None
    } else {
        Some(t)
    }
}

/// Fewer letters than this is not a sentence: it's noise or a stray sound
/// (Whisper writing "." or "Hm."), so Claude isn't called and nothing is
/// inserted. Same rule as Android (`SpeechGate.MIN_LETTERS`). Chinese
/// characters count as letters.
pub const MIN_LETTERS: usize = 3;

/// Whether the heard text has enough letters to be worth cleaning up and
/// inserting.
pub fn has_words(spoken: &str) -> bool {
    spoken.chars().filter(|c| c.is_alphabetic()).count() >= MIN_LETTERS
}

/// The text of a piece with only faint speech (`Chunk::weak_speech`), kept
/// only if it reads as real words: not a made-up phrase or sound tag on its
/// own (`filter_transcript`), and at least [`MIN_LETTERS`] letters.
pub fn weak_chunk_text(text: &str) -> Option<String> {
    let t = filter_transcript(text);
    has_words(&t).then_some(t)
}

/// Replies longer than this are real text, even if they mention silence.
const MAX_REMARK_WORDS: usize = 12;

/// True when Claude's reply talks about the recording ("No speech
/// detected.", "Unclear audio, please try again.") instead of giving text
/// for the field. The whole reply must be such a remark. Same list as
/// Android (`SpeechGate.isRecordingRemark`).
pub fn is_recording_remark(reply: &str) -> bool {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let text = reply.trim();
    if text.is_empty() || text.split_whitespace().count() > MAX_REMARK_WORDS {
        return false;
    }
    let re = RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)^(?:no (?:speech|words|audio|intended words)(?: (?:was |were )?(?:detected|heard|found|recorded))?|",
            r"nothing (?:was )?(?:said|spoken|heard)|silence|silent|",
            r"(?:the )?(?:audio|recording|speech) (?:is|was|seems|sounds) (?:unclear|silent|empty|inaudible)|",
            r"unclear (?:audio|speech)(?:, please try again)?|inaudible|",
            r"i (?:can't|cannot|couldn't|could not) (?:hear|make out) (?:anything|the audio|you)|",
            r"(?:please )?try (?:recording|speaking) again|sorry,? i (?:didn't|did not|can't|cannot) hear)[.!]?$"
        ))
        .unwrap()
    });
    re.is_match(text)
}

/// A reply with no letters or digits at all ("." or "…") is never text
/// worth inserting.
fn has_content(reply: &str) -> bool {
    reply.chars().any(|c| c.is_alphanumeric())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Final {
    /// Nothing to insert: pill disappears, no message.
    Discard,
    /// Insert this text; show `warning` (amber) instead of "Inserted".
    Insert { text: String, warning: Option<String> },
}

/// Combines the raw transcript with the cleanup result. If cleanup fails,
/// the raw transcript is inserted and the reason shown.
pub fn after_cleanup(raw: &str, cleanup: Option<Result<String, CleanupError>>) -> Final {
    match cleanup {
        None => Final::Insert {
            text: raw.to_string(),
            warning: None,
        },
        // Nothing, only punctuation, or a remark about the recording:
        // nothing is inserted and nothing is shown.
        Some(Ok(t)) if !has_content(&t) || is_recording_remark(&t) => Final::Discard,
        Some(Ok(t)) => Final::Insert { text: t, warning: None },
        Some(Err(e)) => Final::Insert {
            text: raw.to_string(),
            warning: Some(e.dictation_message()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_or_silent_recordings_are_discarded() {
        assert!(discard_before_transcription(900, true));
        assert!(discard_before_transcription(5_000, false));
        assert!(!discard_before_transcription(1_200, true));
    }

    #[test]
    fn hallucination_only_is_no_speech() {
        assert_eq!(transcript_or_none(" [BLANK_AUDIO] Thanks for watching!"), None);
        assert_eq!(transcript_or_none("hello").as_deref(), Some("hello"));
    }

    #[test]
    fn cleanup_failure_inserts_raw_with_reason() {
        let f = after_cleanup("raw text", Some(Err(CleanupError::NoInternet)));
        assert_eq!(
            f,
            Final::Insert {
                text: "raw text".into(),
                warning: Some("No internet. Inserted raw text.".into())
            }
        );
        let f = after_cleanup("raw", Some(Err(CleanupError::Timeout)));
        assert!(matches!(f, Final::Insert { warning: Some(w), .. } if w.starts_with("Cleanup timed out")));
    }

    #[test]
    fn cleanup_success_and_disabled() {
        assert_eq!(
            after_cleanup("raw", Some(Ok("Clean.".into()))),
            Final::Insert {
                text: "Clean.".into(),
                warning: None
            }
        );
        assert_eq!(
            after_cleanup("raw", None),
            Final::Insert {
                text: "raw".into(),
                warning: None
            }
        );
        assert_eq!(after_cleanup("raw", Some(Ok("  ".into()))), Final::Discard);
    }

    #[test]
    fn faint_pieces_keep_only_real_words() {
        assert_eq!(weak_chunk_text("很喜欢").as_deref(), Some("很喜欢"));
        assert_eq!(
            weak_chunk_text(" and the client loved it ").as_deref(),
            Some("and the client loved it")
        );
        for noise in [
            "",
            "嗯",
            ".",
            "you",
            "[BLANK_AUDIO]",
            "(clicking)",
            "Thanks for watching!",
        ] {
            assert_eq!(weak_chunk_text(noise), None, "{noise}");
        }
    }

    #[test]
    fn noise_never_inserts_anything() {
        // Too few letters: Claude isn't asked.
        for noise in [".", "…", "Hm.", "Oh", "Mm", "嗯"] {
            assert!(!has_words(noise), "{noise}");
        }
        for words in ["Yes.", "Call me", "你好嗎", "OK go"] {
            assert!(has_words(words), "{words}");
        }
        // Claude's reply is only punctuation, or a remark about the recording.
        for reply in [
            ".",
            "…",
            " - ",
            "No speech detected.",
            "Unclear audio, please try again.",
            "Silence",
            "inaudible",
        ] {
            assert_eq!(after_cleanup("raw", Some(Ok(reply.into()))), Final::Discard, "{reply}");
        }
        // A real sentence that mentions silence is kept.
        let real = "There was silence in the room after the meeting ended.";
        assert!(!is_recording_remark(real));
        assert!(matches!(
            after_cleanup("raw", Some(Ok(real.into()))),
            Final::Insert { .. }
        ));
        assert!(matches!(
            after_cleanup("raw", Some(Ok("42".into()))),
            Final::Insert { .. }
        ));
    }
}
