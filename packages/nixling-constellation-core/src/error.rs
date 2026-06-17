//! Typed error surface for the constellation model (ADR 0032, ADR 0010
//! strict wire discipline). Errors are closed-enum and carry no secrets,
//! command output, store paths, or stream payload.

use crate::capability::Capability;
use serde::{Deserialize, Serialize};

/// Stable, closed-enum classification of a constellation failure. Codecs
/// map this to/from typed error frames; it never carries payload bytes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ErrorKind {
    /// A required capability was not advertised; fail closed (no fallback).
    CapabilityDenied,
    /// The principal is not authorized for this operation/stream.
    Unauthorized,
    /// No realm entrypoint matched the target.
    NoRealmEntrypoint,
    /// The selected gateway is unavailable.
    GatewayUnavailable,
    /// The provider could not allocate/start the workload.
    ProviderAllocationFailed,
    /// Relay reachability or relay authentication failed.
    RelayUnavailable,
    /// Peer-session authentication/handshake failed.
    AuthenticationFailed,
    /// Negotiated version/capability skew; fail closed.
    VersionSkew,
    /// Same idempotency key, request still in progress.
    OperationInProgress,
    /// Same idempotency key, different request fingerprint.
    IdempotencyKeyConflict,
    /// Idempotency key reused after its dedup retention window.
    IdempotencyKeyExpired,
    /// A bounded queue/credit window was exceeded.
    Backpressure,
    /// The operation/stream was cancelled.
    Cancelled,
    /// The operation/stream timed out.
    Timeout,
    /// A frame exceeded the negotiated cap before decode.
    FrameTooLarge,
    /// A malformed or unknown-shaped frame/field was rejected.
    MalformedFrame,
    /// The target name could not be parsed.
    InvalidTarget,
    /// Audit could not be recorded and policy is fail-closed.
    AuditUnavailable,
}

impl ErrorKind {
    /// A short, stable, machine-readable code (kebab-case).
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::CapabilityDenied => "capability-denied",
            ErrorKind::Unauthorized => "unauthorized",
            ErrorKind::NoRealmEntrypoint => "no-realm-entrypoint",
            ErrorKind::GatewayUnavailable => "gateway-unavailable",
            ErrorKind::ProviderAllocationFailed => "provider-allocation-failed",
            ErrorKind::RelayUnavailable => "relay-unavailable",
            ErrorKind::AuthenticationFailed => "authentication-failed",
            ErrorKind::VersionSkew => "version-skew",
            ErrorKind::OperationInProgress => "operation-in-progress",
            ErrorKind::IdempotencyKeyConflict => "idempotency-key-conflict",
            ErrorKind::IdempotencyKeyExpired => "idempotency-key-expired",
            ErrorKind::Backpressure => "backpressure",
            ErrorKind::Cancelled => "cancelled",
            ErrorKind::Timeout => "timeout",
            ErrorKind::FrameTooLarge => "frame-too-large",
            ErrorKind::MalformedFrame => "malformed-frame",
            ErrorKind::InvalidTarget => "invalid-target",
            ErrorKind::AuditUnavailable => "audit-unavailable",
        }
    }
}

/// A typed constellation error: a [`ErrorKind`] plus a bounded,
/// operator-safe message. The message MUST NOT contain secrets, command
/// output, store paths, argv, or stream payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConstellationError {
    /// Stable classification.
    pub kind: ErrorKind,
    /// Bounded operator-actionable message (no payload/secrets).
    pub message: String,
}

impl ConstellationError {
    /// Construct a typed error.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Convenience: a capability-denied error naming the missing
    /// capability (the capability name is not a secret).
    pub fn capability_denied(missing: Capability) -> Self {
        Self::new(
            ErrorKind::CapabilityDenied,
            format!("required capability {missing:?} is not advertised"),
        )
    }
}

impl core::fmt::Display for ConstellationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.kind.code(), self.message)
    }
}

impl std::error::Error for ConstellationError {}
