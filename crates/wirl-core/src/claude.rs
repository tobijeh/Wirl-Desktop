//! Claude cleanup client: one keep-alive HTTP client, streaming responses so
//! the time to first byte is measurable.

use crate::dictionary::Entry;
use crate::prompt::{build_request, build_system_prompt, postprocess_output, PromptMode, ANTHROPIC_VERSION};
use futures_util::StreamExt;
use std::time::{Duration, Instant};

pub const API_URL: &str = "https://api.anthropic.com/v1/messages";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CleanupError {
    #[error("No API key")]
    NoKey,
    #[error("No internet")]
    NoInternet,
    #[error("API key not accepted")]
    InvalidKey,
    #[error("Check your Anthropic billing")]
    Billing,
    #[error("Claude is busy")]
    Busy,
    #[error("Cleanup timed out")]
    Timeout,
    #[error("Cleanup declined this text")]
    Refused,
    #[error("Cleanup failed")]
    Other(String),
}

impl CleanupError {
    /// Short plain reason for the pill.
    pub fn reason(&self) -> &'static str {
        match self {
            CleanupError::NoKey => "Add your API key in Setup",
            CleanupError::NoInternet => "No internet",
            CleanupError::InvalidKey => "API key not accepted",
            CleanupError::Billing => "Check your Anthropic billing",
            CleanupError::Busy => "Claude is busy",
            CleanupError::Timeout => "Cleanup timed out",
            CleanupError::Refused => "Cleanup declined this text",
            CleanupError::Other(_) => "Cleanup failed",
        }
    }

    /// Pill message after a dictation fell back to the raw transcript.
    pub fn dictation_message(&self) -> String {
        format!("{}. Inserted raw text.", self.reason())
    }

    /// Pill message after clean-selection failed (text left as it was).
    pub fn selection_message(&self) -> String {
        format!("{}. Text unchanged.", self.reason())
    }

    /// Whether this needs the user to visit Setup.
    pub fn needs_attention(&self) -> bool {
        matches!(
            self,
            CleanupError::NoKey | CleanupError::InvalidKey | CleanupError::Billing
        )
    }

    pub fn from_status(status: u16, body: &str) -> CleanupError {
        let lower = body.to_lowercase();
        match status {
            401 | 403 => CleanupError::InvalidKey,
            400 if lower.contains("credit balance") || lower.contains("billing") => CleanupError::Billing,
            402 => CleanupError::Billing,
            408 | 504 => CleanupError::Timeout,
            429 | 500 | 502 | 503 | 529 => CleanupError::Busy,
            _ => CleanupError::Other(format!("HTTP {status}: {}", body.chars().take(200).collect::<String>())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CleanupResult {
    pub text: String,
    pub first_byte_ms: u64,
    pub total_ms: u64,
    pub stop_reason: Option<String>,
    /// The connection to Claude was already open (warmed at record start or
    /// used recently), so no new TLS handshake was needed.
    pub warm_connection: bool,
}

#[derive(Clone)]
pub struct ClaudeClient {
    http: reqwest::Client,
    url: String,
    /// When the server last answered; the pool keeps a connection for 120 s.
    last_contact: std::sync::Arc<std::sync::Mutex<Option<Instant>>>,
}

/// Connections idle longer than this may have been closed by the pool.
const WARM_WINDOW: Duration = Duration::from_secs(110);

impl Default for ClaudeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeClient {
    pub fn new() -> Self {
        Self::with_url(API_URL)
    }

    pub fn with_url(url: &str) -> Self {
        let http = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(4)
            .tcp_keepalive(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .user_agent(concat!("Wirl/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client");
        ClaudeClient {
            http,
            url: url.to_string(),
            last_contact: Default::default(),
        }
    }

    /// Opens (or keeps open) the TLS connection so the real request starts
    /// faster. Called when recording starts. Errors are ignored.
    pub async fn warm(&self) {
        if self
            .http
            .head(&self.url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .is_ok()
        {
            self.touch();
        }
    }

    fn touch(&self) {
        if let Ok(mut t) = self.last_contact.lock() {
            *t = Some(Instant::now());
        }
    }

    /// Whether a connection is probably still open from a recent request.
    fn is_warm(&self) -> bool {
        self.last_contact
            .lock()
            .ok()
            .and_then(|t| *t)
            .is_some_and(|t| t.elapsed() < WARM_WINDOW)
    }

    /// The one shared cleanup function for dictation and clean-selection.
    pub async fn cleanup(
        &self,
        api_key: &str,
        mode: PromptMode,
        text: &str,
        style_notes: &str,
        entries: &[&Entry],
        timeout: Duration,
    ) -> Result<CleanupResult, CleanupError> {
        if api_key.trim().is_empty() {
            return Err(CleanupError::NoKey);
        }
        let system = build_system_prompt(mode, style_notes, entries);
        let body = build_request(&system, text, true);
        match tokio::time::timeout(timeout, self.send(api_key, &body)).await {
            Ok(r) => r,
            Err(_) => Err(CleanupError::Timeout),
        }
    }

    async fn send(&self, api_key: &str, body: &serde_json::Value) -> Result<CleanupResult, CleanupError> {
        let warm = self.is_warm();
        let start = Instant::now();
        let resp = self
            .http
            .post(&self.url)
            .header("x-api-key", api_key.trim())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(map_reqwest)?;
        let status = resp.status().as_u16();
        if status != 200 {
            let text = resp.text().await.unwrap_or_default();
            return Err(CleanupError::from_status(status, &text));
        }
        let mut stream = resp.bytes_stream();
        let mut first_byte_ms = None;
        let mut parser = SseParser::default();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_reqwest)?;
            if first_byte_ms.is_none() {
                first_byte_ms = Some(start.elapsed().as_millis() as u64);
            }
            parser.feed(&chunk)?;
            if parser.done {
                break;
            }
        }
        if parser.stop_reason.as_deref() == Some("refusal") {
            return Err(CleanupError::Refused);
        }
        self.touch();
        Ok(CleanupResult {
            text: postprocess_output(&parser.text),
            first_byte_ms: first_byte_ms.unwrap_or(0),
            total_ms: start.elapsed().as_millis() as u64,
            stop_reason: parser.stop_reason,
            warm_connection: warm,
        })
    }
}

fn map_reqwest(e: reqwest::Error) -> CleanupError {
    if e.is_timeout() {
        CleanupError::Timeout
    } else if e.is_connect() || e.is_request() {
        CleanupError::NoInternet
    } else {
        CleanupError::Other(e.to_string())
    }
}

/// Incremental parser for the Messages API server-sent events.
#[derive(Default, Debug)]
pub struct SseParser {
    buf: String,
    pub text: String,
    pub stop_reason: Option<String>,
    pub done: bool,
}

impl SseParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), CleanupError> {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        self.buf = self.buf.replace("\r\n", "\n");
        while let Some(pos) = self.buf.find("\n\n") {
            let event: String = self.buf[..pos].to_string();
            self.buf.drain(..pos + 2);
            self.handle_event(&event)?;
        }
        Ok(())
    }

    fn handle_event(&mut self, event: &str) -> Result<(), CleanupError> {
        let data: String = event
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(|l| l.trim_start())
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            return Ok(());
        }
        let v: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        match v["type"].as_str() {
            Some("content_block_delta") => {
                if v["delta"]["type"] == "text_delta" {
                    if let Some(t) = v["delta"]["text"].as_str() {
                        self.text.push_str(t);
                    }
                }
            }
            Some("message_delta") => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.to_string());
                }
            }
            Some("message_stop") => self.done = true,
            Some("error") => {
                let kind = v["error"]["type"].as_str().unwrap_or("");
                return Err(match kind {
                    "overloaded_error" | "rate_limit_error" | "api_error" => CleanupError::Busy,
                    "authentication_error" | "permission_error" => CleanupError::InvalidKey,
                    _ => CleanupError::Other(v["error"]["message"].as_str().unwrap_or(kind).to_string()),
                });
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stream_split_across_chunks() {
        let mut p = SseParser::default();
        let s = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\\nworld\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let bytes = s.as_bytes();
        for c in bytes.chunks(7) {
            p.feed(c).unwrap();
        }
        assert_eq!(p.text, "Hello\nworld");
        assert_eq!(p.stop_reason.as_deref(), Some("end_turn"));
        assert!(p.done);
    }

    #[test]
    fn stream_error_event_maps() {
        let mut p = SseParser::default();
        let e = p
            .feed(b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n")
            .unwrap_err();
        assert_eq!(e, CleanupError::Busy);
    }

    #[test]
    fn status_mapping_and_messages() {
        assert_eq!(CleanupError::from_status(401, ""), CleanupError::InvalidKey);
        assert_eq!(CleanupError::from_status(529, ""), CleanupError::Busy);
        assert_eq!(
            CleanupError::from_status(400, "Your credit balance is too low"),
            CleanupError::Billing
        );
        assert_eq!(
            CleanupError::NoInternet.dictation_message(),
            "No internet. Inserted raw text."
        );
        assert!(CleanupError::InvalidKey.needs_attention());
    }

    #[tokio::test]
    async fn empty_key_fails_fast() {
        let c = ClaudeClient::with_url("http://127.0.0.1:9/");
        let e = c
            .cleanup("  ", PromptMode::Standard, "hi", "", &[], Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(e, CleanupError::NoKey);
    }

    #[tokio::test]
    async fn unreachable_host_is_no_internet() {
        let c = ClaudeClient::with_url("http://127.0.0.1:9/");
        let e = c
            .cleanup("k", PromptMode::Standard, "hi", "", &[], Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(e, CleanupError::NoInternet | CleanupError::Timeout), "{e:?}");
    }
}
