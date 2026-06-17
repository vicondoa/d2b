//! Workload summaries and lifecycle state (ADR 0032). A workload is a VM,
//! provider session, or sandbox addressed by a stable id/alias.

use crate::capability::CapabilitySet;
use crate::ids::{NodeId, WorkloadId};
use serde::{Deserialize, Serialize};

/// Coarse workload lifecycle state.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadState {
    /// Declared/known but not running.
    Stopped,
    /// Allocation/start in progress.
    Starting,
    /// Running.
    Running,
    /// Stop in progress.
    Stopping,
    /// Terminal failure.
    Failed,
}

/// A selector for listing workloads.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadSelector {
    /// All workloads on the node.
    All,
    /// A single workload by id.
    One(WorkloadId),
}

/// A workload's advertised summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkloadSummary {
    /// Stable operator-facing alias/id.
    pub id: WorkloadId,
    /// Node that owns this workload.
    pub node: NodeId,
    /// Current state.
    pub state: WorkloadState,
    /// Capabilities this workload can present.
    pub capabilities: CapabilitySet,
}
