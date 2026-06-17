//! Minimal provider DTOs (ADR 0032). These are deliberately thin for the
//! Wave 0 skeleton: enough to give the trait signatures meaning without
//! prejudging later-wave detail. Operation/stream payloads stay opaque.

use nixling_constellation_core::{
    CapabilitySet, ExecutionId, NodeId, ProviderId, StreamId, StreamKind, WorkloadId,
    WorkloadSelector, WorkloadSummary,
};
use serde::{Deserialize, Serialize};

/// A request to plan/run a workload, addressed by a stable alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadSpec {
    /// Stable operator-facing alias for the workload.
    pub alias: WorkloadId,
}

/// An opaque, provider-resolved runtime plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePlan {
    /// Provider that produced the plan.
    pub provider: ProviderId,
    /// Workload the plan is for.
    pub workload: WorkloadId,
}

/// An opaque handle to a running runtime instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHandle {
    /// Workload the handle refers to.
    pub workload: WorkloadId,
}

/// Coarse runtime status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// Workload.
    pub workload: WorkloadId,
    /// Whether the runtime is currently running.
    pub running: bool,
}

/// Coarse workload status returned by a [`crate::WorkloadProvider`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadStatus {
    /// Workload.
    pub workload: WorkloadId,
    /// Whether the workload is currently running.
    pub running: bool,
}

/// A request to start an execution in a workload. Argv/env/stdio are
/// opaque payload carried elsewhere; only the workload + tty flag are
/// modeled here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecStartRequest {
    /// Workload to exec in.
    pub workload: WorkloadId,
    /// Whether a TTY is requested.
    pub tty: bool,
}

/// A request to open a named stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamOpenRequest {
    /// Stream id.
    pub id: StreamId,
    /// Stream kind (maps to a required capability).
    pub kind: StreamKind,
}

/// An opaque handle to an opened stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamHandle {
    /// Stream id.
    pub id: StreamId,
}

/// An accepted incoming stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomingStream {
    /// Stream id.
    pub id: StreamId,
    /// Stream kind.
    pub kind: StreamKind,
}

/// A display-session id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionId(pub String);

/// A request to open a display session for a workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionRequest {
    /// Workload presenting the UI.
    pub workload: WorkloadId,
}

/// An opaque handle to an open display session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionHandle {
    /// Session id.
    pub id: DisplaySessionId,
}

/// Re-export the selector + summary used by the workload trait.
pub use nixling_constellation_core::{WorkloadSelector as Selector, WorkloadSummary as Summary};

/// Convenience alias so trait signatures read cleanly.
pub type WorkloadList = Vec<WorkloadSummary>;

/// A node registration handle (transport listener side).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRegistration {
    /// Node being registered.
    pub node: NodeId,
}

/// A transport-level target to connect to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportTarget {
    /// Opaque transport endpoint reference (e.g. a relay rendezvous id).
    pub endpoint: String,
}

/// An opaque transport session (byte channel below the mux).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportSession {
    /// Opaque session label for diagnostics (no secrets).
    pub label: String,
}

/// An opaque transport listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportListener {
    /// Node the listener is registered for.
    pub node: NodeId,
}

/// A selector used by [`crate::WorkloadProvider::list`].
pub type ListSelector = WorkloadSelector;

/// An execution reference returned by exec.
pub type ExecRef = ExecutionId;

/// A capability descriptor bundle attached to provider summaries.
pub type Caps = CapabilitySet;
