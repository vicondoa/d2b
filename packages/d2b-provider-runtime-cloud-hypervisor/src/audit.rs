//! Bounded Cloud Hypervisor audit events.

/// Fixed audit operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudHypervisorAuditOperation {
    /// Launch.
    Launch,
    /// Adopt.
    Adopt,
    /// Finalize.
    Finalize,
    /// Guest-control health.
    Health,
}

/// One redacted audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudHypervisorAuditEvent {
    /// Fixed operation label.
    pub operation: CloudHypervisorAuditOperation,
    /// Stable result.
    pub success: bool,
}
