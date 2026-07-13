//! Serialized d2b 2.0 storage, synchronization, state, and audit contracts.
//!
//! The types in this module are a clean v2 rail. They deliberately expose
//! opaque resource identifiers and canonical runtime identities rather than
//! host paths, configured names, commands, or payload bytes.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::v2_identity::{ProviderId, RealmId, RoleId, WorkloadId};

pub const STATE_SCHEMA_VERSION: u32 = 2;
pub const STATE_SCHEMA_GENERATION: u32 = 1;
pub const MAX_JSON_DOCUMENT_BYTES: u64 = 1_048_576;
pub const MAX_INVENTORY_ROWS: usize = 4_096;
pub const MAX_LOCKS: usize = 1_024;
pub const MAX_LOCK_DEPENDENCIES: usize = 32;
pub const MAX_DISCOVERY_OBSERVATIONS: usize = 4_096;
pub const MAX_PROJECTION_ENTRIES: usize = 4_096;
pub const MAX_AUDIT_RECORD_BYTES: u32 = 8_192;
pub const MAX_AUDIT_RECORDS_PER_SEGMENT: usize = 16_384;
pub const MAX_AUDIT_SEGMENT_BYTES: u64 = 64 * 1_024 * 1_024;
pub const MAX_AUDIT_RETENTION_DAYS: u16 = 14;
pub const MAX_LOCK_DEADLINE_MS: u32 = 300_000;
pub const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_OPAQUE_ID_BYTES: usize = 64;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum StateContractError {
    UnsupportedSchemaVersion,
    UnsupportedSchemaGeneration,
    ContractFingerprintMismatch,
    BoundExceeded,
    EmptyInventory,
    DuplicateResourceId,
    DuplicateLockId,
    MissingInventoryCategory,
    ScopeCategoryMismatch,
    LocationCategoryMismatch,
    RepairAuthorityMismatch,
    InvalidOfdPolicy,
    DuplicateLockOrder,
    UnknownLockDependency,
    LockOrderViolation,
    LockOrderCycle,
    LeaseGenerationMismatch,
    LeaseExpired,
    RestartEvidenceIncomplete,
    RestartAmbiguous,
    CleanupBeforeRecovery,
    CleanupWithoutOwnerAbsenceProof,
    InvalidAtomicTransition,
    SuccessBeforeParentFsync,
    EnvelopeChecksumMissing,
    AuditOwnerMismatch,
    AuditStreamMismatch,
    AuditSequenceMismatch,
    AuditChainMismatch,
    AuditCheckpointMismatch,
    AuditGap,
    AuditExportRangeInvalid,
    RetentionOutOfBounds,
    RetentionCheckpointRequired,
    ProjectionIsNotDiagnostic,
}

impl fmt::Display for StateContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedSchemaVersion => "unsupported state schema version",
            Self::UnsupportedSchemaGeneration => "unsupported state schema generation",
            Self::ContractFingerprintMismatch => "state contract fingerprint mismatch",
            Self::BoundExceeded => "state contract bound exceeded",
            Self::EmptyInventory => "state inventory is empty",
            Self::DuplicateResourceId => "duplicate storage resource id",
            Self::DuplicateLockId => "duplicate synchronization lock id",
            Self::MissingInventoryCategory => "storage inventory category is missing",
            Self::ScopeCategoryMismatch => "storage category and identity scope differ",
            Self::LocationCategoryMismatch => "storage category and logical location differ",
            Self::RepairAuthorityMismatch => "storage owner and repair authority differ",
            Self::InvalidOfdPolicy => "OFD lock policy is not fail-closed",
            Self::DuplicateLockOrder => "duplicate global lock order",
            Self::UnknownLockDependency => "lock dependency is not declared",
            Self::LockOrderViolation => "lock dependency violates global order",
            Self::LockOrderCycle => "lock dependency cycle",
            Self::LeaseGenerationMismatch => "lease generation mismatch",
            Self::LeaseExpired => "lease is expired or revoked",
            Self::RestartEvidenceIncomplete => "restart evidence is incomplete",
            Self::RestartAmbiguous => "restart observation is ambiguous",
            Self::CleanupBeforeRecovery => "cleanup was requested before recovery",
            Self::CleanupWithoutOwnerAbsenceProof => "cleanup lacks exact owner-absence proof",
            Self::InvalidAtomicTransition => "invalid atomic write phase transition",
            Self::SuccessBeforeParentFsync => {
                "authoritative write cannot succeed before parent fsync"
            }
            Self::EnvelopeChecksumMissing => "authoritative envelope checksum is missing",
            Self::AuditOwnerMismatch => "audit stream and owner differ",
            Self::AuditStreamMismatch => "audit stream mismatch",
            Self::AuditSequenceMismatch => "audit sequence mismatch",
            Self::AuditChainMismatch => "audit hash chain mismatch",
            Self::AuditCheckpointMismatch => "audit checkpoint mismatch",
            Self::AuditGap => "audit sequence gap",
            Self::AuditExportRangeInvalid => "audit export range is invalid",
            Self::RetentionOutOfBounds => "audit retention is outside contract bounds",
            Self::RetentionCheckpointRequired => "audit checkpoint is required before retention",
            Self::ProjectionIsNotDiagnostic => "state projection claims authority",
        };
        f.write_str(message)
    }
}

impl Error for StateContractError {}

fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OPAQUE_ID_BYTES
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

macro_rules! opaque_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(#[schemars(regex(pattern = "^[a-z][a-z0-9-]{0,63}$"))] String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, StateContractError> {
                let value = value.into();
                if valid_opaque_id(&value) {
                    Ok(Self(value))
                } else {
                    Err(StateContractError::BoundExceeded)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.0).finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

opaque_id!(
    ResourceId,
    "An opaque generated storage or synchronization resource identifier."
);
opaque_id!(
    CorrelationId,
    "A bounded opaque operation or session correlation identifier."
);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Digest(#[schemars(regex(pattern = "^[0-9a-f]{64}$"))] String);

impl Digest {
    pub fn parse(value: impl Into<String>) -> Result<Self, StateContractError> {
        let value = value.into();
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(value))
        } else {
            Err(StateContractError::BoundExceeded)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Digest").field(&self.0).finish()
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Generation(#[schemars(range(min = 1, max = 9007199254740991_u64))] u64);

impl Generation {
    pub fn new(value: u64) -> Result<Self, StateContractError> {
        if (1..=MAX_SAFE_JSON_INTEGER).contains(&value) {
            Ok(Self(value))
        } else {
            Err(StateContractError::BoundExceeded)
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Generation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IdentityScope {
    LocalRoot,
    Realm {
        realm_id: RealmId,
    },
    Workload {
        realm_id: RealmId,
        workload_id: WorkloadId,
    },
    Provider {
        realm_id: RealmId,
        provider_id: ProviderId,
    },
    Role {
        realm_id: RealmId,
        workload_id: WorkloadId,
        role_id: RoleId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AuthorityRef {
    Pid1,
    LocalRootAllocator,
    LocalRootBroker,
    RealmController {
        realm_id: RealmId,
    },
    RealmBroker {
        realm_id: RealmId,
    },
    WorkloadRole {
        realm_id: RealmId,
        workload_id: WorkloadId,
        role_id: RoleId,
    },
    Provider {
        realm_id: RealmId,
        provider_id: ProviderId,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum StorageCategory {
    LocalRoot,
    Realm,
    Workload,
    Provider,
    Runtime,
    Lock,
    Lease,
    Quarantine,
    Audit,
    Projection,
}

impl StorageCategory {
    pub const ALL: [Self; 10] = [
        Self::LocalRoot,
        Self::Realm,
        Self::Workload,
        Self::Provider,
        Self::Runtime,
        Self::Lock,
        Self::Lease,
        Self::Quarantine,
        Self::Audit,
        Self::Projection,
    ];
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum LogicalLocation {
    HostAllocator,
    HostBroker,
    HostAudit,
    RealmController,
    RealmBroker,
    RealmAudit,
    WorkloadState,
    WorkloadDisks,
    WorkloadStoreView,
    WorkloadTpm,
    WorkloadMedia,
    WorkloadAudio,
    WorkloadKeys,
    ProviderState,
    RuntimeRealm,
    RuntimeWorkload,
    RuntimeRole,
    RuntimeLocks,
    RuntimeLeases,
    Quarantine,
    Projection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceKind {
    Directory,
    JsonDocument,
    JsonLinesSegment,
    UnixSocket,
    OfdLockFile,
    LeaseRecord,
    DiskImage,
    TpmState,
    StoreView,
    ProjectionDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CreationPolicy {
    CreateIfMissing,
    MaterializeGeneration,
    BindPrecreated,
    ObserveOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReconcilePolicy {
    VerifyOnly,
    ReconcileExact,
    AdoptAfterProof,
    Regenerate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RepairPolicy {
    FailClosed,
    RepairExact,
    Quarantine,
    Regenerate,
    NoRepair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeletePolicy {
    Never,
    AfterOwnerAbsenceProof,
    GenerationRetirement,
    RetentionOnly,
    FactoryResetOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PersistenceClass {
    Configuration,
    Persistent,
    BootScoped,
    ProcessScoped,
    Regenerable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SecretClass {
    PublicMetadata,
    PrivateMetadata,
    SecretAdjacent,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum FileMode {
    #[serde(rename = "0600")]
    Mode0600,
    #[serde(rename = "0640")]
    Mode0640,
    #[serde(rename = "0660")]
    Mode0660,
    #[serde(rename = "0700")]
    Mode0700,
    #[serde(rename = "0710")]
    Mode0710,
    #[serde(rename = "0750")]
    Mode0750,
    #[serde(rename = "0770")]
    Mode0770,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GroupPolicy {
    OwnerPrimaryGroup,
    LocalRootControl,
    RealmControl,
    RolePrivate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnershipPolicy {
    pub owner: AuthorityRef,
    pub group: GroupPolicy,
    pub mode: FileMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageRestartPolicy {
    PreserveAndVerify,
    RecreateAfterOwnerAbsence,
    RegenerateFromConfiguration,
    QuarantineOnAmbiguity,
    NotAdoptable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageAdoptionPolicy {
    VerifyIdentityAndGeneration,
    VerifyLease,
    ReopenFromPersistentState,
    QuarantineOnAmbiguity,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageResource {
    pub resource_id: ResourceId,
    pub category: StorageCategory,
    pub kind: ResourceKind,
    pub scope: IdentityScope,
    pub logical_location: LogicalLocation,
    pub creation: CreationPolicy,
    pub reconcile: ReconcilePolicy,
    pub repair: RepairPolicy,
    pub delete: DeletePolicy,
    pub creation_authority: AuthorityRef,
    pub reconcile_authority: AuthorityRef,
    pub repair_authority: AuthorityRef,
    pub delete_authority: AuthorityRef,
    pub ownership: OwnershipPolicy,
    pub persistence: PersistenceClass,
    pub secret_class: SecretClass,
    pub restart: StorageRestartPolicy,
    pub adoption: StorageAdoptionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageInventory {
    pub schema_generation: u32,
    pub contract_fingerprint: Digest,
    #[schemars(length(min = 1, max = 4096))]
    pub resources: Vec<StorageResource>,
}

impl StorageInventory {
    pub fn validate(&self) -> Result<(), StateContractError> {
        if self.schema_generation != STATE_SCHEMA_GENERATION {
            return Err(StateContractError::UnsupportedSchemaGeneration);
        }
        if self.resources.is_empty() {
            return Err(StateContractError::EmptyInventory);
        }
        if self.resources.len() > MAX_INVENTORY_ROWS {
            return Err(StateContractError::BoundExceeded);
        }

        let mut ids = BTreeSet::new();
        let mut categories = BTreeSet::new();
        for resource in &self.resources {
            if !ids.insert(resource.resource_id.as_str()) {
                return Err(StateContractError::DuplicateResourceId);
            }
            categories.insert(resource.category);
            if resource.ownership.owner != resource.repair_authority {
                return Err(StateContractError::RepairAuthorityMismatch);
            }
            validate_category_scope(resource)?;
            validate_category_location(resource)?;
        }
        if StorageCategory::ALL
            .into_iter()
            .any(|category| !categories.contains(&category))
        {
            return Err(StateContractError::MissingInventoryCategory);
        }
        Ok(())
    }
}

fn validate_category_scope(resource: &StorageResource) -> Result<(), StateContractError> {
    let matches = match resource.category {
        StorageCategory::LocalRoot => matches!(resource.scope, IdentityScope::LocalRoot),
        StorageCategory::Realm | StorageCategory::Audit | StorageCategory::Quarantine => {
            matches!(
                resource.scope,
                IdentityScope::LocalRoot | IdentityScope::Realm { .. }
            )
        }
        StorageCategory::Workload => matches!(resource.scope, IdentityScope::Workload { .. }),
        StorageCategory::Provider => matches!(resource.scope, IdentityScope::Provider { .. }),
        StorageCategory::Runtime
        | StorageCategory::Lock
        | StorageCategory::Lease
        | StorageCategory::Projection => true,
    };
    if matches {
        Ok(())
    } else {
        Err(StateContractError::ScopeCategoryMismatch)
    }
}

fn validate_category_location(resource: &StorageResource) -> Result<(), StateContractError> {
    let matches = match resource.category {
        StorageCategory::LocalRoot => matches!(
            resource.logical_location,
            LogicalLocation::HostAllocator | LogicalLocation::HostBroker
        ),
        StorageCategory::Realm => matches!(
            resource.logical_location,
            LogicalLocation::RealmController | LogicalLocation::RealmBroker
        ),
        StorageCategory::Workload => matches!(
            resource.logical_location,
            LogicalLocation::WorkloadState
                | LogicalLocation::WorkloadDisks
                | LogicalLocation::WorkloadStoreView
                | LogicalLocation::WorkloadTpm
                | LogicalLocation::WorkloadMedia
                | LogicalLocation::WorkloadAudio
                | LogicalLocation::WorkloadKeys
        ),
        StorageCategory::Provider => resource.logical_location == LogicalLocation::ProviderState,
        StorageCategory::Runtime => matches!(
            resource.logical_location,
            LogicalLocation::RuntimeRealm
                | LogicalLocation::RuntimeWorkload
                | LogicalLocation::RuntimeRole
        ),
        StorageCategory::Lock => resource.logical_location == LogicalLocation::RuntimeLocks,
        StorageCategory::Lease => resource.logical_location == LogicalLocation::RuntimeLeases,
        StorageCategory::Quarantine => resource.logical_location == LogicalLocation::Quarantine,
        StorageCategory::Audit => matches!(
            resource.logical_location,
            LogicalLocation::HostAudit | LogicalLocation::RealmAudit
        ),
        StorageCategory::Projection => resource.logical_location == LogicalLocation::Projection,
    };
    if matches {
        Ok(())
    } else {
        Err(StateContractError::LocationCategoryMismatch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateEnvelope<T> {
    pub schema_version: u32,
    pub schema_generation: u32,
    pub config_generation: Generation,
    pub state_generation: Generation,
    pub writer: AuthorityRef,
    pub encoded_bytes: u64,
    pub checksum: Digest,
    pub payload: T,
}

impl<T> StateEnvelope<T> {
    pub fn validate_header(&self) -> Result<(), StateContractError> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(StateContractError::UnsupportedSchemaVersion);
        }
        if self.schema_generation != STATE_SCHEMA_GENERATION {
            return Err(StateContractError::UnsupportedSchemaGeneration);
        }
        if self.encoded_bytes == 0 || self.encoded_bytes > MAX_JSON_DOCUMENT_BYTES {
            return Err(StateContractError::BoundExceeded);
        }
        Ok(())
    }

    pub fn next_generation(&self) -> Result<Generation, StateContractError> {
        Generation::new(
            self.state_generation
                .get()
                .checked_add(1)
                .ok_or(StateContractError::BoundExceeded)?,
        )
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AtomicWritePhase {
    Initial,
    TemporaryCreated,
    CompleteDocumentWritten,
    TemporaryFileSynced,
    Renamed,
    ParentDirectorySynced,
}

impl AtomicWritePhase {
    pub const ALL: [Self; 6] = [
        Self::Initial,
        Self::TemporaryCreated,
        Self::CompleteDocumentWritten,
        Self::TemporaryFileSynced,
        Self::Renamed,
        Self::ParentDirectorySynced,
    ];

    pub fn transition(self, next: Self) -> Result<Self, StateContractError> {
        let expected = match self {
            Self::Initial => Some(Self::TemporaryCreated),
            Self::TemporaryCreated => Some(Self::CompleteDocumentWritten),
            Self::CompleteDocumentWritten => Some(Self::TemporaryFileSynced),
            Self::TemporaryFileSynced => Some(Self::Renamed),
            Self::Renamed => Some(Self::ParentDirectorySynced),
            Self::ParentDirectorySynced => None,
        };
        if expected == Some(next) {
            Ok(next)
        } else {
            Err(StateContractError::InvalidAtomicTransition)
        }
    }

    pub const fn can_report_success(self) -> bool {
        matches!(self, Self::ParentDirectorySynced)
    }

    pub const fn crash_outcomes(self) -> &'static [CrashRecoveryOutcome] {
        match self {
            Self::Initial
            | Self::TemporaryCreated
            | Self::CompleteDocumentWritten
            | Self::TemporaryFileSynced => &[CrashRecoveryOutcome::PriorDocument],
            Self::Renamed => &[
                CrashRecoveryOutcome::PriorDocument,
                CrashRecoveryOutcome::CompleteNewDocument,
            ],
            Self::ParentDirectorySynced => &[CrashRecoveryOutcome::CompleteNewDocument],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CrashRecoveryOutcome {
    PriorDocument,
    CompleteNewDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AtomicWriteReceipt {
    pub resource_id: ResourceId,
    pub generation: Generation,
    pub phase: AtomicWritePhase,
    pub checksum: Digest,
    pub success: bool,
}

impl AtomicWriteReceipt {
    pub fn validate(&self) -> Result<(), StateContractError> {
        if self.success && !self.phase.can_report_success() {
            return Err(StateContractError::SuccessBeforeParentFsync);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceVerdict {
    Match,
    Mismatch,
    Missing,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PidfdPersistence {
    ProcessLocalNonPersistent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerEvidence {
    pub role_id: RoleId,
    pub candidate_count: u16,
    pub pidfd_persistence: PidfdPersistence,
    pub identity: EvidenceVerdict,
    pub cgroup_membership: EvidenceVerdict,
    pub executable_fingerprint: Digest,
    pub executable: EvidenceVerdict,
    pub configuration_fingerprint: Digest,
    pub configuration: EvidenceVerdict,
    pub config_generation: Generation,
    pub generation: EvidenceVerdict,
}

impl RunnerEvidence {
    pub fn is_exact(&self) -> bool {
        self.candidate_count == 1
            && [
                self.identity,
                self.cgroup_membership,
                self.executable,
                self.configuration,
                self.generation,
            ]
            .into_iter()
            .all(|verdict| verdict == EvidenceVerdict::Match)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerObservation {
    pub observation_id: ResourceId,
    pub scope: IdentityScope,
    pub observed_pid: u32,
    pub evidence: RunnerEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceEvidence {
    pub resource_id: ResourceId,
    pub owner_identity: EvidenceVerdict,
    pub ownership: EvidenceVerdict,
    pub configuration: EvidenceVerdict,
    pub generation: EvidenceVerdict,
    pub lease: EvidenceVerdict,
}

impl ResourceEvidence {
    pub fn is_exact(&self) -> bool {
        [
            self.owner_identity,
            self.ownership,
            self.configuration,
            self.generation,
            self.lease,
        ]
        .into_iter()
        .all(|verdict| verdict == EvidenceVerdict::Match)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestartDiscovery {
    pub config_generation: Generation,
    #[schemars(length(max = 4096))]
    pub runners: Vec<RunnerObservation>,
    #[schemars(length(max = 4096))]
    pub resources: Vec<ResourceEvidence>,
}

impl RestartDiscovery {
    pub fn validate_bounds(&self) -> Result<(), StateContractError> {
        if self.runners.len() > MAX_DISCOVERY_OBSERVATIONS
            || self.resources.len() > MAX_DISCOVERY_OBSERVATIONS
        {
            return Err(StateContractError::BoundExceeded);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryOrdering {
    RecoverBeforeCleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QuarantineReason {
    MissingIdentityEvidence,
    MultipleCandidates,
    CgroupMismatch,
    ExecutableMismatch,
    ConfigurationMismatch,
    GenerationMismatch,
    OwnerAmbiguous,
    CorruptState,
    AuditGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DegradedReason {
    AdoptionPending,
    AdoptionQuarantined,
    StorageDrift,
    StorageRepairFailed,
    LockContended,
    LockOwnerAmbiguous,
    LeaseExpired,
    AuditChainInvalid,
    AuditGapDetected,
    RestartRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Remediation {
    RetryAdoption,
    InspectQuarantine,
    RestartWorkload,
    RepairStorage,
    ReleaseLease,
    ExportAudit,
    AcknowledgeAuditGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OwnerAbsenceProof {
    EmptyDeclaredCgroup,
    ExitedFreshPidfd,
    RevokedMatchingLease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CleanupTarget {
    Resource {
        resource_id: ResourceId,
    },
    Role {
        realm_id: RealmId,
        workload_id: WorkloadId,
        role_id: RoleId,
    },
    Workload {
        realm_id: RealmId,
        workload_id: WorkloadId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AdoptionDecision {
    Adopt {
        fresh_pidfd_opened: bool,
    },
    Quarantine {
        reason: QuarantineReason,
        remediation: Remediation,
    },
    Cleanup {
        target: CleanupTarget,
        owner_absence_proof: OwnerAbsenceProof,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestartDecision {
    pub observation_id: ResourceId,
    pub ordering: RecoveryOrdering,
    pub recovery_completed: bool,
    pub decision: AdoptionDecision,
}

impl RestartDecision {
    pub fn validate_for_runner(
        &self,
        observation: &RunnerObservation,
    ) -> Result<(), StateContractError> {
        if self.observation_id != observation.observation_id {
            return Err(StateContractError::RestartEvidenceIncomplete);
        }
        match &self.decision {
            AdoptionDecision::Adopt { fresh_pidfd_opened } => {
                if !observation.evidence.is_exact() {
                    return Err(StateContractError::RestartAmbiguous);
                }
                if !fresh_pidfd_opened {
                    return Err(StateContractError::RestartEvidenceIncomplete);
                }
            }
            AdoptionDecision::Cleanup { .. } if !self.recovery_completed => {
                return Err(StateContractError::CleanupBeforeRecovery);
            }
            AdoptionDecision::Quarantine { .. } | AdoptionDecision::Cleanup { .. } => {}
        }
        Ok(())
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum LockClass {
    LocalRoot,
    Realm,
    Workload,
    Provider,
    Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LockKind {
    Ofd,
    InProcess,
    KernelObject,
    FdBackedLease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FdTransferPolicy {
    Never,
    ComponentSessionAttachment,
    ScmRightsLeaseHandoff,
    ExplicitFdMapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ContentionPolicy {
    FailFast,
    BoundedWait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CancellationPolicy {
    Cancellable,
    CompleteAtomicSection,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockKey {
    pub class: LockClass,
    pub scope: IdentityScope,
    pub resource_id: ResourceId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockSpec {
    pub lock_id: ResourceId,
    pub key: LockKey,
    pub kind: LockKind,
    pub owner: AuthorityRef,
    pub release_authority: AuthorityRef,
    pub global_order: u32,
    #[schemars(length(max = 32))]
    pub acquire_after: Vec<ResourceId>,
    pub cloexec: bool,
    pub fd_transfer: FdTransferPolicy,
    pub contention: ContentionPolicy,
    pub deadline_ms: u32,
    pub cancellation: CancellationPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncInventory {
    pub schema_generation: u32,
    pub contract_fingerprint: Digest,
    #[schemars(length(max = 1024))]
    pub locks: Vec<LockSpec>,
}

impl SyncInventory {
    pub fn validate(&self) -> Result<(), StateContractError> {
        if self.schema_generation != STATE_SCHEMA_GENERATION {
            return Err(StateContractError::UnsupportedSchemaGeneration);
        }
        if self.locks.len() > MAX_LOCKS {
            return Err(StateContractError::BoundExceeded);
        }

        let mut by_id = BTreeMap::new();
        let mut orders = BTreeSet::new();
        for lock in &self.locks {
            if lock.acquire_after.len() > MAX_LOCK_DEPENDENCIES {
                return Err(StateContractError::BoundExceeded);
            }
            if by_id.insert(lock.lock_id.as_str(), lock).is_some() {
                return Err(StateContractError::DuplicateLockId);
            }
            if !orders.insert(lock.global_order) {
                return Err(StateContractError::DuplicateLockOrder);
            }
            if lock.deadline_ms == 0 || lock.deadline_ms > MAX_LOCK_DEADLINE_MS {
                return Err(StateContractError::BoundExceeded);
            }
            if lock.owner != lock.release_authority {
                return Err(StateContractError::RepairAuthorityMismatch);
            }
            let scope_matches = matches!(
                (&lock.key.class, &lock.key.scope),
                (LockClass::LocalRoot, IdentityScope::LocalRoot)
                    | (LockClass::Realm, IdentityScope::Realm { .. })
                    | (LockClass::Workload, IdentityScope::Workload { .. })
                    | (LockClass::Provider, IdentityScope::Provider { .. })
                    | (LockClass::Role, IdentityScope::Role { .. })
            );
            if !scope_matches {
                return Err(StateContractError::ScopeCategoryMismatch);
            }
            if lock.kind == LockKind::Ofd
                && (!lock.cloexec
                    || !matches!(
                        lock.fd_transfer,
                        FdTransferPolicy::Never | FdTransferPolicy::ComponentSessionAttachment
                    ))
            {
                return Err(StateContractError::InvalidOfdPolicy);
            }
        }

        let mut indegree: BTreeMap<&str, usize> = by_id.keys().copied().map(|id| (id, 0)).collect();
        let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for lock in &self.locks {
            for dependency in &lock.acquire_after {
                if !by_id.contains_key(dependency.as_str()) {
                    return Err(StateContractError::UnknownLockDependency);
                }
                *indegree
                    .get_mut(lock.lock_id.as_str())
                    .expect("declared lock has indegree row") += 1;
                outgoing
                    .entry(dependency.as_str())
                    .or_default()
                    .push(lock.lock_id.as_str());
            }
        }

        let mut ready: VecDeque<&str> = indegree
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
            .collect();
        let mut visited = 0;
        while let Some(id) = ready.pop_front() {
            visited += 1;
            for dependent in outgoing.get(id).into_iter().flatten() {
                let degree = indegree
                    .get_mut(dependent)
                    .expect("declared dependency target has indegree row");
                *degree -= 1;
                if *degree == 0 {
                    ready.push_back(dependent);
                }
            }
        }
        if visited != self.locks.len() {
            return Err(StateContractError::LockOrderCycle);
        }
        for lock in &self.locks {
            for dependency in &lock.acquire_after {
                if by_id
                    .get(dependency.as_str())
                    .expect("dependencies were checked above")
                    .global_order
                    >= lock.global_order
                {
                    return Err(StateContractError::LockOrderViolation);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LeaseRevocation {
    Active,
    RevokedByOwner,
    RevokedByGenerationChange,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseRecord {
    pub lease_id: ResourceId,
    pub resource_id: ResourceId,
    pub owner: AuthorityRef,
    pub generation: Generation,
    pub expires_at_unix_ms: u64,
    pub revocation: LeaseRevocation,
    pub fd_transfer: FdTransferPolicy,
}

impl LeaseRecord {
    pub fn validate_use(
        &self,
        expected_generation: Generation,
        now_unix_ms: u64,
    ) -> Result<(), StateContractError> {
        if self.generation != expected_generation {
            return Err(StateContractError::LeaseGenerationMismatch);
        }
        if self.expires_at_unix_ms > MAX_SAFE_JSON_INTEGER {
            return Err(StateContractError::BoundExceeded);
        }
        if self.revocation != LeaseRevocation::Active || now_unix_ms >= self.expires_at_unix_ms {
            return Err(StateContractError::LeaseExpired);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AuditStream {
    LocalRoot,
    Realm { realm_id: RealmId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AuditOwner {
    LocalRootBroker,
    RealmBroker { realm_id: RealmId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditEvent {
    StorageCreate,
    StorageReconcile,
    StorageRepair,
    StorageDelete,
    RestartDiscover,
    RestartAdopt,
    RestartQuarantine,
    LockAcquire,
    LockRelease,
    LeaseGrant,
    LeaseRevoke,
    ProviderOperation,
    SessionOperation,
    SegmentSeal,
    Checkpoint,
    GapDetected,
    RetentionPrune,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditOutcome {
    Succeeded,
    Denied,
    Failed,
    Degraded,
    Quarantined,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditReason {
    PolicyAllowed,
    PolicyDenied,
    IdentityVerified,
    IdentityMismatch,
    GenerationMismatch,
    OwnerAmbiguous,
    StorageDrift,
    LockContended,
    LeaseExpired,
    CorruptState,
    SequenceGap,
    RetentionLimit,
    OperatorRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditCorrelation {
    pub operation_id: CorrelationId,
    pub session_id: Option<CorrelationId>,
    pub provider_id: Option<ProviderId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AuditActor {
    LocalRootAllocator,
    LocalRootBroker,
    RealmController { realm_id: RealmId },
    RealmBroker { realm_id: RealmId },
    Provider { provider_id: ProviderId },
    WorkloadRole { role_id: RoleId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditRecord {
    pub schema_version: u32,
    pub stream: AuditStream,
    pub sequence: u64,
    pub occurred_at_unix_ms: u64,
    pub correlation: AuditCorrelation,
    pub actor: AuditActor,
    pub event: AuditEvent,
    pub outcome: AuditOutcome,
    pub reason: AuditReason,
    pub previous_hash: Digest,
    pub record_hash: Digest,
    pub encoded_bytes: u32,
}

impl AuditRecord {
    pub fn validate_bounds(&self) -> Result<(), StateContractError> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(StateContractError::UnsupportedSchemaVersion);
        }
        if self.sequence == 0
            || self.sequence > MAX_SAFE_JSON_INTEGER
            || self.occurred_at_unix_ms > MAX_SAFE_JSON_INTEGER
        {
            return Err(StateContractError::BoundExceeded);
        }
        if self.encoded_bytes == 0 || self.encoded_bytes > MAX_AUDIT_RECORD_BYTES {
            return Err(StateContractError::BoundExceeded);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PruneStatus {
    Retained,
    EligibleAfterCheckpoint,
    PrunedWithCheckpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditSegmentSummary {
    pub stream: AuditStream,
    pub owner: AuditOwner,
    pub segment_id: ResourceId,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub previous_segment_digest: Digest,
    pub segment_digest: Digest,
    pub controller_generation: Generation,
    pub created_at_unix_ms: u64,
    pub sealed_at_unix_ms: u64,
    pub encoded_bytes: u64,
    pub prune_status: PruneStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditSegment {
    pub summary: AuditSegmentSummary,
    #[schemars(length(min = 1, max = 16384))]
    pub records: Vec<AuditRecord>,
}

impl AuditSegment {
    pub fn validate(&self) -> Result<(), StateContractError> {
        validate_audit_owner(&self.summary.stream, &self.summary.owner)?;
        if self.records.is_empty()
            || self.records.len() > MAX_AUDIT_RECORDS_PER_SEGMENT
            || self.summary.encoded_bytes == 0
            || self.summary.encoded_bytes > MAX_AUDIT_SEGMENT_BYTES
        {
            return Err(StateContractError::BoundExceeded);
        }
        if self.summary.first_sequence == 0
            || self.summary.first_sequence > self.summary.last_sequence
            || self.summary.last_sequence > MAX_SAFE_JSON_INTEGER
            || self.summary.created_at_unix_ms > self.summary.sealed_at_unix_ms
            || self.summary.sealed_at_unix_ms > MAX_SAFE_JSON_INTEGER
        {
            return Err(StateContractError::AuditSequenceMismatch);
        }

        let mut expected_sequence = self.summary.first_sequence;
        let mut previous_hash = &self.summary.previous_segment_digest;
        for record in &self.records {
            record.validate_bounds()?;
            if record.stream != self.summary.stream {
                return Err(StateContractError::AuditStreamMismatch);
            }
            if record.sequence != expected_sequence {
                return Err(StateContractError::AuditSequenceMismatch);
            }
            if &record.previous_hash != previous_hash {
                return Err(StateContractError::AuditChainMismatch);
            }
            expected_sequence = expected_sequence
                .checked_add(1)
                .ok_or(StateContractError::BoundExceeded)?;
            previous_hash = &record.record_hash;
        }
        if self.records.last().map(|record| record.sequence) != Some(self.summary.last_sequence)
            || previous_hash != &self.summary.segment_digest
        {
            return Err(StateContractError::AuditChainMismatch);
        }
        Ok(())
    }
}

fn validate_audit_owner(
    stream: &AuditStream,
    owner: &AuditOwner,
) -> Result<(), StateContractError> {
    let matches = match (stream, owner) {
        (AuditStream::LocalRoot, AuditOwner::LocalRootBroker) => true,
        (AuditStream::Realm { realm_id: stream }, AuditOwner::RealmBroker { realm_id: owner }) => {
            stream == owner
        }
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(StateContractError::AuditOwnerMismatch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditCheckpoint {
    pub stream: AuditStream,
    pub owner: AuditOwner,
    pub checkpoint_id: ResourceId,
    pub through_sequence: u64,
    pub segment_digest: Digest,
    pub previous_checkpoint_digest: Digest,
    pub checkpoint_digest: Digest,
    pub controller_generation: Generation,
    pub created_at_unix_ms: u64,
    pub realm_signature_digest: Option<Digest>,
}

impl AuditCheckpoint {
    pub fn validate_for_segment(
        &self,
        segment: &AuditSegmentSummary,
    ) -> Result<(), StateContractError> {
        validate_audit_owner(&self.stream, &self.owner)?;
        if self.through_sequence == 0
            || self.through_sequence > MAX_SAFE_JSON_INTEGER
            || self.created_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self.stream != segment.stream
            || self.through_sequence != segment.last_sequence
            || self.segment_digest != segment.segment_digest
            || self.controller_generation != segment.controller_generation
        {
            return Err(StateContractError::AuditCheckpointMismatch);
        }
        match self.stream {
            AuditStream::LocalRoot if self.realm_signature_digest.is_some() => {
                Err(StateContractError::AuditCheckpointMismatch)
            }
            AuditStream::Realm { .. } if self.realm_signature_digest.is_none() => {
                Err(StateContractError::AuditCheckpointMismatch)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditGap {
    pub stream: AuditStream,
    pub expected_sequence: u64,
    pub observed_sequence: u64,
    pub detected_at_unix_ms: u64,
    pub reason: AuditReason,
}

pub fn detect_audit_gap(
    stream: AuditStream,
    expected_sequence: u64,
    observed_sequence: u64,
    detected_at_unix_ms: u64,
) -> Result<Option<AuditGap>, StateContractError> {
    if expected_sequence == 0
        || observed_sequence == 0
        || detected_at_unix_ms > MAX_SAFE_JSON_INTEGER
    {
        return Err(StateContractError::BoundExceeded);
    }
    if observed_sequence < expected_sequence {
        return Err(StateContractError::AuditSequenceMismatch);
    }
    Ok((observed_sequence > expected_sequence).then_some(AuditGap {
        stream,
        expected_sequence,
        observed_sequence,
        detected_at_unix_ms,
        reason: AuditReason::SequenceGap,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditRetentionPolicy {
    pub max_age_days: u16,
    pub max_segment_bytes: u64,
    pub max_records_per_segment: u32,
    pub checkpoint_required_before_prune: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditRetentionDecision {
    Retain,
    SealCurrentSegment,
    CreateCheckpoint,
    PruneCheckpointedSegment,
}

impl AuditRetentionPolicy {
    pub fn validate(&self) -> Result<(), StateContractError> {
        if !(1..=MAX_AUDIT_RETENTION_DAYS).contains(&self.max_age_days)
            || self.max_segment_bytes == 0
            || self.max_segment_bytes > MAX_AUDIT_SEGMENT_BYTES
            || self.max_records_per_segment == 0
            || self.max_records_per_segment as usize > MAX_AUDIT_RECORDS_PER_SEGMENT
            || !self.checkpoint_required_before_prune
        {
            return Err(StateContractError::RetentionOutOfBounds);
        }
        Ok(())
    }

    pub fn decide(
        &self,
        age_days: u16,
        segment_bytes: u64,
        record_count: u32,
        sealed: bool,
        checkpoint_present: bool,
    ) -> Result<AuditRetentionDecision, StateContractError> {
        self.validate()?;
        let over_limit = age_days >= self.max_age_days
            || segment_bytes >= self.max_segment_bytes
            || record_count >= self.max_records_per_segment;
        if !over_limit {
            return Ok(AuditRetentionDecision::Retain);
        }
        if !sealed {
            return Ok(AuditRetentionDecision::SealCurrentSegment);
        }
        if !checkpoint_present {
            return Ok(AuditRetentionDecision::CreateCheckpoint);
        }
        Ok(AuditRetentionDecision::PruneCheckpointedSegment)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditExportFormat {
    RedactedJsonLines,
    CheckpointBundle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditExportRequest {
    pub stream: AuditStream,
    pub operation_id: CorrelationId,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub format: AuditExportFormat,
    pub include_checkpoints: bool,
}

impl AuditExportRequest {
    pub fn validate(&self) -> Result<(), StateContractError> {
        if self.first_sequence == 0
            || self.first_sequence > self.last_sequence
            || self.last_sequence > MAX_SAFE_JSON_INTEGER
        {
            return Err(StateContractError::AuditExportRangeInvalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditContract {
    pub schema_generation: u32,
    pub contract_fingerprint: Digest,
    pub streams: Vec<AuditStreamSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditStreamSpec {
    pub stream: AuditStream,
    pub owner: AuditOwner,
    pub retention: AuditRetentionPolicy,
}

impl AuditContract {
    pub fn validate(&self) -> Result<(), StateContractError> {
        if self.schema_generation != STATE_SCHEMA_GENERATION {
            return Err(StateContractError::UnsupportedSchemaGeneration);
        }
        if self.streams.is_empty() || self.streams.len() > MAX_INVENTORY_ROWS {
            return Err(StateContractError::BoundExceeded);
        }
        let mut streams = BTreeSet::new();
        for stream in &self.streams {
            if !streams.insert(&stream.stream) {
                return Err(StateContractError::AuditStreamMismatch);
            }
            validate_audit_owner(&stream.stream, &stream.owner)?;
            stream.retention.validate()?;
            if stream.stream == AuditStream::LocalRoot
                && stream.retention.max_age_days != MAX_AUDIT_RETENTION_DAYS
            {
                return Err(StateContractError::RetentionOutOfBounds);
            }
        }
        if !streams.contains(&AuditStream::LocalRoot) {
            return Err(StateContractError::AuditStreamMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectionAuthority {
    DiagnosticsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectionStatus {
    Ready,
    Starting,
    Stopped,
    Degraded,
    Quarantined,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionEntry {
    pub scope: IdentityScope,
    pub status: ProjectionStatus,
    pub reason: Option<DegradedReason>,
    pub remediation: Option<Remediation>,
    pub observed_generation: Option<Generation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateProjection {
    pub schema_version: u32,
    pub generated_at_unix_ms: u64,
    pub authority: ProjectionAuthority,
    #[schemars(length(max = 4096))]
    pub entries: Vec<ProjectionEntry>,
}

impl StateProjection {
    pub fn validate(&self) -> Result<(), StateContractError> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(StateContractError::UnsupportedSchemaVersion);
        }
        if self.generated_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self.entries.len() > MAX_PROJECTION_ENTRIES
        {
            return Err(StateContractError::BoundExceeded);
        }
        Ok(())
    }

    pub const fn can_authorize(&self) -> bool {
        false
    }

    pub const fn can_repair(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateStorageSyncAuditContract {
    pub schema_version: u32,
    pub schema_generation: u32,
    pub contract_fingerprint: Digest,
    pub storage: StorageInventory,
    pub synchronization: SyncInventory,
    pub audit: AuditContract,
}

impl StateStorageSyncAuditContract {
    pub fn validate(&self) -> Result<(), StateContractError> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(StateContractError::UnsupportedSchemaVersion);
        }
        if self.schema_generation != STATE_SCHEMA_GENERATION {
            return Err(StateContractError::UnsupportedSchemaGeneration);
        }
        if self.storage.contract_fingerprint != self.contract_fingerprint
            || self.synchronization.contract_fingerprint != self.contract_fingerprint
            || self.audit.contract_fingerprint != self.contract_fingerprint
        {
            return Err(StateContractError::ContractFingerprintMismatch);
        }
        self.storage.validate()?;
        self.synchronization.validate()?;
        self.audit.validate()
    }
}
