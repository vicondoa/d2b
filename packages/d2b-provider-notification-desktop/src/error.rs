//! Stable notification Provider error surface.

/// Stable error code family used by stream and lifecycle adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderError {
    /// Session admission failed.
    Session,
    /// A field or category was rejected.
    Schema,
    /// The desktop sink is unavailable.
    SinkUnavailable,
    /// A bounded queue or action capability limit was reached.
    Capacity,
    /// The authenticated display dependency route was refused.
    DisplayDependencyUnauthenticated,
    /// The Provider reference is invalid or not the notification Provider.
    ProviderRefInvalid,
    /// A configured Guest source reference is not a Guest.
    SourceRefInvalid,
    /// The configured Guest source category set is empty.
    CategorySetEmpty,
    /// The request category is not allowlisted for the Guest source.
    CategoryDenied,
    /// The Guest source session generation is zero.
    SourceGenerationInvalid,
    /// The Guest source session is not authenticated as a source.
    SourceUnauthenticated,
    /// The Guest source session binding does not match the configured source.
    SourceBindingMismatch,
    /// The Guest source session generation is stale.
    SourceStaleGeneration,
    /// More than one authenticated session matched one configured source.
    SourceAmbiguous,
    /// The configured Guest source set exceeds its bound.
    SourceCapacity,
    /// The configured Guest source set contains a duplicate reference.
    SourceDuplicate,
    /// The pending projection bound was refused.
    PendingCapacity,
    /// The action capability TTL is outside its bound.
    ActionNonceTtl,
    /// The action capability store size is outside its bound.
    ActionNonceCapacity,
    /// The observer acknowledgement timeout is outside its bound.
    AcknowledgeTimeout,
    /// The display Provider dependency is not the canonical display Provider.
    DisplayProviderInvalid,
    /// The configured display Provider does not match the authenticated one.
    DisplayProviderMismatch,
    /// The Host or User binding is invalid.
    HostBindingInvalid,
    /// The Host or User binding is missing.
    HostBindingMissing,
    /// The authenticated display binding does not match the configuration.
    HostBindingMismatch,
    /// A configured Guest source is in a different Zone than the display.
    SourceZoneMismatch,
    /// The display dependency is not available.
    DisplayDependencyUnavailable,
    /// The supervisor receipt does not match the lifecycle plan.
    SupervisorReceiptMismatch,
    /// The process effect receipt is incomplete.
    ProcessEffectIncomplete,
    /// The process effect proof does not match the reconciliation result.
    ProcessEffectProofMismatch,
    /// A lifecycle source identity is invalid.
    LifecycleSourceInvalid,
    /// A lifecycle host-sink identity is invalid.
    LifecycleHostSinkInvalid,
    /// The lifecycle Provider reference is invalid.
    LifecycleProviderInvalid,
    /// The lifecycle plan is invalid.
    LifecyclePlanInvalid,
    /// The lifecycle adoption observation is invalid.
    LifecycleAdoptionInvalid,
    /// The adopted source does not match the plan.
    LifecycleSourceAdoptionMismatch,
    /// The adopted host sink does not match the plan.
    LifecycleHostSinkAdoptionMismatch,
    /// A different source is already active for the same reference.
    LifecycleSourceAlreadyActive,
    /// A different host sink is already active.
    LifecycleHostSinkAlreadyActive,
    /// The lifecycle state lock is unavailable.
    LifecycleStateUnavailable,
    /// The active host sink is missing.
    LifecycleHostSinkMissing,
    /// The lifecycle Zone is unavailable.
    LifecycleZoneUnavailable,
    /// Compensation failed and supervisor recovery is required.
    LifecycleRecoveryRequired,
    /// The host backend refused to start a Guest source.
    LifecycleSourceStartFailed,
    /// The host backend lifecycle state is unavailable.
    LifecycleSourceUnavailable,
    /// The host backend source set does not match the plan.
    LifecycleSourceMismatch,
    /// The host sink is unavailable.
    HostSinkUnavailable,
    /// The active host sink does not match the plan.
    HostSinkLifecycleMismatch,
    /// The notification supervisor is unavailable.
    SupervisorUnavailable,
    /// The ComponentSession authority release is incomplete.
    AuthorityReleaseIncomplete,
    /// A collector telemetry field was rejected.
    TelemetryFieldRejected,
}

impl ProviderError {
    /// Return the stable error slug.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session-denied",
            Self::Schema => "notification-schema-invalid",
            Self::SinkUnavailable => "sink-unavailable",
            Self::Capacity => "capacity-exceeded",
            Self::DisplayDependencyUnauthenticated => "display-dependency-unauthenticated",
            Self::ProviderRefInvalid => "notification-provider-ref-invalid",
            Self::SourceRefInvalid => "notification-source-ref-invalid",
            Self::CategorySetEmpty => "notification-category-set-empty",
            Self::CategoryDenied => "notification-category-denied",
            Self::SourceGenerationInvalid => "notification-source-generation-invalid",
            Self::SourceUnauthenticated => "notification-source-unauthenticated",
            Self::SourceBindingMismatch => "notification-source-binding-mismatch",
            Self::SourceStaleGeneration => "notification-source-stale-generation",
            Self::SourceAmbiguous => "notification-source-ambiguous",
            Self::SourceCapacity => "notification-source-capacity",
            Self::SourceDuplicate => "notification-source-duplicate",
            Self::PendingCapacity => "notification-pending-capacity",
            Self::ActionNonceTtl => "notification-action-nonce-ttl",
            Self::ActionNonceCapacity => "notification-action-nonce-capacity",
            Self::AcknowledgeTimeout => "notification-acknowledge-timeout",
            Self::DisplayProviderInvalid => "notification-display-provider-invalid",
            Self::DisplayProviderMismatch => "notification-display-provider-mismatch",
            Self::HostBindingInvalid => "notification-host-binding-invalid",
            Self::HostBindingMissing => "notification-host-binding-missing",
            Self::HostBindingMismatch => "notification-host-binding-mismatch",
            Self::SourceZoneMismatch => "notification-source-zone-mismatch",
            Self::DisplayDependencyUnavailable => "notification-display-dependency-unavailable",
            Self::SupervisorReceiptMismatch => "notification-supervisor-receipt-mismatch",
            Self::ProcessEffectIncomplete => "notification-process-effect-incomplete",
            Self::ProcessEffectProofMismatch => "notification-process-effect-proof-mismatch",
            Self::LifecycleSourceInvalid => "notification-lifecycle-source-invalid",
            Self::LifecycleHostSinkInvalid => "notification-lifecycle-host-sink-invalid",
            Self::LifecycleProviderInvalid => "notification-lifecycle-provider-invalid",
            Self::LifecyclePlanInvalid => "notification-lifecycle-plan-invalid",
            Self::LifecycleAdoptionInvalid => "notification-lifecycle-adoption-invalid",
            Self::LifecycleSourceAdoptionMismatch => {
                "notification-lifecycle-source-adoption-mismatch"
            }
            Self::LifecycleHostSinkAdoptionMismatch => {
                "notification-lifecycle-host-sink-adoption-mismatch"
            }
            Self::LifecycleSourceAlreadyActive => "notification-lifecycle-source-already-active",
            Self::LifecycleHostSinkAlreadyActive => "notification-lifecycle-host-sink-already-active",
            Self::LifecycleStateUnavailable => "notification-lifecycle-state-unavailable",
            Self::LifecycleHostSinkMissing => "notification-lifecycle-host-sink-missing",
            Self::LifecycleZoneUnavailable => "notification-lifecycle-zone-unavailable",
            Self::LifecycleRecoveryRequired => "notification-lifecycle-recovery-required",
            Self::LifecycleSourceStartFailed => "notification-lifecycle-source-start-failed",
            Self::LifecycleSourceUnavailable => "notification-source-lifecycle-unavailable",
            Self::LifecycleSourceMismatch => "notification-source-lifecycle-mismatch",
            Self::HostSinkUnavailable => "notification-host-sink-unavailable",
            Self::HostSinkLifecycleMismatch => "notification-host-sink-lifecycle-mismatch",
            Self::SupervisorUnavailable => "notification-supervisor-unavailable",
            Self::AuthorityReleaseIncomplete => "notification-authority-release-incomplete",
            Self::TelemetryFieldRejected => "notification-telemetry-field-rejected",
        }
    }
}

impl core::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for ProviderError {}