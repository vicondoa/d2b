//! Storage-neutral resource store errors.

use d2b_contracts::v3::{MAX_BATCH_MUTATIONS, RetryClass, ZoneRevision};

/// Zero-based index of a mutation in a bounded commit batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MutationOrdinal(u8);

impl MutationOrdinal {
    /// Construct an ordinal inside the frozen batch bound.
    pub fn new(value: u32) -> Result<Self, MutationOrdinalError> {
        if usize::try_from(value).map_or(true, |value| value >= MAX_BATCH_MUTATIONS) {
            return Err(MutationOrdinalError);
        }
        Ok(Self(u8::try_from(value).map_err(|_| MutationOrdinalError)?))
    }

    pub const fn get(self) -> u32 {
        self.0 as u32
    }
}

/// Mutation ordinal exceeded the frozen commit-batch bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutationOrdinalError;

impl core::fmt::Display for MutationOrdinalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("mutation ordinal exceeds the commit-batch bound")
    }
}

impl std::error::Error for MutationOrdinalError {}

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
    mutation_ordinal: Option<MutationOrdinal>,
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
            mutation_ordinal: None,
            retry_after_ms,
            retry_class,
            reason_code,
        }
    }

    /// Construct a conflict that identifies the stale mutation in a batch.
    pub const fn batch_conflict(
        current_revision: ZoneRevision,
        mutation_ordinal: MutationOrdinal,
        retry_class: RetryClass,
        reason_code: &'static str,
    ) -> Self {
        Self {
            kind: StoreErrorKind::ResourceConflict,
            current_revision: Some(current_revision),
            mutation_ordinal: Some(mutation_ordinal),
            retry_after_ms: None,
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

    pub const fn mutation_ordinal(&self) -> Option<MutationOrdinal> {
        self.mutation_ordinal
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
            .field("mutation_ordinal", &self.mutation_ordinal)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_ordinal_is_zero_based_and_bounded_by_batch_limit() {
        assert_eq!(MutationOrdinal::new(0).unwrap().get(), 0);
        assert_eq!(
            MutationOrdinal::new(u32::try_from(MAX_BATCH_MUTATIONS - 1).unwrap())
                .unwrap()
                .get(),
            u32::try_from(MAX_BATCH_MUTATIONS - 1).unwrap()
        );
        assert_eq!(
            MutationOrdinal::new(u32::try_from(MAX_BATCH_MUTATIONS).unwrap()),
            Err(MutationOrdinalError)
        );
    }

    #[test]
    fn batch_conflict_carries_only_revision_and_bounded_ordinal() {
        let error = StoreError::batch_conflict(
            ZoneRevision::new(9),
            MutationOrdinal::new(3).unwrap(),
            RetryClass::Reauthorize,
            "revision-changed",
        );

        assert_eq!(error.kind(), StoreErrorKind::ResourceConflict);
        assert_eq!(error.current_revision(), Some(ZoneRevision::new(9)));
        assert_eq!(error.mutation_ordinal().unwrap().get(), 3);
        assert_eq!(error.retry_after_ms(), None);
        assert_eq!(error.reason_code(), "revision-changed");
    }
}
