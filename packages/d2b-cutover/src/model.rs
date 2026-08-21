//! Shared closed values for the cutover contract.

use std::fmt;

use d2b_contracts::v3::is_canonical_digest;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as DeserializeError};

const MAX_OPAQUE_ID_BYTES: usize = 128;
const MAX_REASON_BYTES: usize = 256;

macro_rules! opaque_id {
    ($name:ident, $label:literal) => {
        #[doc = concat!("Opaque ", $label, " identity.")]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Parse one bounded identifier without path syntax.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > MAX_OPAQUE_ID_BYTES
                    || value
                        .bytes()
                        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
                    || value.contains('/')
                    || value.contains('\\')
                {
                    return Err(IdError::Invalid($label));
                }
                Ok(Self(value))
            }

            /// Borrow the opaque identifier.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.debug_tuple($label).field(&"<opaque>").finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(D::Error::custom)
            }
        }
    };
}

opaque_id!(OperationId, "OperationId");
opaque_id!(CandidateId, "CandidateId");
opaque_id!(RevisionPlanId, "RevisionPlanId");
opaque_id!(RecoveryId, "RecoveryId");
opaque_id!(OperatorId, "OperatorId");
opaque_id!(ZoneId, "ZoneId");
opaque_id!(ArtifactId, "ArtifactId");
opaque_id!(EffectId, "EffectId");
opaque_id!(StepId, "StepId");
opaque_id!(LockId, "LockId");
opaque_id!(AuditRecordId, "AuditRecordId");

/// A bounded identity parsing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdError {
    /// The value was empty, oversized, contained control syntax, or looked like a path.
    Invalid(&'static str),
    /// A digest did not use the canonical SHA-256 spelling.
    InvalidDigest,
    /// A reason exceeded its bounded text contract.
    InvalidReason,
}

impl fmt::Display for IdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(label) => write!(formatter, "invalid {label}"),
            Self::InvalidDigest => formatter.write_str("invalid canonical digest"),
            Self::InvalidReason => formatter.write_str("invalid bounded reason"),
        }
    }
}

impl std::error::Error for IdError {}

/// A canonical SHA-256 digest used at a contract boundary.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(String);

impl Digest {
    /// Parse a canonical `sha256:` digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if is_canonical_digest(&value) {
            Ok(Self(value))
        } else {
            Err(IdError::InvalidDigest)
        }
    }

    /// Derive a domain-separated digest from canonical bytes.
    pub fn derive(domain: &str, bytes: &[u8]) -> Self {
        Self(d2b_contracts::v3::canonical_digest(domain, bytes))
    }

    /// Borrow the rendered digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Digest(<redacted>)")
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// A bounded operator-supplied hold reason.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HoldReason(String);

impl HoldReason {
    /// Parse a bounded, path-free reason.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_REASON_BYTES
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(IdError::InvalidReason);
        }
        Ok(Self(value))
    }

    /// Borrow the reason.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for HoldReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("HoldReason")
            .field(&"<bounded>")
            .finish()
    }
}

impl Serialize for HoldReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for HoldReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// The closed phase sequence shared by cutover and reset operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CutoverPhase {
    /// Read-only baseline and candidate checks.
    Preflight,
    /// Read-only consent admission.
    Consent,
    /// Read-only authoritative inventory construction.
    Inventory,
    /// Quiesce the legacy control plane.
    Drain,
    /// Stage dispositions while sources remain intact.
    Disposition,
    /// Initialize or reset the resource store.
    ResourceStore,
    /// Install and start providers.
    ProviderInstall,
    /// Reconcile Zones and ZoneLinks.
    ZoneCutover,
    /// Activate Guests and runtime resources.
    Activation,
    /// Verify every Zone and preserved identity.
    Verification,
    /// Separately consented old-artifact finalization.
    Finalization,
}

impl CutoverPhase {
    /// Return the protocol phase number.
    pub const fn number(self) -> u8 {
        match self {
            Self::Preflight => 0,
            Self::Consent => 1,
            Self::Inventory => 2,
            Self::Drain => 3,
            Self::Disposition => 4,
            Self::ResourceStore => 5,
            Self::ProviderInstall => 6,
            Self::ZoneCutover => 7,
            Self::Activation => 8,
            Self::Verification => 9,
            Self::Finalization => 10,
        }
    }

    /// Return whether native rollback is still available at this phase.
    pub const fn is_before_or_at_native_rollback_boundary(self) -> bool {
        self.number() <= Self::Disposition.number()
    }

    /// Return the next phase, if one exists.
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Preflight => Some(Self::Consent),
            Self::Consent => Some(Self::Inventory),
            Self::Inventory => Some(Self::Drain),
            Self::Drain => Some(Self::Disposition),
            Self::Disposition => Some(Self::ResourceStore),
            Self::ResourceStore => Some(Self::ProviderInstall),
            Self::ProviderInstall => Some(Self::ZoneCutover),
            Self::ZoneCutover => Some(Self::Activation),
            Self::Activation => Some(Self::Verification),
            Self::Verification => Some(Self::Finalization),
            Self::Finalization => None,
        }
    }
}

/// The distinct operation authorities in this unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationKind {
    /// One host-wide cutover.
    Cutover,
    /// One scoped post-cutover reset.
    ScopedReset(ResetScope),
}

impl OperationKind {
    /// Return whether this is the host-wide cutover authority.
    pub const fn is_cutover(self) -> bool {
        matches!(self, Self::Cutover)
    }

    /// Return the reset scope, when this is a reset.
    pub const fn reset_scope(self) -> Option<ResetScope> {
        match self {
            Self::Cutover => None,
            Self::ScopedReset(scope) => Some(scope),
        }
    }
}

/// The three supported scoped reset boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResetScope {
    /// Reset a complete Zone store.
    Zone,
    /// Reset one Provider and its owned children.
    Provider,
    /// Reset one Guest and its owned children.
    Guest,
}

/// The disposition assigned to an inventory item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    /// Stage a destination while retaining the source.
    Adopt,
    /// Leave the source untouched.
    Preserve,
    /// Remove only after a separately cleared finalization gate.
    Destroy,
}

/// Replay behavior for an effect after a crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReplayClass {
    /// The effect may be issued again with the same request.
    Repeatable,
    /// A journaled identity must be reopened instead of creating again.
    ReopenByJournaledIdentity,
    /// The effect is unsafe to infer and must quarantine on uncertainty.
    QuarantineOnly,
}

/// Closed effect kinds used by the pure allowlist model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectKind {
    /// Stop the legacy daemon and broker control plane.
    HostDrain,
    /// Stage one cutover disposition.
    CutoverDisposition,
    /// Create or reopen the resource store.
    ResourceStoreCreate,
    /// Install a Provider.
    ProviderInstall,
    /// Activate the candidate Zone.
    ZoneActivation,
    /// Activate a Guest or runtime sidecar.
    GuestActivation,
    /// Verify a Zone or identity-bearing artifact.
    Verification,
    /// Remove one legacy artifact after phase-10 consent.
    CutoverFinalization,
    /// Reset one complete Zone.
    ScopedZoneReset,
    /// Reset one Provider.
    ScopedProviderReset,
    /// Reset one Guest.
    ScopedGuestReset,
    /// Destroy a reset-scoped durable Volume when explicitly allowed.
    DestroyDurableVolume,
    /// Preserve an identity-bearing source.
    PreserveSource,
    /// Quarantine an ambiguous staged destination.
    QuarantineDestination,
    /// A cutover-specific broker capability.
    CutoverBroker,
    /// Activate a frozen system closure.
    ClosureActivation,
}

/// State visible to a pure operation observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationState {
    /// No apply transition has consumed consent.
    Planned,
    /// An operator hold is active.
    Held,
    /// The operation is applying the named phase.
    Applying(CutoverPhase),
    /// Phase-9 verification succeeded and finalization is pending.
    CutoverSucceeded,
    /// Phase-10 finalization is active.
    Finalizing,
    /// Native rollback completed through phase 4.
    RolledBack,
    /// Recovery from an external snapshot is required.
    RestoreRequired,
    /// Phase 10 completed and the operation is closed.
    Closed,
    /// A write-once terminal failure was published.
    Failed,
}

impl OperationState {
    /// Return whether this state cannot accept another transition.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::RolledBack | Self::RestoreRequired | Self::Closed | Self::Failed
        )
    }
}

/// Closed terminal outcome classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalOutcomeKind {
    /// Native rollback completed.
    RolledBack,
    /// The operation crossed phase 5 and needs external restore.
    RestoreRequired,
    /// Phase 10 finalization completed.
    Closed,
    /// A fail-closed terminal error was published.
    Failed,
}

/// Stable failure reasons used by transitions and replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureCode {
    /// The host inventory did not cover every configured Zone.
    InventoryIncomplete,
    /// The inventory contained inconsistent or duplicate state.
    InventoryInconsistent,
    /// The caller attempted to enumerate gateway credentials or audit.
    GatewayCredentialAuditEnumerationForbidden,
    /// Consent did not match the operation or was already consumed.
    ConsentInvalid,
    /// Recovery evidence did not match the bound candidate.
    RecoveryMismatch,
    /// Candidate, marker, or ownership evidence drifted.
    CandidateDrift,
    /// A request or journal digest did not match.
    RequestMismatch,
    /// Journal integrity could not be proven.
    JournalTampered,
    /// Another operation owns the host-wide linearization lock.
    LockContended,
    /// A hold blocks the next destructive transition.
    HoldActive,
    /// The effect failed before durable completion.
    EffectFailed,
    /// Audit evidence was not durable.
    AuditNotDurable,
    /// An identity-bearing replay could not prove the original identity.
    IdentityMismatch,
    /// A staged destination was partial or ambiguous.
    DestinationAmbiguous,
    /// Native rollback was requested after phase 4.
    RollbackWindowClosed,
    /// The source artifact was not retained as required.
    SourceNotPreserved,
    /// The operation attempted a forbidden effect.
    EffectNotAllowed,
    /// A phase was not ready for the requested transition.
    InvalidTransition,
    /// A terminal result already exists.
    TerminalAlreadyWritten,
    /// Finalization consent was missing or mismatched.
    FinalizationConsentRequired,
    /// Verification did not cover every required Zone.
    VerificationIncomplete,
}

/// A durable effect result supplied by an external adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectOutcome {
    /// The effect completed.
    Succeeded,
    /// The effect failed before completion.
    Failed,
    /// The effect may have completed but ownership is ambiguous.
    Ambiguous,
}

/// A pure observation of the privileged audit publication boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvidence {
    durable: bool,
    record_id: Option<AuditRecordId>,
}

impl AuditEvidence {
    /// Construct durable audit evidence.
    pub fn durable(record_id: impl Into<String>) -> Result<Self, IdError> {
        Ok(Self {
            durable: true,
            record_id: Some(AuditRecordId::new(record_id)?),
        })
    }

    /// Construct a failed audit publication result.
    pub const fn unavailable() -> Self {
        Self {
            durable: false,
            record_id: None,
        }
    }

    /// Return whether the audit record is durable.
    pub const fn is_durable(&self) -> bool {
        self.durable
    }

    /// Return the opaque audit record identity.
    pub fn record_id(&self) -> Option<&AuditRecordId> {
        self.record_id.as_ref()
    }
}

/// A pure effect result, optionally carrying a journaled identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEvidence {
    outcome: EffectOutcome,
    identity: Option<ArtifactId>,
}

impl EffectEvidence {
    /// Construct a successful effect without an identity-bearing result.
    pub const fn succeeded() -> Self {
        Self {
            outcome: EffectOutcome::Succeeded,
            identity: None,
        }
    }

    /// Construct a successful effect with an observed identity.
    pub fn succeeded_with_identity(identity: impl Into<String>) -> Result<Self, IdError> {
        Ok(Self {
            outcome: EffectOutcome::Succeeded,
            identity: Some(ArtifactId::new(identity)?),
        })
    }

    /// Construct a failed effect.
    pub const fn failed() -> Self {
        Self {
            outcome: EffectOutcome::Failed,
            identity: None,
        }
    }

    /// Construct an ambiguous effect.
    pub const fn ambiguous() -> Self {
        Self {
            outcome: EffectOutcome::Ambiguous,
            identity: None,
        }
    }

    /// Return the effect outcome.
    pub const fn outcome(&self) -> &EffectOutcome {
        &self.outcome
    }

    /// Return the observed identity, if one was supplied.
    pub fn identity(&self) -> Option<&ArtifactId> {
        self.identity.as_ref()
    }
}

/// Evidence required before a mutating transition may advance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionEvidence {
    /// Effect result from the injected adapter.
    pub effect: EffectEvidence,
    /// Durable privileged audit result.
    pub audit: AuditEvidence,
}

/// Observation used to classify a crash-replay boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayObservation {
    /// No durable destination was observed.
    Absent,
    /// The expected journaled identity is present and valid.
    JournaledIdentity(ArtifactId),
    /// A different identity replaced the expected destination.
    WrongIdentity,
    /// More than one identity claims the destination.
    DuplicateIdentity,
    /// The provisioning marker is invalid or missing.
    InvalidMarker,
    /// A destination exists but is incomplete.
    PartialDestination,
    /// A previously accepted destination was replaced.
    ReplacedDestination,
    /// A foreign owner controls the destination.
    ForeignOwner,
    /// The observation cannot be classified safely.
    Ambiguous,
}

/// The pure replay instruction returned for one effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayDecision {
    /// Re-run the repeatable effect.
    Repeat,
    /// Reopen the original identity without creating another one.
    Reopen(ArtifactId),
    /// Stop and quarantine the ambiguous destination.
    Quarantine(FailureCode),
}

/// A bounded host-wide linearization lock model.
#[derive(Debug, Default)]
pub struct HostLockContract {
    owner: Option<OperationId>,
}

/// Result of attempting to acquire the host-wide lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockAcquire {
    /// The caller now owns the lock.
    Acquired,
    /// Another operation already owns the lock.
    Contended(OperationId),
    /// The caller already owns the lock.
    AlreadyOwned,
}

impl HostLockContract {
    /// Construct an unowned host-wide lock model.
    pub const fn new() -> Self {
        Self { owner: None }
    }

    /// Attempt one linearized acquisition without touching the host.
    pub fn acquire(&mut self, operation: &OperationId) -> LockAcquire {
        match &self.owner {
            None => {
                self.owner = Some(operation.clone());
                LockAcquire::Acquired
            }
            Some(owner) if owner == operation => LockAcquire::AlreadyOwned,
            Some(owner) => LockAcquire::Contended(owner.clone()),
        }
    }

    /// Release the lock only for its current owner.
    pub fn release(&mut self, operation: &OperationId) -> bool {
        if self.owner.as_ref() == Some(operation) {
            self.owner = None;
            true
        } else {
            false
        }
    }

    /// Return the opaque current owner.
    pub fn owner(&self) -> Option<&OperationId> {
        self.owner.as_ref()
    }
}
