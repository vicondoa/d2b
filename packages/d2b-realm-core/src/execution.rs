//! Execution model (ADR 0032). A durable execution is referenced by an
//! [`crate::ids::ExecutionId`] so a retried request can rediscover it.
//! Arguments, environment, cwd, stdio, and log bytes are payload and never
//! appear in these metadata DTOs.

use crate::ids::{ExecutionId, StreamCursor, WorkloadId};
use crate::token::ProtocolToken;
use serde::{Deserialize, Deserializer, Serialize};
use std::num::NonZeroU32;

/// Coarse execution lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExecState {
    /// Accepted, not yet started.
    Pending,
    /// Running.
    Running,
    /// Exited (see `exit_code`).
    Exited,
    /// Cancelled.
    Cancelled,
    /// Failed before a normal exit code was available.
    Failed,
}

impl ExecState {
    /// Whether the state is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Cancelled | Self::Failed)
    }
}

/// Generation/boot binding for reconnect safety. A reconnect request must
/// match the generation the durable execution was created under; otherwise it
/// is stale and must fail closed rather than attach to a different boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionGeneration {
    /// Guest boot identifier or equivalent provider boot token.
    pub guest_boot_id: ProtocolToken,
    /// Workload generation/closure token.
    pub workload_generation: ProtocolToken,
}

/// Execution attachment mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExecAttachMode {
    /// Attach to the interactive owner stream.
    Attached,
    /// Detached job with retained logs.
    Detached,
}

/// Durable execution start request metadata. Command details live in the
/// opaque operation body and are never logged by this DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecStartRequest {
    pub execution_id: ExecutionId,
    pub workload: WorkloadId,
    pub generation: ExecutionGeneration,
    pub attach_mode: ExecAttachMode,
    pub tty: bool,
}

/// Attach/reconnect request for a durable execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecAttachRequest {
    pub execution_id: ExecutionId,
    pub generation: ExecutionGeneration,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stdout_cursor: Option<StreamCursor>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stderr_cursor: Option<StreamCursor>,
}

/// Bounded retained-log request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecLogsRequest {
    pub execution_id: ExecutionId,
    pub generation: ExecutionGeneration,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cursor: Option<StreamCursor>,
    pub max_bytes: NonZeroU32,
}

impl<'de> Deserialize<'de> for ExecLogsRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            execution_id: ExecutionId,
            generation: ExecutionGeneration,
            #[serde(default)]
            cursor: Option<StreamCursor>,
            max_bytes: NonZeroU32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            execution_id: raw.execution_id,
            generation: raw.generation,
            cursor: raw.cursor,
            max_bytes: raw.max_bytes,
        })
    }
}

/// Idempotent cancel request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecCancelRequest {
    pub execution_id: ExecutionId,
    pub generation: ExecutionGeneration,
}

/// A durable execution summary. Argv/stdio/env are payload and never
/// appear here; only bounded metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSummary {
    /// Durable execution id.
    pub id: ExecutionId,
    /// Workload the execution runs in.
    pub workload: WorkloadId,
    /// State.
    pub state: ExecState,
    /// Exit code once `state == Exited`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exit_code: Option<i32>,
    /// Whether a TTY was allocated (metadata only).
    pub tty: bool,
    /// Generation this execution is bound to.
    pub generation: ExecutionGeneration,
    /// Attachment mode selected at start.
    pub attach_mode: ExecAttachMode,
    /// Last retained stdout cursor, if available.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stdout_cursor: Option<StreamCursor>,
    /// Last retained stderr cursor, if available.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stderr_cursor: Option<StreamCursor>,
}

impl ExecutionSummary {
    /// True when a reconnect with `generation` is safe to consider.
    pub fn generation_matches(&self, generation: &ExecutionGeneration) -> bool {
        &self.generation == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation() -> ExecutionGeneration {
        ExecutionGeneration {
            guest_boot_id: ProtocolToken::parse("boot-1").unwrap(),
            workload_generation: ProtocolToken::parse("gen-1").unwrap(),
        }
    }

    #[test]
    fn exec_logs_request_requires_nonzero_bound() {
        let zero = "{\"execution_id\":\"exec-1\",\"generation\":{\"guest_boot_id\":\"boot-1\",\
                    \"workload_generation\":\"gen-1\"},\"max_bytes\":0}";
        assert!(serde_json::from_str::<ExecLogsRequest>(zero).is_err());
        let ok = "{\"execution_id\":\"exec-1\",\"generation\":{\"guest_boot_id\":\"boot-1\",\
                  \"workload_generation\":\"gen-1\"},\"max_bytes\":4096}";
        assert!(serde_json::from_str::<ExecLogsRequest>(ok).is_ok());
    }

    #[test]
    fn summaries_carry_only_bounded_metadata() {
        let summary = ExecutionSummary {
            id: ExecutionId::parse("exec-1").unwrap(),
            workload: WorkloadId::parse("workload-a").unwrap(),
            state: ExecState::Running,
            exit_code: None,
            tty: true,
            generation: generation(),
            attach_mode: ExecAttachMode::Detached,
            stdout_cursor: Some(StreamCursor::parse("cur-out").unwrap()),
            stderr_cursor: None,
        };
        assert!(summary.generation_matches(&generation()));
        let json = serde_json::to_string(&summary).unwrap();
        for forbidden in ["argv", "TOKEN=", "secret", "/nix/store", "stdout bytes"] {
            assert!(
                !json.contains(forbidden),
                "summary leaked {forbidden}: {json}"
            );
        }
    }
}
