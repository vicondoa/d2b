//! Bounded Azure VM audit events.

/// Fixed Azure VM audit operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureVmAuditOperation {
    /// Provision.
    Provision,
    /// Adopt.
    Adopt,
    /// Delete.
    Delete,
    /// Bootstrap enrollment.
    Bootstrap,
}

impl AzureVmAuditOperation {
    /// Return the stable label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provision => "provision",
            Self::Adopt => "adopt",
            Self::Delete => "delete",
            Self::Bootstrap => "bootstrap",
        }
    }
}

/// A redacted audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzureVmAuditEvent {
    /// Fixed operation.
    pub operation: AzureVmAuditOperation,
    /// Whether the operation succeeded.
    pub success: bool,
    /// Stable error code.
    pub error: Option<crate::AzureVmError>,
}
