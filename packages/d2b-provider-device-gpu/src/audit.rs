//! Path-free GPU audit records.

use crate::GpuEffectError;

/// Closed brokered GPU operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuAuditOperation {
    /// Open an opaque GPU device grant.
    OpenDevice,
    /// Spawn the full GPU worker.
    SpawnGpu,
    /// Spawn the render-node worker.
    SpawnRenderNode,
    /// Spawn the video worker.
    SpawnVideo,
    /// Adopt an existing worker.
    Adopt,
    /// Close an owned worker.
    Close,
}

/// Closed audit outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuAuditOutcome {
    /// The operation succeeded.
    Success,
    /// The operation was denied.
    Denied,
    /// The operation failed.
    Failure,
}

/// Bounded GPU audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuAuditRecord {
    /// Closed operation.
    pub operation: GpuAuditOperation,
    /// Closed outcome.
    pub outcome: GpuAuditOutcome,
    /// Stable error class, or `none`.
    pub error: &'static str,
    /// Opaque correlation token, never rendered as text.
    correlation: [u8; 16],
}

impl GpuAuditRecord {
    /// Construct a path-free audit record.
    pub const fn new(
        operation: GpuAuditOperation,
        outcome: GpuAuditOutcome,
        error: Option<GpuEffectError>,
        correlation: [u8; 16],
    ) -> Self {
        Self {
            operation,
            outcome,
            error: match error {
                Some(error) => error.code(),
                None => "none",
            },
            correlation,
        }
    }

    /// Borrow the opaque correlation value for the Core outbox join.
    pub const fn correlation(&self) -> &[u8; 16] {
        &self.correlation
    }
}
