//! Shared terminal DTOs used by exec today and future interactive adapters.
//!
//! These are semantic DTOs, not a replacement for the existing public exec wire.
//! `Exec*` request/response structs in `public_wire` keep their current serde
//! shape.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TerminalStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalSize {
    pub rows: u32,
    pub cols: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalWriteStdin {
    pub session: String,
    pub offset: u64,
    pub chunk_base64: String,
    #[serde(default)]
    pub eof: bool,
}

impl std::fmt::Debug for TerminalWriteStdin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalWriteStdin")
            .field("session", &"<redacted>")
            .field("offset", &self.offset)
            .field("chunk_base64_len", &self.chunk_base64.len())
            .field("eof", &self.eof)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalReadOutput {
    pub session: String,
    pub stream: TerminalStream,
    pub offset: u64,
    pub max_len: u64,
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    pub timeout_ms: u64,
}

impl std::fmt::Debug for TerminalReadOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalReadOutput")
            .field("session", &"<redacted>")
            .field("stream", &self.stream)
            .field("offset", &self.offset)
            .field("max_len", &self.max_len)
            .field("wait", &self.wait)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalResize {
    pub session: String,
    pub rows: u32,
    pub cols: u32,
    #[serde(default)]
    pub op_id: u64,
}

impl std::fmt::Debug for TerminalResize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalResize")
            .field("session", &"<redacted>")
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .field("op_id", &self.op_id)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalWriteStdinResult {
    pub accepted_len: u64,
    pub next_offset: u64,
    #[serde(default)]
    pub backpressured: bool,
    #[serde(default)]
    pub stdin_closed: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalReadOutputChunk {
    pub data_base64: String,
    pub next_offset: u64,
    #[serde(default)]
    pub eof: bool,
    #[serde(default)]
    pub dropped_bytes: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub timed_out: bool,
}

impl std::fmt::Debug for TerminalReadOutputChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalReadOutputChunk")
            .field("data_base64_len", &self.data_base64.len())
            .field("next_offset", &self.next_offset)
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("timed_out", &self.timed_out)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::terminal_wire::{TerminalReadOutputChunk, TerminalWriteStdin};

    #[test]
    fn terminal_debug_redacts_sensitive_values() {
        let write = TerminalWriteStdin {
            session: "session-secret".to_owned(),
            offset: 0,
            chunk_base64: "c2VjcmV0LWtleXM=".to_owned(),
            eof: false,
        };
        let debug = format!("{write:?}");
        assert!(!debug.contains("session-secret"));
        assert!(!debug.contains("c2VjcmV0"));
        assert!(debug.contains("chunk_base64_len"));
    }

    #[test]
    fn output_chunk_debug_redacts_payload() {
        let chunk = TerminalReadOutputChunk {
            data_base64: "c2VjcmV0LW91dHB1dA==".to_owned(),
            next_offset: 20,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        };
        let debug = format!("{chunk:?}");
        assert!(!debug.contains("c2VjcmV0"));
        assert!(debug.contains("data_base64_len"));
    }
}