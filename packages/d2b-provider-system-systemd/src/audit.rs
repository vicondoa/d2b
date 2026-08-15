//! Redacted ProcessEffect audit projection.

use d2b_process_conformance::ProcessIdentityDigest;
use serde::Serialize;

/// Typed systemd process audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SystemdAuditOperation {
    /// Transient unit start.
    Start,
    /// Adoption after restart.
    Adopt,
    /// Exact transient unit stop.
    Stop,
}

/// No raw unit name, PID, path, or property fragment is exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemdProcessAudit {
    /// Operation class.
    pub operation: SystemdAuditOperation,
    /// Opaque process identity digest.
    pub process_identity: ProcessIdentityDigest,
    /// Whether the operation completed after the broker record boundary.
    pub durable: bool,
}

impl SystemdProcessAudit {
    /// Construct a bounded audit record.
    pub const fn new(
        operation: SystemdAuditOperation,
        process_identity: ProcessIdentityDigest,
        durable: bool,
    ) -> Self {
        Self {
            operation,
            process_identity,
            durable,
        }
    }
}
