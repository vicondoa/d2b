//! Content-free authoritative notification audit.

use sha2::{Digest, Sha256};

/// Closed notification audit operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationAuditKind {
    /// A source session connected.
    SourceConnected,
    /// A source session disconnected.
    SourceDisconnected,
    /// A notification was accepted.
    Delivered,
    /// A notification was rejected.
    Rejected,
    /// An action was invoked.
    ActionInvoked,
}

impl NotificationAuditKind {
    /// Return the stable audit kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourceConnected => "notification-desktop/source-connected",
            Self::SourceDisconnected => "notification-desktop/source-disconnected",
            Self::Delivered => "notification-desktop/delivered",
            Self::Rejected => "notification-desktop/rejected",
            Self::ActionInvoked => "notification-desktop/action-invoked",
        }
    }
}

/// Content-free notification audit record.
pub struct NotificationAuditRecord {
    kind: NotificationAuditKind,
    subject_digest: String,
    operation_digest: String,
    outcome: &'static str,
}

impl NotificationAuditRecord {
    /// Construct a record after authentication and policy evaluation.
    pub fn new(
        kind: NotificationAuditKind,
        subject: &str,
        operation_id: &str,
        outcome: &'static str,
    ) -> Self {
        Self {
            kind,
            subject_digest: digest(subject),
            operation_digest: digest(operation_id),
            outcome,
        }
    }

    /// Render the bounded wire form.
    pub fn to_wire_record(&self) -> String {
        format!(
            "kind={} subject={} operation={} outcome={}",
            self.kind.as_str(),
            self.subject_digest,
            self.operation_digest,
            self.outcome
        )
    }
}

impl core::fmt::Debug for NotificationAuditRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationAuditRecord(<redacted>)")
    }
}

fn digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}
