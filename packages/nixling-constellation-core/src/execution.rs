//! Execution model (ADR 0032). A durable execution is referenced by an
//! [`crate::ids::ExecutionId`] so a retried request can rediscover it.

use crate::ids::{ExecutionId, WorkloadId};
use serde::{Deserialize, Serialize};

/// Coarse execution lifecycle state.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
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
}
