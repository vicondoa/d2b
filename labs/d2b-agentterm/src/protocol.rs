//! The agent-facing wire protocol.
//!
//! Newline-delimited JSON over a unix socket: one request object per line, one
//! response object per line. Every CLI subcommand is a single
//! connect/request/response, so an agent drives this with ordinary shell
//! commands rather than holding a long-lived session open.

use serde::{Deserialize, Serialize};

use crate::delta::DeltaReport;
use crate::screen::ScreenSnapshot;

/// Protocol version, bumped on any incompatible change to these types.
pub const PROTOCOL_VERSION: u32 = 1;

/// A request from the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Request {
    /// Session metadata.
    Info,
    /// The current screen.
    Screen,
    /// What changed over the trailing window.
    Delta { window_ms: u64 },
    /// Send key names, using the tmux-style grammar in [`crate::keys`].
    Keys { keys: Vec<String> },
    /// Send literal text, bracketed when the child has paste mode enabled.
    Text { text: String },
    /// Send raw bytes, base64-free: the exact string is written as UTF-8.
    /// Escape hatch for sequences the key grammar cannot express.
    Raw { data: String },
    /// Request a resize. Advisory: the human's terminal wins.
    Resize { cols: u16, rows: u16 },
    /// A sequence that reconstructs the current screen on a blank terminal.
    Dump,
}

/// Session metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub protocol_version: u32,
    pub child_pid: i32,
    pub cols: u16,
    pub rows: u16,
    pub alt_screen: bool,
    pub bracketed_paste: bool,
    pub cursor_key_app_mode: bool,
    pub uptime_ms: u64,
    pub exit_status: Option<i32>,
}

/// Result of an input or resize request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    /// Bytes written to the child.
    pub bytes: usize,
    /// Whether bracketed paste framing was applied.
    pub bracketed: bool,
    /// The size actually in effect after a resize request.
    pub cols: u16,
    pub rows: u16,
    /// Set when the request was honoured differently than asked.
    pub note: Option<String>,
}

/// A response to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Response {
    Info(SessionInfo),
    Screen(ScreenSnapshot),
    Delta(Box<DeltaReport>),
    Applied(Applied),
    Dump { seq: String },
    Error { message: String },
}

impl Response {
    pub fn error(message: impl Into<String>) -> Self {
        Response::Error {
            message: message.into(),
        }
    }

    /// True for an error response, so a client can set its exit code.
    pub fn is_error(&self) -> bool {
        matches!(self, Response::Error { .. })
    }
}

/// Encode a value as one protocol line, newline included.
pub fn encode_line<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::{Applied, PROTOCOL_VERSION, Request, Response, SessionInfo, encode_line};

    fn round_trip(req: &Request) -> Request {
        let json = serde_json::to_string(req).unwrap_or_default();
        serde_json::from_str(&json).unwrap_or_else(|_| req.clone())
    }

    #[test]
    fn requests_round_trip() {
        for req in [
            Request::Info,
            Request::Screen,
            Request::Delta { window_ms: 10_000 },
            Request::Keys {
                keys: vec!["Enter".into()],
            },
            Request::Text { text: "hi".into() },
            Request::Raw {
                data: "\x03".into(),
            },
            Request::Resize { cols: 80, rows: 24 },
            Request::Dump,
        ] {
            assert_eq!(round_trip(&req), req);
        }
    }

    #[test]
    fn request_tag_is_camel_case_on_the_wire() {
        let json = serde_json::to_string(&Request::Delta { window_ms: 500 }).unwrap_or_default();
        assert!(json.contains("\"type\":\"delta\""), "{json}");
        assert!(json.contains("\"windowMs\":500"), "{json}");
    }

    #[test]
    fn unknown_request_type_is_rejected() {
        let parsed: Result<Request, _> = serde_json::from_str(r#"{"type":"nope"}"#);
        assert!(parsed.is_err());
    }

    #[test]
    fn error_response_is_flagged() {
        assert!(Response::error("boom").is_error());
        assert!(!Response::Dump { seq: String::new() }.is_error());
    }

    #[test]
    fn encoded_line_is_newline_terminated_and_single_line() {
        let line = encode_line(&Response::error("x")).unwrap_or_default();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
    }

    #[test]
    fn session_info_round_trips() {
        let info = SessionInfo {
            protocol_version: PROTOCOL_VERSION,
            child_pid: 42,
            cols: 80,
            rows: 24,
            alt_screen: true,
            bracketed_paste: false,
            cursor_key_app_mode: true,
            uptime_ms: 1234,
            exit_status: None,
        };
        let json = serde_json::to_string(&info).unwrap_or_default();
        let back: SessionInfo = serde_json::from_str(&json).unwrap_or_else(|_| info.clone());
        assert_eq!(info, back);
    }

    #[test]
    fn applied_round_trips() {
        let applied = Applied {
            bytes: 3,
            bracketed: true,
            cols: 80,
            rows: 24,
            note: Some("clamped".into()),
        };
        let json = serde_json::to_string(&applied).unwrap_or_default();
        let back: Applied = serde_json::from_str(&json).unwrap_or_else(|_| applied.clone());
        assert_eq!(applied, back);
    }
}
