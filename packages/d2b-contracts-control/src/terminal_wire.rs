//! Shared terminal DTOs used by exec today and future interactive adapters.
//!
//! These are semantic DTOs, not a replacement for the existing public exec wire.
//! `Exec*` request/response structs in `public_wire` keep their current serde
//! shape.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
/// Which output stream a terminal read targets.
pub enum TerminalStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Terminal dimensions in rows and columns.
pub struct TerminalSize {
    /// Row count.
    pub rows: u32,
    /// Column count.
    pub cols: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One stdin write to a terminal session.
///
/// The session identifier is redacted in `Debug`.
pub struct TerminalWriteStdin {
    /// Session identifier.
    pub session: String,
    /// Byte offset this chunk continues from.
    pub offset: u64,
    /// Base64-encoded chunk bytes.
    pub chunk_base64: String,
    /// Whether this chunk closes stdin.
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
/// One output read from a terminal session.
///
/// The session identifier is redacted in `Debug`.
pub struct TerminalReadOutput {
    /// Session identifier.
    pub session: String,
    /// Stream to read from.
    pub stream: TerminalStream,
    /// Byte offset to read from.
    pub offset: u64,
    /// Maximum bytes to return.
    pub max_len: u64,
    /// Whether to block until output is available.
    #[serde(default)]
    pub wait: bool,
    /// Bound on the wait, in milliseconds.
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
/// One resize request for a terminal session.
///
/// The session identifier is redacted in `Debug`.
pub struct TerminalResize {
    /// Session identifier.
    pub session: String,
    /// New row count.
    pub rows: u32,
    /// New column count.
    pub cols: u32,
    /// Caller-supplied operation id for correlation.
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
/// Result of one terminal stdin write.
pub struct TerminalWriteStdinResult {
    /// Bytes accepted by the session.
    pub accepted_len: u64,
    /// Offset the next chunk should continue from.
    pub next_offset: u64,
    /// Whether the write was backpressured.
    #[serde(default)]
    pub backpressured: bool,
    /// Whether stdin is now closed.
    #[serde(default)]
    pub stdin_closed: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One output chunk read from a terminal session.
pub struct TerminalReadOutputChunk {
    /// Base64-encoded output bytes.
    pub data_base64: String,
    /// Offset the next read should continue from.
    pub next_offset: u64,
    /// Whether this chunk is the last output.
    #[serde(default)]
    pub eof: bool,
    /// Bytes dropped because the ring buffer overflowed.
    #[serde(default)]
    pub dropped_bytes: u64,
    /// Whether the chunk was truncated to the requested bound.
    #[serde(default)]
    pub truncated: bool,
    /// Whether the read timed out before output arrived.
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