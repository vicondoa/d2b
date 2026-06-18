//! `GatewayError`: a transparent map from gateway-orchestration failures to
//! the constellation [`ErrorKind`] (and thence to stable CLI error slugs).
//! No stringly errors leak across the boundary.

use crate::handshake::HandshakeError;
use nixling_constellation_core::{ConstellationError, ErrorKind};

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
    /// A per-realm/principal/session quota or buffer ceiling was exceeded.
    QuotaExceeded,
    /// The workload provider failed to allocate / exec the agent.
    ProviderAllocationFailed,
    /// The relay transport could not be reached / armed.
    RelayUnavailable,
    /// The per-session display handshake failed (bytes never admitted).
    DisplayAuthFailed(HandshakeError),
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
            GatewayError::QuotaExceeded => ErrorKind::Backpressure,
            GatewayError::ProviderAllocationFailed => ErrorKind::ProviderAllocationFailed,
            GatewayError::RelayUnavailable => ErrorKind::RelayUnavailable,
            GatewayError::DisplayAuthFailed(_) => ErrorKind::AuthenticationFailed,
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
            GatewayError::QuotaExceeded => "gateway-quota-exceeded",
            GatewayError::ProviderAllocationFailed => "provider-allocation-failed",
            GatewayError::RelayUnavailable => "relay-unavailable",
            GatewayError::DisplayAuthFailed(_) => "display-auth-failed",
            GatewayError::MissingWindowForwarding => "missing-window-forwarding",
            GatewayError::Timeout => "gateway-timeout",
            GatewayError::Cancelled => "gateway-cancelled",
            GatewayError::AuditUnavailable => "audit-unavailable",
        }
    }
}

impl From<HandshakeError> for GatewayError {
    fn from(e: HandshakeError) -> Self {
        GatewayError::DisplayAuthFailed(e)
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
    fn handshake_error_maps_to_auth_failed() {
        let e: GatewayError = HandshakeError::Replay.into();
        assert_eq!(e.kind(), ErrorKind::AuthenticationFailed);
        assert_eq!(e.slug(), "display-auth-failed");
        let ce: ConstellationError = e.into();
        assert_eq!(ce.kind(), ErrorKind::AuthenticationFailed);
    }

    #[test]
    fn slugs_are_stable_and_distinct() {
        let all = [
            GatewayError::NoRealmEntrypoint,
            GatewayError::GatewayUnavailable,
            GatewayError::Busy,
            GatewayError::QuotaExceeded,
            GatewayError::ProviderAllocationFailed,
            GatewayError::RelayUnavailable,
            GatewayError::DisplayAuthFailed(HandshakeError::BadMac),
            GatewayError::MissingWindowForwarding,
            GatewayError::Timeout,
            GatewayError::Cancelled,
            GatewayError::AuditUnavailable,
        ];
        let mut slugs: Vec<&str> = all.iter().map(|e| e.slug()).collect();
        let n = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), n, "every gateway error slug must be distinct");
    }
}
