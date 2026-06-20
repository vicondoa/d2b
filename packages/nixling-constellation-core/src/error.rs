//! Typed error surface for the constellation model (ADR 0032, ADR 0010
//! strict wire discipline). Errors are closed-enum and carry no secrets,
//! command output, store paths, or stream payload.

use crate::capability::Capability;
use serde::{Deserialize, Deserializer, Serialize};

/// Stable, closed-enum classification of a constellation failure. Codecs
/// map this to/from typed error frames; it never carries payload bytes.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
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
    /// The requested feature/transport mode is not implemented by this
    /// build (a typed, non-fallback refusal).
    UnsupportedFeature,
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
            ErrorKind::UnsupportedFeature => "unsupported-feature",
        }
    }
}

/// Maximum length of an operator-safe error message. Messages are
/// constructed by trusted code and never carry payload, but the length is
/// bounded so a message can never become an unbounded side channel.
pub const MAX_MESSAGE_LEN: usize = 256;
/// Maximum length of a bounded capability-negotiation fingerprint.
pub const MAX_FINGERPRINT_LEN: usize = 64;

/// A typed constellation error: a [`ErrorKind`], an optional structured
/// missing capability (for `CapabilityDenied`), plus a bounded,
/// operator-safe message. The message MUST NOT contain secrets, command
/// output, store paths, argv, or stream payload, and is bounded at decode
/// so it can never be an unbounded side channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConstellationError {
    /// Stable classification. Private so the `CapabilityDenied` =>
    /// `Some(capability)` invariant cannot be broken by external mutation
    /// or struct-literal construction.
    kind: ErrorKind,
    /// The structured missing capability, present for `CapabilityDenied`
    /// so callers route on a stable field rather than parsing the message.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    capability: Option<Capability>,
    /// Fingerprint of the negotiated capability set used for the denial.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(length(max = 64))]
    negotiated_capability_fingerprint: Option<String>,
    /// Bounded operator-actionable message (no payload/secrets).
    #[schemars(length(max = 256))]
    message: String,
}

impl ConstellationError {
    /// Construct a typed error. The message is bounded to
    /// [`MAX_MESSAGE_LEN`] bytes (truncated on a char boundary). Use
    /// [`ConstellationError::capability_denied`] for `CapabilityDenied`.
    ///
    /// To keep the `CapabilityDenied => Some(capability)` invariant airtight
    /// in every build, passing `CapabilityDenied` here (a misuse — there is
    /// no capability to attach) is downgraded to `Unauthorized` rather than
    /// producing a denial with no structured capability.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        debug_assert!(
            kind != ErrorKind::CapabilityDenied,
            "use ConstellationError::capability_denied for capability denials"
        );
        let kind = if kind == ErrorKind::CapabilityDenied {
            ErrorKind::Unauthorized
        } else {
            kind
        };
        Self {
            kind,
            capability: None,
            negotiated_capability_fingerprint: None,
            message: bound_message(message.into()),
        }
    }

    /// Convenience: a capability-denied error carrying the structured
    /// missing capability (the capability name is not a secret).
    pub fn capability_denied(missing: Capability) -> Self {
        Self::capability_denied_with_fingerprint(missing, None)
    }

    /// Capability-denied error with the negotiated set fingerprint that
    /// produced the refusal.
    pub fn capability_denied_with_fingerprint(
        missing: Capability,
        negotiated_fingerprint: impl Into<Option<String>>,
    ) -> Self {
        Self {
            kind: ErrorKind::CapabilityDenied,
            capability: Some(missing),
            negotiated_capability_fingerprint: negotiated_fingerprint.into().map(bound_fingerprint),
            message: bound_message(format!(
                "required capability {} is not advertised",
                missing.code()
            )),
        }
    }

    /// The stable error classification.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The structured missing capability, if this is a capability denial.
    /// Guaranteed `Some` whenever [`kind`](Self::kind) is `CapabilityDenied`.
    pub fn missing_capability(&self) -> Option<Capability> {
        self.capability
    }

    /// Fingerprint of the negotiated capability set used for this denial.
    pub fn negotiated_capability_fingerprint(&self) -> Option<&str> {
        self.negotiated_capability_fingerprint.as_deref()
    }

    /// The bounded, operator-safe message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

// Fail-closed decode: the message is bounded at decode (truncated to
// [`MAX_MESSAGE_LEN`]) so a `TypedError` frame can never carry an unbounded
// message that `Display`/`Debug` would then emit.
impl<'de> Deserialize<'de> for ConstellationError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            kind: ErrorKind,
            #[serde(default)]
            capability: Option<Capability>,
            #[serde(default)]
            negotiated_capability_fingerprint: Option<String>,
            message: String,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.kind == ErrorKind::CapabilityDenied && raw.capability.is_none() {
            return Err(serde::de::Error::custom(
                "capability-denied error must carry the missing capability",
            ));
        }
        if raw.kind != ErrorKind::CapabilityDenied
            && raw.negotiated_capability_fingerprint.is_some()
        {
            return Err(serde::de::Error::custom(
                "only capability-denied errors may carry a negotiated capability fingerprint",
            ));
        }
        Ok(Self {
            kind: raw.kind,
            capability: raw.capability,
            negotiated_capability_fingerprint: raw
                .negotiated_capability_fingerprint
                .map(bound_fingerprint),
            message: bound_message(raw.message),
        })
    }
}

/// Truncate a message to [`MAX_MESSAGE_LEN`] on a UTF-8 char boundary.
fn bound_message(mut s: String) -> String {
    if s.len() > MAX_MESSAGE_LEN {
        let mut end = MAX_MESSAGE_LEN;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

fn bound_fingerprint(mut s: String) -> String {
    if s.len() > MAX_FINGERPRINT_LEN {
        let mut end = MAX_FINGERPRINT_LEN;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

impl core::fmt::Display for ConstellationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.kind.code(), self.message)
    }
}

impl std::error::Error for ConstellationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_denied_carries_structured_capability() {
        let e = ConstellationError::capability_denied(Capability::WindowForwarding);
        assert_eq!(e.kind(), ErrorKind::CapabilityDenied);
        assert_eq!(e.missing_capability(), Some(Capability::WindowForwarding));
        assert!(e.message().contains("window-forwarding"));
    }

    #[test]
    fn capability_denied_can_carry_negotiated_fingerprint() {
        let e = ConstellationError::capability_denied_with_fingerprint(
            Capability::WindowForwarding,
            Some("cap-v1-cbf29ce484222325".to_owned()),
        );
        assert_eq!(
            e.negotiated_capability_fingerprint(),
            Some("cap-v1-cbf29ce484222325")
        );
    }

    #[test]
    fn deserialize_bounds_message() {
        let overlong = "x".repeat(MAX_MESSAGE_LEN + 50);
        let json = format!("{{\"kind\":\"timeout\",\"message\":\"{overlong}\"}}");
        let e: ConstellationError = serde_json::from_str(&json).unwrap();
        assert!(e.message().len() <= MAX_MESSAGE_LEN);
    }

    #[test]
    fn deserialize_rejects_unknown_fields() {
        let json = "{\"kind\":\"timeout\",\"message\":\"x\",\"extra\":1}";
        assert!(serde_json::from_str::<ConstellationError>(json).is_err());
    }

    #[test]
    fn deserialize_rejects_capability_denied_without_capability() {
        // CapabilityDenied must carry the structured missing capability.
        let bad = "{\"kind\":\"capability-denied\",\"message\":\"x\"}";
        assert!(serde_json::from_str::<ConstellationError>(bad).is_err());
        let ok = "{\"kind\":\"capability-denied\",\"capability\":\"window-forwarding\",\"message\":\"x\"}";
        let e: ConstellationError = serde_json::from_str(ok).unwrap();
        assert_eq!(e.missing_capability(), Some(Capability::WindowForwarding));
    }
}
