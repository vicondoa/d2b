//! Redacted, closed-domain audit event definitions.

/// Closed transport lifecycle action suitable for an audit sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAuditAction {
    /// A validated transport was opened.
    Opened,
    /// A portal-owned monitor was retired.
    Closed,
    /// The service finalized its local monitor table.
    Finalized,
}
