//! Typed, fail-closed client refusals.
//!
//! Every variant is a closed, low-cardinality code. No variant carries a Zone
//! path, a resource name, caller-supplied text, a descriptor, or any host,
//! socket, or store path, so a refusal is always safe for a diagnostic, an
//! audit record, or a metric reason label.

use core::{error::Error, fmt};

use d2b_contracts::v3::{ResourceErrorKind, RetryClass};

/// The closed set of client-local refusals.
///
/// [`ClientError::Remote`] is the one variant sourced from the peer, and it
/// reuses the canonical v3 [`ResourceErrorKind`] and [`RetryClass`] rather than
/// restating a parallel remote-error taxonomy. An authorization verdict reaches
/// the client only as such a remote error: the client never mints, infers, or
/// presents authority of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    /// The target input was internally inconsistent or matched ambiguously.
    InvalidTarget,
    /// No route record admits the requested owner.
    RouteUnavailable,
    /// A route record exists for the owner but not for the requested carriage.
    TransportPolicyMismatch,
    /// The requested service does not match the declared or resolved service.
    InvalidService,
    /// The method does not belong to the resolved service.
    InvalidMethod,
    /// Request metadata failed a structural or lifetime bound.
    InvalidMetadata,
    /// The request deadline had already passed when an attempt was prepared.
    DeadlineExpired,
    /// A mutating method requiring an idempotency key was called without one.
    IdempotencyRequired,
    /// The retry policy was exhausted without a terminal answer.
    RetryLimitExceeded,
    /// The caller cancelled the call.
    Cancelled,
    /// The underlying session was lost.
    SessionLost,
    /// The carriage failed before a terminal answer was observed.
    TransportFailed,
    /// A mutating call reached an ambiguous outcome and must not be retried.
    AmbiguousMutation,
    /// The peer violated the wire contract.
    ContractViolation,
    /// The peer refused or failed the call.
    Remote {
        /// The canonical v3 error kind reported by the peer.
        kind: ResourceErrorKind,
        /// The retry disposition the peer declared.
        retry: RetryClass,
    },
}

impl ClientError {
    /// The stable kebab-case label for a diagnostic or a metric reason label.
    ///
    /// [`ClientError::Remote`] collapses to a single fixed label so the peer's
    /// kind and retry class cannot inflate metric cardinality; a caller that
    /// needs those reads the variant fields directly.
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidTarget => "client-invalid-target",
            Self::RouteUnavailable => "client-route-unavailable",
            Self::TransportPolicyMismatch => "client-transport-policy-mismatch",
            Self::InvalidService => "client-invalid-service",
            Self::InvalidMethod => "client-invalid-method",
            Self::InvalidMetadata => "client-invalid-metadata",
            Self::DeadlineExpired => "client-deadline-expired",
            Self::IdempotencyRequired => "client-idempotency-required",
            Self::RetryLimitExceeded => "client-retry-limit-exceeded",
            Self::Cancelled => "client-cancelled",
            Self::SessionLost => "client-session-lost",
            Self::TransportFailed => "client-transport-failed",
            Self::AmbiguousMutation => "client-ambiguous-mutation",
            Self::ContractViolation => "client-contract-violation",
            Self::Remote { .. } => "client-remote",
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl Error for ClientError {}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[ClientError] = &[
        ClientError::InvalidTarget,
        ClientError::RouteUnavailable,
        ClientError::TransportPolicyMismatch,
        ClientError::InvalidService,
        ClientError::InvalidMethod,
        ClientError::InvalidMetadata,
        ClientError::DeadlineExpired,
        ClientError::IdempotencyRequired,
        ClientError::RetryLimitExceeded,
        ClientError::Cancelled,
        ClientError::SessionLost,
        ClientError::TransportFailed,
        ClientError::AmbiguousMutation,
        ClientError::ContractViolation,
        ClientError::Remote {
            kind: ResourceErrorKind::AuthorizationDenied,
            retry: RetryClass::Never,
        },
    ];

    #[test]
    fn every_label_is_unique_stable_and_client_prefixed() {
        let mut labels = ALL.iter().map(|error| error.label()).collect::<Vec<_>>();
        for error in ALL {
            assert!(error.label().starts_with("client-"), "{error:?}");
            assert_eq!(format!("{error}"), error.label());
        }
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), ALL.len());
    }

    #[test]
    fn a_remote_refusal_collapses_to_one_low_cardinality_label() {
        let mut labels = Vec::new();
        for kind in ResourceErrorKind::all() {
            for retry in [
                RetryClass::Never,
                RetryClass::Immediate,
                RetryClass::AfterDelay,
                RetryClass::Reauthorize,
            ] {
                labels.push(ClientError::Remote { kind: *kind, retry }.label());
            }
        }
        labels.dedup();
        assert_eq!(labels, vec!["client-remote"]);
    }
}
