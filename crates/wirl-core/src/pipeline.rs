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
        Some(Ok(t)) if t.trim().is_empty() => Final::Discard,
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
}
