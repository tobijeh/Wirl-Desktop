//! Messages between the app and the Whisper sidecar process. One JSON object
//! per line on stdin/stdout. Audio is 16 kHz mono 16-bit PCM, base64.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Request {
    Load {
        model_path: String,
        use_gpu: bool,
    },
    Transcribe {
        id: u64,
        pcm16_b64: String,
        language: String,
        initial_prompt: String,
        audio_ctx: i32,
    },
    Ping {
        id: u64,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Response {
    Ready { version: String },
    Loaded { model_path: String, gpu: bool, ms: u64 },
    Result { id: u64, text: String, ms: u64 },
    Pong { id: u64 },
    Error { id: Option<u64>, message: String },
}

pub fn encode_pcm(samples: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    B64.encode(bytes)
}

pub fn decode_pcm(b64: &str) -> Result<Vec<f32>, String> {
    let bytes = B64.decode(b64).map_err(|e| e.to_string())?;
    Ok(bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / i16::MAX as f32)
        .collect())
}

/// Builds Whisper's initial prompt from dictionary terms (most relevant
/// first), kept short because Whisper only uses the last ~224 tokens.
pub fn initial_prompt(terms: &[&str]) -> String {
    let mut out = String::new();
    for t in terms {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if out.len() + t.len() + 2 > 600 {
            break;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(t);
    }
    if out.is_empty() {
        out
    } else {
        format!("{out}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_roundtrip() {
        let s = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        let d = decode_pcm(&encode_pcm(&s)).unwrap();
        for (a, b) in s.iter().zip(d.iter()) {
            assert!((a - b).abs() < 1e-3);
        }
    }

    #[test]
    fn message_shape() {
        let r = Request::Ping { id: 3 };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"type":"ping","id":3}"#);
        let r: Response = serde_json::from_str(r#"{"type":"result","id":1,"text":"hi","ms":5}"#).unwrap();
        assert_eq!(
            r,
            Response::Result {
                id: 1,
                text: "hi".into(),
                ms: 5
            }
        );
    }

    #[test]
    fn prompt_from_terms() {
        assert_eq!(initial_prompt(&["Sela", " Wirl "]), "Sela, Wirl.");
        assert_eq!(initial_prompt(&[]), "");
        let many: Vec<String> = (0..500).map(|i| format!("Term{i}")).collect();
        let refs: Vec<&str> = many.iter().map(|s| s.as_str()).collect();
        assert!(initial_prompt(&refs).len() <= 602);
    }
}
