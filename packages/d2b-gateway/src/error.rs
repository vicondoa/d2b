//! `GatewayError`: a transparent map from gateway-orchestration failures to
//! the constellation [`ErrorKind`] (and thence to stable CLI error slugs).
//! No stringly errors leak across the boundary.

use d2b_realm_core::{ConstellationError, ErrorKind};

/// A gateway display-orchestration failure. Each variant maps to exactly one
/// [`ErrorKind`]; the gateway never surfaces a raw provider/relay error string
/// (those are redacted at the boundary — see the audit contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayError {
    /// The target's realm has no entrypoint in the table.
    NoRealmEntrypoint,
    /// The gateway-mode daemon is unreachable / not in gateway mode.
    GatewayUnavailable,
    /// A session already exists for the target (single-session cap).
    Busy,
    /// The same operation id was reused with a different request (non-retryable
    /// idempotency conflict, distinct from a retryable busy/in-progress).
    Conflict,
    /// An internal gateway invariant was violated (e.g. an id-source collision);
    /// fail-closed rather than corrupt session accounting.
    Internal,
    /// A per-realm/principal/session quota or buffer ceiling was exceeded.
    QuotaExceeded,
    /// The workload provider failed to allocate / exec the agent.
    ProviderAllocationFailed,
    /// The relay transport could not be reached / armed.
    RelayUnavailable,
    /// The provider does not advertise window forwarding.
    MissingWindowForwarding,
    /// A lifecycle step exceeded its timeout.
    Timeout,
    /// The session was cancelled / closed before completing.
    Cancelled,
    /// A required audit/denial record could not be durably written
    /// (fail-closed).
    AuditUnavailable,
}

impl GatewayError {
    /// The constellation error kind this maps to.
    pub fn kind(&self) -> ErrorKind {
        match self {
            GatewayError::NoRealmEntrypoint => ErrorKind::NoRealmEntrypoint,
            GatewayError::GatewayUnavailable => ErrorKind::GatewayUnavailable,
            GatewayError::Busy => ErrorKind::OperationInProgress,
            GatewayError::Conflict => ErrorKind::IdempotencyKeyConflict,
            GatewayError::Internal => ErrorKind::GatewayUnavailable,
            GatewayError::QuotaExceeded => ErrorKind::Backpressure,
            GatewayError::ProviderAllocationFailed => ErrorKind::ProviderAllocationFailed,
            GatewayError::RelayUnavailable => ErrorKind::RelayUnavailable,
            GatewayError::MissingWindowForwarding => ErrorKind::UnsupportedFeature,
            GatewayError::Timeout => ErrorKind::Timeout,
            GatewayError::Cancelled => ErrorKind::Cancelled,
            GatewayError::AuditUnavailable => ErrorKind::AuditUnavailable,
        }
    }

    /// The stable CLI error slug (matches the generated error-codes contract).
    pub fn slug(&self) -> &'static str {
        match self {
            GatewayError::NoRealmEntrypoint => "missing-realm-entrypoint",
            GatewayError::GatewayUnavailable => "gateway-unavailable",
            GatewayError::Busy => "gateway-busy",
            GatewayError::Conflict => "idempotency-conflict",
            GatewayError::Internal => "gateway-internal",
            GatewayError::QuotaExceeded => "gateway-quota-exceeded",
            GatewayError::ProviderAllocationFailed => "provider-allocation-failed",
            GatewayError::RelayUnavailable => "relay-unavailable",
            GatewayError::MissingWindowForwarding => "missing-window-forwarding",
            GatewayError::Timeout => "gateway-timeout",
            GatewayError::Cancelled => "gateway-cancelled",
            GatewayError::AuditUnavailable => "audit-unavailable",
        }
    }
}

impl From<GatewayError> for ConstellationError {
    fn from(e: GatewayError) -> Self {
        // The message is the stable slug, never a raw provider/relay string.
        ConstellationError::new(e.kind(), e.slug())
    }
}

impl core::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.slug())
    }
}

impl std::error::Error for GatewayError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_stable_and_distinct() {
        for (e, slug, kind) in cases() {
            assert_eq!(e.slug(), slug, "slug for {e:?}");
            assert_eq!(e.kind(), kind, "kind for {e:?}");
        }
        let mut slugs: Vec<&str> = cases().iter().map(|(_, s, _)| *s).collect();
        let n = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), n, "every gateway error slug must be distinct");
    }

    /// The exact, stable (variant -> slug, kind) contract. A breaking change
    /// to any slug or kind mapping fails here, not just a uniqueness check.
    fn cases() -> Vec<(GatewayError, &'static str, ErrorKind)> {
        vec![
            (
                GatewayError::NoRealmEntrypoint,
                "missing-realm-entrypoint",
                ErrorKind::NoRealmEntrypoint,
            ),
            (
                GatewayError::GatewayUnavailable,
                "gateway-unavailable",
                ErrorKind::GatewayUnavailable,
            ),
            (
                GatewayError::Busy,
                "gateway-busy",
                ErrorKind::OperationInProgress,
            ),
            (
                GatewayError::Conflict,
                "idempotency-conflict",
                ErrorKind::IdempotencyKeyConflict,
            ),
            (
                GatewayError::Internal,
                "gateway-internal",
                ErrorKind::GatewayUnavailable,
            ),
            (
                GatewayError::QuotaExceeded,
                "gateway-quota-exceeded",
                ErrorKind::Backpressure,
            ),
            (
                GatewayError::ProviderAllocationFailed,
                "provider-allocation-failed",
                ErrorKind::ProviderAllocationFailed,
            ),
            (
                GatewayError::RelayUnavailable,
                "relay-unavailable",
                ErrorKind::RelayUnavailable,
            ),
            (
                GatewayError::MissingWindowForwarding,
                "missing-window-forwarding",
                ErrorKind::UnsupportedFeature,
            ),
            (GatewayError::Timeout, "gateway-timeout", ErrorKind::Timeout),
            (
                GatewayError::Cancelled,
                "gateway-cancelled",
                ErrorKind::Cancelled,
            ),
            (
                GatewayError::AuditUnavailable,
                "audit-unavailable",
                ErrorKind::AuditUnavailable,
            ),
        ]
    }
}
