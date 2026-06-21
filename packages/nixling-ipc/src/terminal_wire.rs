//! Shared terminal DTOs used by exec today and shell adapters later.
//!
//! These are semantic DTOs, not a replacement for the existing public exec wire.
//! `Exec*` request/response structs in `public_wire` keep their current serde
//! shape; conversions here are explicit so future shell adapters can reuse the
//! same terminal vocabulary without changing existing exec JSON.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::public_wire::{
    ExecCloseArgs, ExecCloseResult, ExecControlResult, ExecReadOutputArgs, ExecReadOutputResult,
    ExecResizeArgs, ExecSignalArgs, ExecStream, ExecTermSize, ExecTerminalStatus, ExecWaitArgs,
    ExecWaitResult, ExecWriteStdinArgs, ExecWriteStdinResult,
};

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
pub struct TerminalSignal {
    pub session: String,
    pub signo: u32,
    #[serde(default)]
    pub op_id: u64,
}

impl std::fmt::Debug for TerminalSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalSignal")
            .field("session", &"<redacted>")
            .field("signo", &self.signo)
            .field("op_id", &self.op_id)
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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalWait {
    pub session: String,
    #[serde(default)]
    pub timeout_ms: u64,
}

impl std::fmt::Debug for TerminalWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalWait")
            .field("session", &"<redacted>")
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalClose {
    pub session: String,
}

impl std::fmt::Debug for TerminalClose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalClose")
            .field("session", &"<redacted>")
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalControlResult {
    #[serde(default)]
    pub delivered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum TerminalStatus {
    Exited { code: i32 },
    Signaled { signal: u32 },
    Error { slug: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalWaitResult {
    #[serde(default)]
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_status: Option<TerminalStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalCloseResult {
    #[serde(default)]
    pub stdin_closed: bool,
}

impl From<ExecStream> for TerminalStream {
    fn from(value: ExecStream) -> Self {
        match value {
            ExecStream::Stdout => Self::Stdout,
            ExecStream::Stderr => Self::Stderr,
        }
    }
}

impl From<TerminalStream> for ExecStream {
    fn from(value: TerminalStream) -> Self {
        match value {
            TerminalStream::Stdout => Self::Stdout,
            TerminalStream::Stderr => Self::Stderr,
        }
    }
}

impl From<ExecTermSize> for TerminalSize {
    fn from(value: ExecTermSize) -> Self {
        Self {
            rows: value.rows,
            cols: value.cols,
        }
    }
}

impl From<TerminalSize> for ExecTermSize {
    fn from(value: TerminalSize) -> Self {
        Self {
            rows: value.rows,
            cols: value.cols,
        }
    }
}

impl From<ExecWriteStdinArgs> for TerminalWriteStdin {
    fn from(value: ExecWriteStdinArgs) -> Self {
        Self {
            session: value.session,
            offset: value.offset,
            chunk_base64: value.chunk_base64,
            eof: value.eof,
        }
    }
}

impl From<TerminalWriteStdin> for ExecWriteStdinArgs {
    fn from(value: TerminalWriteStdin) -> Self {
        Self {
            session: value.session,
            offset: value.offset,
            chunk_base64: value.chunk_base64,
            eof: value.eof,
        }
    }
}

impl From<ExecReadOutputArgs> for TerminalReadOutput {
    fn from(value: ExecReadOutputArgs) -> Self {
        Self {
            session: value.session,
            stream: value.stream.into(),
            offset: value.offset,
            max_len: value.max_len,
            wait: value.wait,
            timeout_ms: value.timeout_ms,
        }
    }
}

impl From<TerminalReadOutput> for ExecReadOutputArgs {
    fn from(value: TerminalReadOutput) -> Self {
        Self {
            session: value.session,
            stream: value.stream.into(),
            offset: value.offset,
            max_len: value.max_len,
            wait: value.wait,
            timeout_ms: value.timeout_ms,
        }
    }
}

impl From<ExecSignalArgs> for TerminalSignal {
    fn from(value: ExecSignalArgs) -> Self {
        Self {
            session: value.session,
            signo: value.signo,
            op_id: value.op_id,
        }
    }
}

impl From<TerminalSignal> for ExecSignalArgs {
    fn from(value: TerminalSignal) -> Self {
        Self {
            session: value.session,
            signo: value.signo,
            op_id: value.op_id,
        }
    }
}

impl From<ExecResizeArgs> for TerminalResize {
    fn from(value: ExecResizeArgs) -> Self {
        Self {
            session: value.session,
            rows: value.rows,
            cols: value.cols,
            op_id: value.op_id,
        }
    }
}

impl From<TerminalResize> for ExecResizeArgs {
    fn from(value: TerminalResize) -> Self {
        Self {
            session: value.session,
            rows: value.rows,
            cols: value.cols,
            op_id: value.op_id,
        }
    }
}

impl From<ExecWaitArgs> for TerminalWait {
    fn from(value: ExecWaitArgs) -> Self {
        Self {
            session: value.session,
            timeout_ms: value.timeout_ms,
        }
    }
}

impl From<TerminalWait> for ExecWaitArgs {
    fn from(value: TerminalWait) -> Self {
        Self {
            session: value.session,
            timeout_ms: value.timeout_ms,
        }
    }
}

impl From<ExecCloseArgs> for TerminalClose {
    fn from(value: ExecCloseArgs) -> Self {
        Self {
            session: value.session,
        }
    }
}

impl From<TerminalClose> for ExecCloseArgs {
    fn from(value: TerminalClose) -> Self {
        Self {
            session: value.session,
        }
    }
}

impl From<ExecWriteStdinResult> for TerminalWriteStdinResult {
    fn from(value: ExecWriteStdinResult) -> Self {
        Self {
            accepted_len: value.accepted_len,
            next_offset: value.next_offset,
            backpressured: value.backpressured,
            stdin_closed: value.stdin_closed,
        }
    }
}

impl From<TerminalWriteStdinResult> for ExecWriteStdinResult {
    fn from(value: TerminalWriteStdinResult) -> Self {
        Self {
            accepted_len: value.accepted_len,
            next_offset: value.next_offset,
            backpressured: value.backpressured,
            stdin_closed: value.stdin_closed,
        }
    }
}

impl From<ExecReadOutputResult> for TerminalReadOutputChunk {
    fn from(value: ExecReadOutputResult) -> Self {
        Self {
            data_base64: value.data_base64,
            next_offset: value.next_offset,
            eof: value.eof,
            dropped_bytes: value.dropped_bytes,
            truncated: value.truncated,
            timed_out: value.timed_out,
        }
    }
}

impl From<TerminalReadOutputChunk> for ExecReadOutputResult {
    fn from(value: TerminalReadOutputChunk) -> Self {
        Self {
            data_base64: value.data_base64,
            next_offset: value.next_offset,
            eof: value.eof,
            dropped_bytes: value.dropped_bytes,
            truncated: value.truncated,
            timed_out: value.timed_out,
        }
    }
}

impl From<ExecControlResult> for TerminalControlResult {
    fn from(value: ExecControlResult) -> Self {
        Self {
            delivered: value.delivered,
        }
    }
}

impl From<TerminalControlResult> for ExecControlResult {
    fn from(value: TerminalControlResult) -> Self {
        Self {
            delivered: value.delivered,
        }
    }
}

impl From<ExecTerminalStatus> for TerminalStatus {
    fn from(value: ExecTerminalStatus) -> Self {
        match value {
            ExecTerminalStatus::Exited { code } => Self::Exited { code },
            ExecTerminalStatus::Signaled { signal } => Self::Signaled { signal },
            ExecTerminalStatus::Error { slug } => Self::Error { slug },
        }
    }
}

impl From<TerminalStatus> for ExecTerminalStatus {
    fn from(value: TerminalStatus) -> Self {
        match value {
            TerminalStatus::Exited { code } => Self::Exited { code },
            TerminalStatus::Signaled { signal } => Self::Signaled { signal },
            TerminalStatus::Error { slug } => Self::Error { slug },
        }
    }
}

impl From<ExecWaitResult> for TerminalWaitResult {
    fn from(value: ExecWaitResult) -> Self {
        Self {
            running: value.running,
            terminal_status: value.terminal_status.map(Into::into),
        }
    }
}

impl From<TerminalWaitResult> for ExecWaitResult {
    fn from(value: TerminalWaitResult) -> Self {
        Self {
            running: value.running,
            terminal_status: value.terminal_status.map(Into::into),
        }
    }
}

impl From<ExecCloseResult> for TerminalCloseResult {
    fn from(value: ExecCloseResult) -> Self {
        Self {
            stdin_closed: value.stdin_closed,
        }
    }
}

impl From<TerminalCloseResult> for ExecCloseResult {
    fn from(value: TerminalCloseResult) -> Self {
        Self {
            stdin_closed: value.stdin_closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::public_wire::{
        ExecOp, ExecReadOutputArgs, ExecReadOutputResult, ExecResizeArgs, ExecStream,
        ExecWaitResult, ExecWriteStdinArgs,
    };
    use crate::terminal_wire::{TerminalReadOutput, TerminalStatus, TerminalWriteStdin};

    #[test]
    fn exec_write_json_is_preserved_across_terminal_conversion() {
        let exec = ExecWriteStdinArgs {
            session: "s-opaque".to_owned(),
            offset: 7,
            chunk_base64: "c2VjcmV0".to_owned(),
            eof: true,
        };
        let before = serde_json::to_value(&exec).expect("exec serializes");
        let terminal: TerminalWriteStdin = exec.clone().into();
        let round_trip: ExecWriteStdinArgs = terminal.into();
        let after = serde_json::to_value(&round_trip).expect("exec serializes after conversion");
        assert_eq!(before, after);
        assert_eq!(
            serde_json::to_string(&ExecOp::WriteStdin(round_trip)).expect("op serializes"),
            r#"{"op":"writeStdin","args":{"session":"s-opaque","offset":7,"chunkBase64":"c2VjcmV0","eof":true}}"#
        );
    }

    #[test]
    fn exec_read_json_is_preserved_across_terminal_conversion() {
        let exec = ExecReadOutputArgs {
            session: "s-opaque".to_owned(),
            stream: ExecStream::Stdout,
            offset: 11,
            max_len: 4096,
            wait: true,
            timeout_ms: 50,
        };
        let before = serde_json::to_value(&exec).expect("exec serializes");
        let terminal: TerminalReadOutput = exec.clone().into();
        let round_trip: ExecReadOutputArgs = terminal.into();
        assert_eq!(before, serde_json::to_value(&round_trip).unwrap());
        assert_eq!(
            serde_json::to_string(&ExecOp::ReadOutput(round_trip)).expect("op serializes"),
            r#"{"op":"readOutput","args":{"session":"s-opaque","stream":"stdout","offset":11,"maxLen":4096,"wait":true,"timeoutMs":50}}"#
        );
    }

    #[test]
    fn control_and_status_json_is_preserved_across_terminal_conversion() {
        let resize = ExecResizeArgs {
            session: "s-opaque".to_owned(),
            rows: 40,
            cols: 120,
            op_id: 9,
        };
        let resize_json = serde_json::to_value(&resize).unwrap();
        let round_trip: ExecResizeArgs =
            crate::terminal_wire::TerminalResize::from(resize.clone()).into();
        assert_eq!(resize_json, serde_json::to_value(&round_trip).unwrap());

        let wait = ExecWaitResult {
            running: false,
            terminal_status: Some(crate::public_wire::ExecTerminalStatus::Error {
                slug: "lost-guestd".to_owned(),
            }),
        };
        let terminal: crate::terminal_wire::TerminalWaitResult = wait.clone().into();
        assert_eq!(
            terminal.terminal_status,
            Some(TerminalStatus::Error {
                slug: "lost-guestd".to_owned()
            })
        );
        let round_trip: ExecWaitResult = terminal.into();
        assert_eq!(
            serde_json::to_value(&wait).unwrap(),
            serde_json::to_value(&round_trip).unwrap()
        );
    }

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
        let chunk: crate::terminal_wire::TerminalReadOutputChunk = ExecReadOutputResult {
            data_base64: "c2VjcmV0LW91dHB1dA==".to_owned(),
            next_offset: 20,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        }
        .into();
        let debug = format!("{chunk:?}");
        assert!(!debug.contains("c2VjcmV0"));
        assert!(debug.contains("data_base64_len"));
    }
}
