//! Codec-neutral resource-plane errors.

use serde::{Deserialize, Deserializer, Serialize};

use super::{ZoneRevision, resource_schema::validate_canonical_string};

/// Maximum bytes in a redacted resource error reason.
pub const MAX_RESOURCE_ERROR_REASON_BYTES: usize = 512;
/// Maximum retry delay carried by a resource error.
pub const MAX_RESOURCE_ERROR_RETRY_AFTER_MS: u32 = 86_400_000;

/// Closed resource-plane error classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceErrorKind {
    ResourceNotFound,
    ResourceAlreadyExists,
    ResourceConflict,
    ResourceSchemaInvalid,
    ResourceRefInvalid,
    ResourceOwnerCycle,
    ResourceOwnerDepth,
    ResourceFinalizerDenied,
    ResourceProviderUnavailable,
    ResourceControllerMismatch,
    ResourceStatusOwnerMismatch,
    StatusOversize,
    StatusProviderSchemaInvalid,
    StatusProviderOverlap,
    SpecProviderSchemaInvalid,
    SpecProviderShadow,
    UnsupportedCapability,
    ExpeditedNotAuthorized,
    ExpeditedQuotaExceeded,
    ExpeditedReconcilePending,
    UpgradeRequired,
    EndpointResolveDenied,
    RelayDenied,
    RoleRelayGrantRestricted,
    AuthorizationDenied,
    RevisionExpired,
    Backpressure,
    Timeout,
    Cancelled,
    ResourcePlaneUnavailable,
    InternalIntegrityFailure,
}

impl ResourceErrorKind {
    /// Exact stable wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResourceNotFound => "resource-not-found",
            Self::ResourceAlreadyExists => "resource-already-exists",
            Self::ResourceConflict => "resource-conflict",
            Self::ResourceSchemaInvalid => "resource-schema-invalid",
            Self::ResourceRefInvalid => "resource-ref-invalid",
            Self::ResourceOwnerCycle => "resource-owner-cycle",
            Self::ResourceOwnerDepth => "resource-owner-depth",
            Self::ResourceFinalizerDenied => "resource-finalizer-denied",
            Self::ResourceProviderUnavailable => "resource-provider-unavailable",
            Self::ResourceControllerMismatch => "resource-controller-mismatch",
            Self::ResourceStatusOwnerMismatch => "resource-status-owner-mismatch",
            Self::StatusOversize => "status-oversize",
            Self::StatusProviderSchemaInvalid => "status-provider-schema-invalid",
            Self::StatusProviderOverlap => "status-provider-overlap",
            Self::SpecProviderSchemaInvalid => "spec-provider-schema-invalid",
            Self::SpecProviderShadow => "spec-provider-shadow",
            Self::UnsupportedCapability => "unsupported-capability",
            Self::ExpeditedNotAuthorized => "expedited-not-authorized",
            Self::ExpeditedQuotaExceeded => "expedited-quota-exceeded",
            Self::ExpeditedReconcilePending => "expedited-reconcile-pending",
            Self::UpgradeRequired => "upgrade-required",
            Self::EndpointResolveDenied => "endpoint-resolve-denied",
            Self::RelayDenied => "relay-denied",
            Self::RoleRelayGrantRestricted => "role-relay-grant-restricted",
            Self::AuthorizationDenied => "authorization-denied",
            Self::RevisionExpired => "revision-expired",
            Self::Backpressure => "backpressure",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::ResourcePlaneUnavailable => "resource-plane-unavailable",
            Self::InternalIntegrityFailure => "internal-integrity-failure",
        }
    }

    /// Exhaustive stable variant order.
    pub const fn all() -> &'static [Self; 31] {
        &[
            Self::ResourceNotFound,
            Self::ResourceAlreadyExists,
            Self::ResourceConflict,
            Self::ResourceSchemaInvalid,
            Self::ResourceRefInvalid,
            Self::ResourceOwnerCycle,
            Self::ResourceOwnerDepth,
            Self::ResourceFinalizerDenied,
            Self::ResourceProviderUnavailable,
            Self::ResourceControllerMismatch,
            Self::ResourceStatusOwnerMismatch,
            Self::StatusOversize,
            Self::StatusProviderSchemaInvalid,
            Self::StatusProviderOverlap,
            Self::SpecProviderSchemaInvalid,
            Self::SpecProviderShadow,
            Self::UnsupportedCapability,
            Self::ExpeditedNotAuthorized,
            Self::ExpeditedQuotaExceeded,
            Self::ExpeditedReconcilePending,
            Self::UpgradeRequired,
            Self::EndpointResolveDenied,
            Self::RelayDenied,
            Self::RoleRelayGrantRestricted,
            Self::AuthorizationDenied,
            Self::RevisionExpired,
            Self::Backpressure,
            Self::Timeout,
            Self::Cancelled,
            Self::ResourcePlaneUnavailable,
            Self::InternalIntegrityFailure,
        ]
    }
}

/// Closed client retry disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetryClass {
    Never,
    Immediate,
    AfterDelay,
    Reauthorize,
}

/// A bounded redacted error reason.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ResourceErrorReason(String);

impl ResourceErrorReason {
    /// Validate a reason without normalizing or echoing rejected input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ResourceErrorValidation> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_RESOURCE_ERROR_REASON_BYTES
            || validate_canonical_string(&value).is_err()
        {
            return Err(ResourceErrorValidation::InvalidReason);
        }
        Ok(Self(value))
    }

    /// Borrow the reason for an explicitly authorized presentation surface.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for ResourceErrorReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ResourceErrorReason(<{} bytes>)", self.0.len())
    }
}

impl<'de> Deserialize<'de> for ResourceErrorReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Typed resource-plane domain error.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceError {
    kind: ResourceErrorKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<ZoneRevision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u32>,
    retry_class: RetryClass,
    reason: ResourceErrorReason,
}

impl ResourceError {
    /// Construct an error while enforcing revision and retry field invariants.
    pub fn new(
        kind: ResourceErrorKind,
        current_revision: Option<ZoneRevision>,
        retry_after_ms: Option<u32>,
        retry_class: RetryClass,
        reason: ResourceErrorReason,
    ) -> Result<Self, ResourceErrorValidation> {
        if current_revision.is_some()
            && !matches!(
                kind,
                ResourceErrorKind::ResourceConflict | ResourceErrorKind::RevisionExpired
            )
        {
            return Err(ResourceErrorValidation::RevisionNotAllowed);
        }
        if retry_after_ms.is_some_and(|delay| {
            delay == 0
                || delay > MAX_RESOURCE_ERROR_RETRY_AFTER_MS
                || retry_class != RetryClass::AfterDelay
        }) || (retry_class == RetryClass::AfterDelay && retry_after_ms.is_none())
        {
            return Err(ResourceErrorValidation::InvalidRetryAfter);
        }
        Ok(Self {
            kind,
            current_revision,
            retry_after_ms,
            retry_class,
            reason,
        })
    }

    /// Construct a non-retryable error with no optional fields.
    pub fn terminal(kind: ResourceErrorKind, reason: &'static str) -> Self {
        Self::new(
            kind,
            None,
            None,
            RetryClass::Never,
            ResourceErrorReason::parse(reason).expect("static error reason is valid"),
        )
        .expect("terminal error fields are valid")
    }

    /// Return the stable error kind.
    pub const fn kind(&self) -> ResourceErrorKind {
        self.kind
    }

    /// Return a readable current revision only when one was authorized.
    pub const fn current_revision(&self) -> Option<ZoneRevision> {
        self.current_revision
    }

    /// Return the retry delay.
    pub const fn retry_after_ms(&self) -> Option<u32> {
        self.retry_after_ms
    }

    /// Return the retry disposition.
    pub const fn retry_class(&self) -> RetryClass {
        self.retry_class
    }

    /// Borrow the redacted reason.
    pub const fn reason(&self) -> &ResourceErrorReason {
        &self.reason
    }
}

impl core::fmt::Debug for ResourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceError")
            .field("kind", &self.kind)
            .field("current_revision", &self.current_revision)
            .field("retry_after_ms", &self.retry_after_ms)
            .field("retry_class", &self.retry_class)
            .field("reason", &"<redacted>")
            .finish()
    }
}

/// Invalid typed error construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceErrorValidation {
    InvalidReason,
    RevisionNotAllowed,
    InvalidRetryAfter,
}

impl core::fmt::Display for ResourceErrorValidation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidReason => f.write_str("resource error reason is invalid"),
            Self::RevisionNotAllowed => {
                f.write_str("currentRevision is not allowed for this error kind")
            }
            Self::InvalidRetryAfter => f.write_str("resource error retry fields are inconsistent"),
        }
    }
}

impl std::error::Error for ResourceErrorValidation {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_set_and_wire_names_are_frozen() {
        assert_eq!(ResourceErrorKind::all().len(), 31);
        let names = ResourceErrorKind::all()
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "resource-not-found",
                "resource-already-exists",
                "resource-conflict",
                "resource-schema-invalid",
                "resource-ref-invalid",
                "resource-owner-cycle",
                "resource-owner-depth",
                "resource-finalizer-denied",
                "resource-provider-unavailable",
                "resource-controller-mismatch",
                "resource-status-owner-mismatch",
                "status-oversize",
                "status-provider-schema-invalid",
                "status-provider-overlap",
                "spec-provider-schema-invalid",
                "spec-provider-shadow",
                "unsupported-capability",
                "expedited-not-authorized",
                "expedited-quota-exceeded",
                "expedited-reconcile-pending",
                "upgrade-required",
                "endpoint-resolve-denied",
                "relay-denied",
                "role-relay-grant-restricted",
                "authorization-denied",
                "revision-expired",
                "backpressure",
                "timeout",
                "cancelled",
                "resource-plane-unavailable",
                "internal-integrity-failure",
            ]
        );
    }

    #[test]
    fn optional_error_fields_are_narrowed() {
        let reason = ResourceErrorReason::parse("current revision changed").unwrap();
        assert_eq!(
            ResourceError::new(
                ResourceErrorKind::AuthorizationDenied,
                Some(ZoneRevision::new(4)),
                None,
                RetryClass::Never,
                reason.clone(),
            ),
            Err(ResourceErrorValidation::RevisionNotAllowed)
        );
        assert_eq!(
            ResourceError::new(
                ResourceErrorKind::Backpressure,
                None,
                Some(0),
                RetryClass::AfterDelay,
                reason,
            ),
            Err(ResourceErrorValidation::InvalidRetryAfter)
        );
    }
}
