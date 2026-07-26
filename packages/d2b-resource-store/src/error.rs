//! Storage-neutral resource store errors.

use d2b_contracts::v3::{RetryClass, ZoneRevision};

/// Closed store error classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreErrorKind {
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
    StoreIntegrityFailure,
    StoreBackpressure,
    StoreQuarantined,
}

impl StoreErrorKind {
    /// Exact stable contract spelling.
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
            Self::StoreIntegrityFailure => "store-integrity-failure",
            Self::StoreBackpressure => "store-backpressure",
            Self::StoreQuarantined => "store-quarantined",
        }
    }

    /// Exhaustive stable variant order.
    pub const fn all() -> &'static [Self; 34] {
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
            Self::StoreIntegrityFailure,
            Self::StoreBackpressure,
            Self::StoreQuarantined,
        ]
    }
}

/// Store error with only API-safe optional metadata.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreError {
    kind: StoreErrorKind,
    current_revision: Option<ZoneRevision>,
    retry_after_ms: Option<u32>,
    retry_class: RetryClass,
    reason_code: &'static str,
}

impl StoreError {
    /// Construct a store error from a fixed, non-sensitive reason code.
    pub const fn new(
        kind: StoreErrorKind,
        current_revision: Option<ZoneRevision>,
        retry_after_ms: Option<u32>,
        retry_class: RetryClass,
        reason_code: &'static str,
    ) -> Self {
        Self {
            kind,
            current_revision,
            retry_after_ms,
            retry_class,
            reason_code,
        }
    }

    pub const fn kind(&self) -> StoreErrorKind {
        self.kind
    }

    pub const fn current_revision(&self) -> Option<ZoneRevision> {
        self.current_revision
    }

    pub const fn retry_after_ms(&self) -> Option<u32> {
        self.retry_after_ms
    }

    pub const fn retry_class(&self) -> RetryClass {
        self.retry_class
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

impl core::fmt::Debug for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoreError")
            .field("kind", &self.kind)
            .field("current_revision", &self.current_revision)
            .field("retry_after_ms", &self.retry_after_ms)
            .field("retry_class", &self.retry_class)
            .field("reason_code", &self.reason_code)
            .finish()
    }
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.kind.as_str())
    }
}

impl std::error::Error for StoreError {}
