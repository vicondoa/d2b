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
use sha2::{Digest as ShaDigest, Sha256};

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
    DuplicateIdentityScope,
    InventoryScopeMismatch,
    MissingParentScope,
    MissingMandatoryResource,
    DuplicateMandatoryResource,
    ScopeCategoryMismatch,
    LocationCategoryMismatch,
    AuthorityScopeMismatch,
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
    DiscoveryTimestampInvalid,
    DiscoveryFuture,
    DiscoveryStale,
    CleanupLockNotHeld,
    CleanupOwnershipEpochMismatch,
    CleanupWithoutOwnerAbsenceProof,
    InvalidAtomicTransition,
    SuccessBeforeParentFsync,
    EnvelopeChecksumMismatch,
    EnvelopePayloadMismatch,
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
            Self::DuplicateIdentityScope => "duplicate identity scope",
            Self::InventoryScopeMismatch => {
                "inventory scopes differ from trusted configured scopes"
            }
            Self::MissingParentScope => "configured identity scope is missing a required parent",
            Self::MissingMandatoryResource => "mandatory storage resource is missing",
            Self::DuplicateMandatoryResource => "mandatory storage resource is duplicated",
            Self::ScopeCategoryMismatch => "storage category and identity scope differ",
            Self::LocationCategoryMismatch => "storage category and logical location differ",
            Self::AuthorityScopeMismatch => "authority and identity scope differ",
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
            Self::DiscoveryTimestampInvalid => "restart discovery timestamps are invalid",
            Self::DiscoveryFuture => "restart discovery is from the future",
            Self::DiscoveryStale => "restart discovery is stale",
            Self::CleanupLockNotHeld => "cleanup ownership lock is not held",
            Self::CleanupOwnershipEpochMismatch => "cleanup ownership epoch differs",
            Self::CleanupWithoutOwnerAbsenceProof => "cleanup lacks exact owner-absence proof",
            Self::InvalidAtomicTransition => "invalid atomic write phase transition",
            Self::SuccessBeforeParentFsync => {
                "authoritative write cannot succeed before parent fsync"
            }
            Self::EnvelopeChecksumMismatch => "authoritative envelope checksum differs",
            Self::EnvelopePayloadMismatch => {
                "authoritative envelope payload bytes are not canonical or do not match"
            }
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

    fn from_bytes(bytes: [u8; 32]) -> Self {
        let mut encoded = String::with_capacity(64);
        for byte in bytes {
            use fmt::Write as _;
            write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
        }
        Self(encoded)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct OwnershipEpoch(Generation);

impl OwnershipEpoch {
    pub fn new(value: u64) -> Result<Self, StateContractError> {
        Ok(Self(Generation::new(value)?))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl<'de> Deserialize<'de> for OwnershipEpoch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct SafeJsonInteger(#[schemars(range(min = 0, max = 9007199254740991_u64))] u64);

impl SafeJsonInteger {
    pub fn new(value: u64) -> Result<Self, StateContractError> {
        ensure_safe_json_integer(value)?;
        Ok(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for SafeJsonInteger {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn ensure_safe_json_integer(value: u64) -> Result<(), StateContractError> {
    if value <= MAX_SAFE_JSON_INTEGER {
        Ok(())
    } else {
        Err(StateContractError::BoundExceeded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum IdentityScope {
    LocalRoot,
    Realm {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
    Workload {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
    },
    Provider {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "providerId")]
        provider_id: ProviderId,
    },
    Role {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
        #[serde(rename = "roleId")]
        role_id: RoleId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AuthorityRef {
    Pid1,
    LocalRootAllocator,
    LocalRootBroker,
    RealmController {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
    RealmBroker {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
    WorkloadController {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
    },
    WorkloadBroker {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
    },
    WorkloadRole {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
        #[serde(rename = "roleId")]
        role_id: RoleId,
    },
    Provider {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "providerId")]
        provider_id: ProviderId,
    },
}

fn authority_matches_scope(authority: &AuthorityRef, scope: &IdentityScope) -> bool {
    match (authority, scope) {
        (
            AuthorityRef::Pid1 | AuthorityRef::LocalRootAllocator | AuthorityRef::LocalRootBroker,
            IdentityScope::LocalRoot,
        ) => true,
        (
            AuthorityRef::RealmController {
                realm_id: authority,
            }
            | AuthorityRef::RealmBroker {
                realm_id: authority,
            },
            IdentityScope::Realm { realm_id: scope },
        ) => authority == scope,
        (
            AuthorityRef::WorkloadController {
                realm_id: authority_realm,
                workload_id: authority_workload,
            }
            | AuthorityRef::WorkloadBroker {
                realm_id: authority_realm,
                workload_id: authority_workload,
            },
            IdentityScope::Workload {
                realm_id: scope_realm,
                workload_id: scope_workload,
            },
        ) => authority_realm == scope_realm && authority_workload == scope_workload,
        (
            AuthorityRef::Provider {
                realm_id: authority_realm,
                provider_id: authority_provider,
            },
            IdentityScope::Provider {
                realm_id: scope_realm,
                provider_id: scope_provider,
            },
        ) => authority_realm == scope_realm && authority_provider == scope_provider,
        (
            AuthorityRef::WorkloadRole {
                realm_id: authority_realm,
                workload_id: authority_workload,
                role_id: authority_role,
            },
            IdentityScope::Role {
                realm_id: scope_realm,
                workload_id: scope_workload,
                role_id: scope_role,
            },
        ) => {
            authority_realm == scope_realm
                && authority_workload == scope_workload
                && authority_role == scope_role
        }
        _ => false,
    }
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
    pub applicable_scopes: Vec<IdentityScope>,
    #[schemars(length(min = 1, max = 4096))]
    pub resources: Vec<StorageResource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MandatoryScopeKind {
    LocalRoot,
    Realm,
    Workload,
    Provider,
    Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MandatoryResourceSpec {
    pub category: StorageCategory,
    pub logical_location: LogicalLocation,
    scope_kind: MandatoryScopeKind,
}

pub const MANDATORY_RESOURCE_CATALOG: [MandatoryResourceSpec; 10] = [
    MandatoryResourceSpec {
        category: StorageCategory::LocalRoot,
        logical_location: LogicalLocation::HostBroker,
        scope_kind: MandatoryScopeKind::LocalRoot,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Realm,
        logical_location: LogicalLocation::RealmController,
        scope_kind: MandatoryScopeKind::Realm,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Lock,
        logical_location: LogicalLocation::RuntimeLocks,
        scope_kind: MandatoryScopeKind::Realm,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Quarantine,
        logical_location: LogicalLocation::Quarantine,
        scope_kind: MandatoryScopeKind::Realm,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Audit,
        logical_location: LogicalLocation::RealmAudit,
        scope_kind: MandatoryScopeKind::Realm,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Projection,
        logical_location: LogicalLocation::Projection,
        scope_kind: MandatoryScopeKind::Realm,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Workload,
        logical_location: LogicalLocation::WorkloadState,
        scope_kind: MandatoryScopeKind::Workload,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Lease,
        logical_location: LogicalLocation::RuntimeLeases,
        scope_kind: MandatoryScopeKind::Workload,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Provider,
        logical_location: LogicalLocation::ProviderState,
        scope_kind: MandatoryScopeKind::Provider,
    },
    MandatoryResourceSpec {
        category: StorageCategory::Runtime,
        logical_location: LogicalLocation::RuntimeRole,
        scope_kind: MandatoryScopeKind::Role,
    },
];

impl StorageInventory {
    /// Validates only the inventory's self-contained structure.
    ///
    /// Completeness is configuration-relative and requires
    /// [`Self::validate_complete`] with trusted expected scopes.
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
        if self.applicable_scopes.is_empty() || self.applicable_scopes.len() > MAX_INVENTORY_ROWS {
            return Err(StateContractError::EmptyInventory);
        }
        let applicable_scopes = self.applicable_scopes.iter().collect::<BTreeSet<_>>();
        if applicable_scopes.len() != self.applicable_scopes.len() {
            return Err(StateContractError::DuplicateIdentityScope);
        }

        let mut ids = BTreeSet::new();
        let mut resource_keys = BTreeSet::new();
        for resource in &self.resources {
            if !ids.insert(resource.resource_id.as_str()) {
                return Err(StateContractError::DuplicateResourceId);
            }
            if !resource_keys.insert((
                resource.category,
                resource.logical_location,
                &resource.scope,
            )) {
                return Err(StateContractError::DuplicateMandatoryResource);
            }
            if resource.ownership.owner != resource.repair_authority {
                return Err(StateContractError::RepairAuthorityMismatch);
            }
            validate_category_scope(resource)?;
            validate_category_location(resource)?;
            if !applicable_scopes.contains(&resource.scope) {
                return Err(StateContractError::ScopeCategoryMismatch);
            }
            for authority in [
                &resource.creation_authority,
                &resource.reconcile_authority,
                &resource.repair_authority,
                &resource.delete_authority,
                &resource.ownership.owner,
            ] {
                if !authority_matches_scope(authority, &resource.scope) {
                    return Err(StateContractError::AuthorityScopeMismatch);
                }
            }
        }
        Ok(())
    }

    pub fn validate_complete(
        &self,
        expected_scopes: &[IdentityScope],
    ) -> Result<(), StateContractError> {
        self.validate()?;
        if expected_scopes.is_empty() || expected_scopes.len() > MAX_INVENTORY_ROWS {
            return Err(StateContractError::InventoryScopeMismatch);
        }
        let expected = expected_scopes.iter().collect::<BTreeSet<_>>();
        if expected.len() != expected_scopes.len() {
            return Err(StateContractError::DuplicateIdentityScope);
        }
        validate_scope_parents(&expected)?;
        let declared = self.applicable_scopes.iter().collect::<BTreeSet<_>>();
        if declared != expected {
            return Err(StateContractError::InventoryScopeMismatch);
        }
        self.validate_mandatory_catalog(expected_scopes)
    }

    fn validate_mandatory_catalog(
        &self,
        expected_scopes: &[IdentityScope],
    ) -> Result<(), StateContractError> {
        for scope in expected_scopes {
            let kind = match scope {
                IdentityScope::LocalRoot => MandatoryScopeKind::LocalRoot,
                IdentityScope::Realm { .. } => MandatoryScopeKind::Realm,
                IdentityScope::Workload { .. } => MandatoryScopeKind::Workload,
                IdentityScope::Provider { .. } => MandatoryScopeKind::Provider,
                IdentityScope::Role { .. } => MandatoryScopeKind::Role,
            };
            for spec in MANDATORY_RESOURCE_CATALOG
                .iter()
                .filter(|spec| spec.scope_kind == kind)
            {
                let count = self
                    .resources
                    .iter()
                    .filter(|resource| {
                        resource.category == spec.category
                            && resource.logical_location == spec.logical_location
                            && &resource.scope == scope
                    })
                    .count();
                match count {
                    1 => {}
                    0 => return Err(StateContractError::MissingMandatoryResource),
                    _ => return Err(StateContractError::DuplicateMandatoryResource),
                }
            }
        }
        Ok(())
    }
}

fn validate_scope_parents(expected: &BTreeSet<&IdentityScope>) -> Result<(), StateContractError> {
    if !expected.contains(&IdentityScope::LocalRoot) {
        return Err(StateContractError::MissingParentScope);
    }
    for scope in expected {
        let realm_parent = match scope {
            IdentityScope::Realm { .. } | IdentityScope::LocalRoot => None,
            IdentityScope::Workload { realm_id, .. }
            | IdentityScope::Provider { realm_id, .. }
            | IdentityScope::Role { realm_id, .. } => Some(IdentityScope::Realm {
                realm_id: realm_id.clone(),
            }),
        };
        if let Some(realm_parent) = realm_parent
            && !expected.contains(&realm_parent)
        {
            return Err(StateContractError::MissingParentScope);
        }
        if let IdentityScope::Role {
            realm_id,
            workload_id,
            ..
        } = scope
        {
            let workload_parent = IdentityScope::Workload {
                realm_id: realm_id.clone(),
                workload_id: workload_id.clone(),
            };
            if !expected.contains(&workload_parent) {
                return Err(StateContractError::MissingParentScope);
            }
        }
    }
    Ok(())
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

pub trait CanonicalPayloadVerifier<T> {
    fn decode_canonical(&self, raw_payload: &[u8]) -> Result<T, StateContractError>;
}

const STATE_PAYLOAD_DIGEST_DOMAIN: &[u8] = b"d2b.v2.state-envelope.payload.sha256\0";
const AUDIT_RECORD_DIGEST_DOMAIN: &[u8] = b"d2b.v2.audit.record.sha256\0";
const AUDIT_SEGMENT_DIGEST_DOMAIN: &[u8] = b"d2b.v2.audit.segment.sha256\0";
const AUDIT_CHECKPOINT_DIGEST_DOMAIN: &[u8] = b"d2b.v2.audit.checkpoint.sha256\0";

fn digest_domain_bytes(domain: &[u8], payload: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((payload.len() as u64).to_be_bytes());
    hasher.update(payload);
    Digest::from_bytes(hasher.finalize().into())
}

pub fn state_payload_digest(raw_payload: &[u8]) -> Result<Digest, StateContractError> {
    if raw_payload.is_empty() || raw_payload.len() as u64 > MAX_JSON_DOCUMENT_BYTES {
        return Err(StateContractError::BoundExceeded);
    }
    Ok(digest_domain_bytes(
        STATE_PAYLOAD_DIGEST_DOMAIN,
        raw_payload,
    ))
}

impl<T: PartialEq> StateEnvelope<T> {
    fn validate_header(&self) -> Result<(), StateContractError> {
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

    pub fn validate_payload_bytes<V>(
        &self,
        raw_payload: &[u8],
        verifier: &V,
    ) -> Result<(), StateContractError>
    where
        V: CanonicalPayloadVerifier<T>,
    {
        self.validate_header()?;
        let encoded_bytes =
            u64::try_from(raw_payload.len()).map_err(|_| StateContractError::BoundExceeded)?;
        if encoded_bytes != self.encoded_bytes || encoded_bytes > MAX_JSON_DOCUMENT_BYTES {
            return Err(StateContractError::BoundExceeded);
        }
        if digest_domain_bytes(STATE_PAYLOAD_DIGEST_DOMAIN, raw_payload) != self.checksum {
            return Err(StateContractError::EnvelopeChecksumMismatch);
        }
        let decoded = verifier.decode_canonical(raw_payload)?;
        if decoded != self.payload {
            return Err(StateContractError::EnvelopePayloadMismatch);
        }
        Ok(())
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
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub role_id: RoleId,
    pub candidate_count: u16,
    pub pidfd_persistence: PidfdPersistence,
    pub identity: EvidenceVerdict,
    pub cgroup_identity: Digest,
    pub cgroup_membership: EvidenceVerdict,
    pub executable_fingerprint: Digest,
    pub executable: EvidenceVerdict,
    pub configuration_fingerprint: Digest,
    pub configuration: EvidenceVerdict,
    pub config_generation: Generation,
    pub runner_generation: Generation,
    pub ownership_epoch: OwnershipEpoch,
    pub generation: EvidenceVerdict,
}

impl RunnerEvidence {
    pub fn is_exact_for(&self, target: &RunnerAdoptionTarget) -> bool {
        self.candidate_count == 1
            && self.realm_id == target.realm_id
            && self.workload_id == target.workload_id
            && self.role_id == target.role_id
            && self.cgroup_identity == target.cgroup_identity
            && self.executable_fingerprint == target.executable_fingerprint
            && self.configuration_fingerprint == target.configuration_fingerprint
            && self.config_generation == target.config_generation
            && self.runner_generation == target.runner_generation
            && self.ownership_epoch == target.ownership_epoch
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

    fn proves_absence_for(&self, target: &RunnerCleanupTarget) -> bool {
        self.candidate_count == 0
            && self.realm_id == target.realm_id
            && self.workload_id == target.workload_id
            && self.role_id == target.role_id
            && self.cgroup_identity == target.cgroup_identity
            && self.executable_fingerprint == target.executable_fingerprint
            && self.configuration_fingerprint == target.configuration_fingerprint
            && self.config_generation == target.config_generation
            && self.runner_generation == target.runner_generation
            && self.ownership_epoch == target.ownership_epoch
            && [
                self.identity,
                self.cgroup_membership,
                self.executable,
                self.configuration,
                self.generation,
            ]
            .into_iter()
            .all(|verdict| verdict == EvidenceVerdict::Missing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerAdoptionTarget {
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub role_id: RoleId,
    pub config_generation: Generation,
    pub cgroup_identity: Digest,
    pub executable_fingerprint: Digest,
    pub configuration_fingerprint: Digest,
    pub runner_generation: Generation,
    pub ownership_epoch: OwnershipEpoch,
}

impl RunnerAdoptionTarget {
    fn scope(&self) -> IdentityScope {
        IdentityScope::Role {
            realm_id: self.realm_id.clone(),
            workload_id: self.workload_id.clone(),
            role_id: self.role_id.clone(),
        }
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
    pub observation_id: ResourceId,
    pub resource_id: ResourceId,
    pub scope: IdentityScope,
    pub configuration_fingerprint: Digest,
    pub resource_generation: Generation,
    pub ownership_epoch: OwnershipEpoch,
    pub presence: EvidenceVerdict,
    pub owner_identity: EvidenceVerdict,
    pub ownership: EvidenceVerdict,
    pub configuration: EvidenceVerdict,
    pub generation: EvidenceVerdict,
    pub lease: EvidenceVerdict,
}

impl ResourceEvidence {
    pub fn is_exact(&self) -> bool {
        [
            self.presence,
            self.owner_identity,
            self.ownership,
            self.configuration,
            self.generation,
            self.lease,
        ]
        .into_iter()
        .all(|verdict| verdict == EvidenceVerdict::Match)
    }

    fn proves_absence_for(&self, target: &ResourceCleanupTarget) -> bool {
        self.resource_id == target.resource_id
            && self.scope == target.scope
            && self.configuration_fingerprint == target.configuration_fingerprint
            && self.resource_generation == target.resource_generation
            && self.ownership_epoch == target.ownership_epoch
            && [
                self.presence,
                self.owner_identity,
                self.ownership,
                self.configuration,
                self.generation,
                self.lease,
            ]
            .into_iter()
            .all(|verdict| verdict == EvidenceVerdict::Missing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestartDiscovery {
    pub discovery_id: ResourceId,
    pub config_generation: Generation,
    pub issued_at_unix_ms: SafeJsonInteger,
    pub completed_at_unix_ms: SafeJsonInteger,
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

    fn validate_freshness(&self, freshness: DiscoveryFreshness) -> Result<(), StateContractError> {
        let issued = self.issued_at_unix_ms.get();
        let completed = self.completed_at_unix_ms.get();
        let now = freshness.trusted_now_unix_ms.get();
        let max_age = freshness.max_age_ms.get();
        if issued == 0 || completed == 0 || max_age == 0 || completed < issued {
            return Err(StateContractError::DiscoveryTimestampInvalid);
        }
        if issued > now || completed > now {
            return Err(StateContractError::DiscoveryFuture);
        }
        if now - issued > max_age {
            return Err(StateContractError::DiscoveryStale);
        }
        Ok(())
    }

    pub fn prove_owner_absence<G: CleanupLockGuard + ?Sized>(
        &self,
        target: CleanupTarget,
        freshness: DiscoveryFreshness,
        guard: &G,
    ) -> Result<OwnerAbsenceProof, StateContractError> {
        self.validate_bounds()?;
        self.validate_freshness(freshness)?;
        validate_cleanup_guard(self, &target, guard)?;

        let (observation_id, observation_class) = match &target {
            CleanupTarget::Runner { runner } => {
                let scope = runner.scope();
                let target_class = self
                    .runners
                    .iter()
                    .filter(|observation| observation.scope == scope)
                    .collect::<Vec<_>>();
                if target_class.len() > 1 {
                    return Err(StateContractError::RestartAmbiguous);
                }
                if target_class
                    .iter()
                    .any(|observation| observation.evidence.candidate_count != 0)
                {
                    return Err(StateContractError::CleanupWithoutOwnerAbsenceProof);
                }
                let Some(observation) = target_class
                    .into_iter()
                    .find(|observation| observation.evidence.proves_absence_for(runner))
                else {
                    return Err(StateContractError::RestartEvidenceIncomplete);
                };
                (
                    observation.observation_id.clone(),
                    AbsenceObservationClass::Runner,
                )
            }
            CleanupTarget::Resource { resource } => {
                let target_class = self
                    .resources
                    .iter()
                    .filter(|observation| observation.resource_id == resource.resource_id)
                    .collect::<Vec<_>>();
                if target_class.len() > 1 {
                    return Err(StateContractError::RestartAmbiguous);
                }
                if target_class
                    .iter()
                    .any(|observation| observation.presence != EvidenceVerdict::Missing)
                {
                    return Err(StateContractError::CleanupWithoutOwnerAbsenceProof);
                }
                let Some(observation) = target_class
                    .into_iter()
                    .find(|observation| observation.proves_absence_for(resource))
                else {
                    return Err(StateContractError::RestartEvidenceIncomplete);
                };
                (
                    observation.observation_id.clone(),
                    AbsenceObservationClass::Resource,
                )
            }
        };

        Ok(OwnerAbsenceProof {
            discovery_id: self.discovery_id.clone(),
            config_generation: self.config_generation,
            issued_at_unix_ms: self.issued_at_unix_ms,
            completed_at_unix_ms: self.completed_at_unix_ms,
            target,
            observation_id,
            observation_class,
            lock_id: guard.lock_id().clone(),
            ownership_epoch: guard.ownership_epoch(),
            evidence: AbsenceEvidence::CompletedDiscovery,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryFreshness {
    pub trusted_now_unix_ms: SafeJsonInteger,
    pub max_age_ms: SafeJsonInteger,
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
pub enum AbsenceEvidence {
    CompletedDiscovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AbsenceObservationClass {
    Runner,
    Resource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerCleanupTarget {
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub role_id: RoleId,
    pub cgroup_identity: Digest,
    pub executable_fingerprint: Digest,
    pub configuration_fingerprint: Digest,
    pub config_generation: Generation,
    pub runner_generation: Generation,
    pub ownership_epoch: OwnershipEpoch,
}

impl RunnerCleanupTarget {
    fn scope(&self) -> IdentityScope {
        IdentityScope::Role {
            realm_id: self.realm_id.clone(),
            workload_id: self.workload_id.clone(),
            role_id: self.role_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceCleanupTarget {
    pub resource_id: ResourceId,
    pub scope: IdentityScope,
    pub configuration_fingerprint: Digest,
    pub resource_generation: Generation,
    pub ownership_epoch: OwnershipEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CleanupTarget {
    Resource { resource: ResourceCleanupTarget },
    Runner { runner: RunnerCleanupTarget },
}

impl CleanupTarget {
    pub fn ownership_epoch(&self) -> OwnershipEpoch {
        match self {
            Self::Resource { resource } => resource.ownership_epoch,
            Self::Runner { runner } => runner.ownership_epoch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerAbsenceProof {
    pub discovery_id: ResourceId,
    pub config_generation: Generation,
    pub issued_at_unix_ms: SafeJsonInteger,
    pub completed_at_unix_ms: SafeJsonInteger,
    pub target: CleanupTarget,
    pub observation_id: ResourceId,
    pub observation_class: AbsenceObservationClass,
    pub lock_id: ResourceId,
    pub ownership_epoch: OwnershipEpoch,
    pub evidence: AbsenceEvidence,
}

pub trait CleanupLockGuard {
    fn lock_id(&self) -> &ResourceId;
    fn target(&self) -> &CleanupTarget;
    fn ownership_epoch(&self) -> OwnershipEpoch;
    fn acquired_at_unix_ms(&self) -> SafeJsonInteger;
    fn is_held(&self) -> bool;
    fn owner_absent(&self) -> bool;
}

fn validate_cleanup_guard<G: CleanupLockGuard + ?Sized>(
    discovery: &RestartDiscovery,
    target: &CleanupTarget,
    guard: &G,
) -> Result<(), StateContractError> {
    if !guard.is_held() || guard.target() != target {
        return Err(StateContractError::CleanupLockNotHeld);
    }
    if guard.ownership_epoch() != target.ownership_epoch() {
        return Err(StateContractError::CleanupOwnershipEpochMismatch);
    }
    if guard.acquired_at_unix_ms().get() > discovery.issued_at_unix_ms.get() {
        return Err(StateContractError::CleanupLockNotHeld);
    }
    if !guard.owner_absent() {
        return Err(StateContractError::CleanupWithoutOwnerAbsenceProof);
    }
    Ok(())
}

pub struct HeldCleanupAuthorization<'guard, G: CleanupLockGuard + ?Sized> {
    guard: &'guard G,
    target: &'guard CleanupTarget,
}

impl<G: CleanupLockGuard + ?Sized> HeldCleanupAuthorization<'_, G> {
    pub fn target(&self) -> &CleanupTarget {
        self.target
    }

    pub fn ownership_epoch(&self) -> OwnershipEpoch {
        self.guard.ownership_epoch()
    }

    pub fn is_held(&self) -> bool {
        self.guard.is_held()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AdoptionDecision {
    Adopt {
        #[serde(rename = "freshPidfdOpened")]
        fresh_pidfd_opened: bool,
    },
    Quarantine {
        reason: QuarantineReason,
        remediation: Remediation,
    },
    Cleanup {
        target: Box<CleanupTarget>,
        #[serde(rename = "ownerAbsenceProof")]
        owner_absence_proof: Box<OwnerAbsenceProof>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestartDecision {
    pub observation_id: ResourceId,
    pub ordering: RecoveryOrdering,
    pub decision: AdoptionDecision,
}

impl RestartDecision {
    pub fn validate_for_runner(
        &self,
        observation: &RunnerObservation,
        target: &RunnerAdoptionTarget,
    ) -> Result<(), StateContractError> {
        if self.observation_id != observation.observation_id {
            return Err(StateContractError::RestartEvidenceIncomplete);
        }
        let exact =
            observation.scope == target.scope() && observation.evidence.is_exact_for(target);
        match &self.decision {
            AdoptionDecision::Adopt { fresh_pidfd_opened } => {
                if !exact {
                    return Err(StateContractError::RestartAmbiguous);
                }
                if !fresh_pidfd_opened {
                    return Err(StateContractError::RestartEvidenceIncomplete);
                }
            }
            AdoptionDecision::Quarantine { .. } if exact => {
                return Err(StateContractError::RestartEvidenceIncomplete);
            }
            AdoptionDecision::Cleanup { .. } => {
                return Err(StateContractError::RestartAmbiguous);
            }
            AdoptionDecision::Quarantine { .. } => {}
        }
        Ok(())
    }

    pub fn validate_cleanup<'guard, G: CleanupLockGuard + ?Sized>(
        &'guard self,
        discovery: &'guard RestartDiscovery,
        expected_generation: Generation,
        freshness: DiscoveryFreshness,
        guard: &'guard G,
    ) -> Result<HeldCleanupAuthorization<'guard, G>, StateContractError> {
        let AdoptionDecision::Cleanup {
            target,
            owner_absence_proof,
        } = &self.decision
        else {
            return Err(StateContractError::CleanupWithoutOwnerAbsenceProof);
        };
        let target = target.as_ref();
        let owner_absence_proof = owner_absence_proof.as_ref();
        let expected = discovery.prove_owner_absence(target.clone(), freshness, guard)?;
        if owner_absence_proof != &expected
            || owner_absence_proof.target != *target
            || owner_absence_proof.config_generation != expected_generation
            || owner_absence_proof.ownership_epoch != guard.ownership_epoch()
            || self.observation_id != owner_absence_proof.observation_id
        {
            return Err(StateContractError::CleanupWithoutOwnerAbsenceProof);
        }
        validate_cleanup_guard(discovery, target, guard)?;
        Ok(HeldCleanupAuthorization { guard, target })
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
            if !authority_matches_scope(&lock.owner, &lock.key.scope)
                || !authority_matches_scope(&lock.release_authority, &lock.key.scope)
            {
                return Err(StateContractError::AuthorityScopeMismatch);
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
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AuditStream {
    LocalRoot,
    Realm {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AuditOwner {
    LocalRootBroker,
    RealmBroker {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
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
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AuditActor {
    LocalRootAllocator,
    LocalRootBroker,
    RealmController {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
    RealmBroker {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
    Provider {
        #[serde(rename = "providerId")]
        provider_id: ProviderId,
    },
    WorkloadRole {
        #[serde(rename = "roleId")]
        role_id: RoleId,
    },
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

struct CanonicalHasher(Sha256);

impl CanonicalHasher {
    fn new(domain: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(domain);
        Self(hasher)
    }

    fn bytes(&mut self, value: &[u8]) {
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.update(value.to_be_bytes());
    }

    fn optional_string(&mut self, value: Option<&str>) {
        self.0.update([u8::from(value.is_some())]);
        if let Some(value) = value {
            self.string(value);
        }
    }

    fn finish(self) -> Digest {
        Digest::from_bytes(self.0.finalize().into())
    }
}

fn hash_audit_stream(hasher: &mut CanonicalHasher, stream: &AuditStream) {
    match stream {
        AuditStream::LocalRoot => hasher.string("local-root"),
        AuditStream::Realm { realm_id } => {
            hasher.string("realm");
            hasher.string(realm_id.as_str());
        }
    }
}

fn hash_audit_owner(hasher: &mut CanonicalHasher, owner: &AuditOwner) {
    match owner {
        AuditOwner::LocalRootBroker => hasher.string("local-root-broker"),
        AuditOwner::RealmBroker { realm_id } => {
            hasher.string("realm-broker");
            hasher.string(realm_id.as_str());
        }
    }
}

fn hash_audit_actor(hasher: &mut CanonicalHasher, actor: &AuditActor) {
    match actor {
        AuditActor::LocalRootAllocator => hasher.string("local-root-allocator"),
        AuditActor::LocalRootBroker => hasher.string("local-root-broker"),
        AuditActor::RealmController { realm_id } => {
            hasher.string("realm-controller");
            hasher.string(realm_id.as_str());
        }
        AuditActor::RealmBroker { realm_id } => {
            hasher.string("realm-broker");
            hasher.string(realm_id.as_str());
        }
        AuditActor::Provider { provider_id } => {
            hasher.string("provider");
            hasher.string(provider_id.as_str());
        }
        AuditActor::WorkloadRole { role_id } => {
            hasher.string("workload-role");
            hasher.string(role_id.as_str());
        }
    }
}

impl AuditRecord {
    fn validate_bounds(&self) -> Result<(), StateContractError> {
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

    pub fn computed_hash(&self) -> Digest {
        let mut hasher = CanonicalHasher::new(AUDIT_RECORD_DIGEST_DOMAIN);
        hasher.u64(u64::from(self.schema_version));
        hash_audit_stream(&mut hasher, &self.stream);
        hasher.u64(self.sequence);
        hasher.u64(self.occurred_at_unix_ms);
        hasher.string(self.correlation.operation_id.as_str());
        hasher.optional_string(
            self.correlation
                .session_id
                .as_ref()
                .map(CorrelationId::as_str),
        );
        hasher.optional_string(
            self.correlation
                .provider_id
                .as_ref()
                .map(ProviderId::as_str),
        );
        hash_audit_actor(&mut hasher, &self.actor);
        hasher.string(match self.event {
            AuditEvent::StorageCreate => "storage-create",
            AuditEvent::StorageReconcile => "storage-reconcile",
            AuditEvent::StorageRepair => "storage-repair",
            AuditEvent::StorageDelete => "storage-delete",
            AuditEvent::RestartDiscover => "restart-discover",
            AuditEvent::RestartAdopt => "restart-adopt",
            AuditEvent::RestartQuarantine => "restart-quarantine",
            AuditEvent::LockAcquire => "lock-acquire",
            AuditEvent::LockRelease => "lock-release",
            AuditEvent::LeaseGrant => "lease-grant",
            AuditEvent::LeaseRevoke => "lease-revoke",
            AuditEvent::ProviderOperation => "provider-operation",
            AuditEvent::SessionOperation => "session-operation",
            AuditEvent::SegmentSeal => "segment-seal",
            AuditEvent::Checkpoint => "checkpoint",
            AuditEvent::GapDetected => "gap-detected",
            AuditEvent::RetentionPrune => "retention-prune",
            AuditEvent::Export => "export",
        });
        hasher.string(match self.outcome {
            AuditOutcome::Succeeded => "succeeded",
            AuditOutcome::Denied => "denied",
            AuditOutcome::Failed => "failed",
            AuditOutcome::Degraded => "degraded",
            AuditOutcome::Quarantined => "quarantined",
            AuditOutcome::Cancelled => "cancelled",
        });
        hasher.string(match self.reason {
            AuditReason::PolicyAllowed => "policy-allowed",
            AuditReason::PolicyDenied => "policy-denied",
            AuditReason::IdentityVerified => "identity-verified",
            AuditReason::IdentityMismatch => "identity-mismatch",
            AuditReason::GenerationMismatch => "generation-mismatch",
            AuditReason::OwnerAmbiguous => "owner-ambiguous",
            AuditReason::StorageDrift => "storage-drift",
            AuditReason::LockContended => "lock-contended",
            AuditReason::LeaseExpired => "lease-expired",
            AuditReason::CorruptState => "corrupt-state",
            AuditReason::SequenceGap => "sequence-gap",
            AuditReason::RetentionLimit => "retention-limit",
            AuditReason::OperatorRequested => "operator-requested",
        });
        hasher.string(self.previous_hash.as_str());
        hasher.u64(u64::from(self.encoded_bytes));
        hasher.finish()
    }

    pub fn validate_integrity(&self) -> Result<(), StateContractError> {
        self.validate_bounds()?;
        if self.record_hash != self.computed_hash() {
            return Err(StateContractError::AuditChainMismatch);
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
    pub fn computed_digest(&self) -> Digest {
        let mut hasher = CanonicalHasher::new(AUDIT_SEGMENT_DIGEST_DOMAIN);
        hash_audit_stream(&mut hasher, &self.summary.stream);
        hash_audit_owner(&mut hasher, &self.summary.owner);
        hasher.string(self.summary.segment_id.as_str());
        hasher.u64(self.summary.first_sequence);
        hasher.u64(self.summary.last_sequence);
        hasher.string(self.summary.previous_segment_digest.as_str());
        hasher.u64(self.summary.controller_generation.get());
        hasher.u64(self.summary.created_at_unix_ms);
        hasher.u64(self.summary.sealed_at_unix_ms);
        hasher.u64(self.summary.encoded_bytes);
        hasher.string(match self.summary.prune_status {
            PruneStatus::Retained => "retained",
            PruneStatus::EligibleAfterCheckpoint => "eligible-after-checkpoint",
            PruneStatus::PrunedWithCheckpoint => "pruned-with-checkpoint",
        });
        hasher.u64(self.records.len() as u64);
        for record in &self.records {
            hasher.string(record.record_hash.as_str());
        }
        hasher.finish()
    }

    pub fn validate(&self) -> Result<(), StateContractError> {
        self.validate_internal()?;
        if self.summary.first_sequence != 1
            || !self
                .summary
                .previous_segment_digest
                .as_str()
                .bytes()
                .all(|byte| byte == b'0')
        {
            return Err(StateContractError::AuditChainMismatch);
        }
        Ok(())
    }

    pub fn validate_after(&self, previous: &AuditSegment) -> Result<(), StateContractError> {
        previous.validate_internal()?;
        self.validate_internal()?;
        if self.summary.stream != previous.summary.stream
            || self.summary.owner != previous.summary.owner
            || self.summary.previous_segment_digest != previous.summary.segment_digest
            || previous
                .summary
                .last_sequence
                .checked_add(1)
                .ok_or(StateContractError::BoundExceeded)?
                != self.summary.first_sequence
        {
            return Err(StateContractError::AuditChainMismatch);
        }
        Ok(())
    }

    fn validate_internal(&self) -> Result<(), StateContractError> {
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
            record.validate_integrity()?;
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
        if self.records.last().map(|record| record.sequence) != Some(self.summary.last_sequence) {
            return Err(StateContractError::AuditChainMismatch);
        }
        if self.summary.segment_digest != self.computed_digest() {
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

pub trait AuditCheckpointSignatureVerifier {
    fn verify_realm_signature(
        &self,
        realm_id: &RealmId,
        checkpoint_digest: &Digest,
        signature_digest: &Digest,
    ) -> bool;
}

impl AuditCheckpoint {
    pub fn computed_digest(&self) -> Digest {
        let mut hasher = CanonicalHasher::new(AUDIT_CHECKPOINT_DIGEST_DOMAIN);
        hash_audit_stream(&mut hasher, &self.stream);
        hash_audit_owner(&mut hasher, &self.owner);
        hasher.string(self.checkpoint_id.as_str());
        hasher.u64(self.through_sequence);
        hasher.string(self.segment_digest.as_str());
        hasher.string(self.previous_checkpoint_digest.as_str());
        hasher.u64(self.controller_generation.get());
        hasher.u64(self.created_at_unix_ms);
        hasher.finish()
    }

    pub fn verify_for_segment<V: AuditCheckpointSignatureVerifier>(
        &self,
        segment: &AuditSegment,
        previous_checkpoint: Option<&AuditCheckpoint>,
        signature_verifier: &V,
    ) -> Result<(), StateContractError> {
        segment.validate_internal()?;
        validate_audit_owner(&self.stream, &self.owner)?;
        if self.through_sequence == 0
            || self.through_sequence > MAX_SAFE_JSON_INTEGER
            || self.created_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self.stream != segment.summary.stream
            || self.owner != segment.summary.owner
            || self.through_sequence != segment.summary.last_sequence
            || self.segment_digest != segment.summary.segment_digest
            || self.controller_generation != segment.summary.controller_generation
            || self.created_at_unix_ms < segment.summary.sealed_at_unix_ms
            || self.checkpoint_digest != self.computed_digest()
        {
            return Err(StateContractError::AuditCheckpointMismatch);
        }

        match previous_checkpoint {
            Some(previous) => {
                if previous.stream != self.stream
                    || previous.owner != self.owner
                    || previous.checkpoint_digest != previous.computed_digest()
                    || self.previous_checkpoint_digest != previous.checkpoint_digest
                    || previous
                        .through_sequence
                        .checked_add(1)
                        .ok_or(StateContractError::BoundExceeded)?
                        != segment.summary.first_sequence
                    || segment.summary.previous_segment_digest != previous.segment_digest
                {
                    return Err(StateContractError::AuditCheckpointMismatch);
                }
                previous.verify_signature(signature_verifier)?;
            }
            None => {
                if segment.summary.first_sequence != 1
                    || !segment
                        .summary
                        .previous_segment_digest
                        .as_str()
                        .bytes()
                        .all(|byte| byte == b'0')
                    || !self
                        .previous_checkpoint_digest
                        .as_str()
                        .bytes()
                        .all(|byte| byte == b'0')
                {
                    return Err(StateContractError::AuditCheckpointMismatch);
                }
            }
        }
        self.verify_signature(signature_verifier)
    }

    fn verify_signature<V: AuditCheckpointSignatureVerifier>(
        &self,
        signature_verifier: &V,
    ) -> Result<(), StateContractError> {
        match (&self.stream, &self.realm_signature_digest) {
            (AuditStream::LocalRoot, None) => Ok(()),
            (AuditStream::Realm { realm_id }, Some(signature))
                if signature_verifier.verify_realm_signature(
                    realm_id,
                    &self.checkpoint_digest,
                    signature,
                ) =>
            {
                Ok(())
            }
            _ => Err(StateContractError::AuditCheckpointMismatch),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditGap {
    pub stream: AuditStream,
    pub expected_sequence: SafeJsonInteger,
    pub observed_sequence: SafeJsonInteger,
    pub detected_at_unix_ms: SafeJsonInteger,
    pub reason: AuditReason,
}

pub fn detect_audit_gap(
    stream: AuditStream,
    expected_sequence: u64,
    observed_sequence: u64,
    detected_at_unix_ms: u64,
) -> Result<Option<AuditGap>, StateContractError> {
    if expected_sequence == 0 || observed_sequence == 0 {
        return Err(StateContractError::BoundExceeded);
    }
    let expected_sequence = SafeJsonInteger::new(expected_sequence)?;
    let observed_sequence = SafeJsonInteger::new(observed_sequence)?;
    let detected_at_unix_ms = SafeJsonInteger::new(detected_at_unix_ms)?;
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

pub struct AuditRetentionEvidence<'a, V> {
    pub age_days: u16,
    pub segment_bytes: u64,
    pub record_count: u32,
    pub sealed_segment: Option<&'a AuditSegment>,
    pub checkpoint: Option<&'a AuditCheckpoint>,
    pub previous_checkpoint: Option<&'a AuditCheckpoint>,
    pub signature_verifier: &'a V,
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

    pub fn decide<V: AuditCheckpointSignatureVerifier>(
        &self,
        evidence: AuditRetentionEvidence<'_, V>,
    ) -> Result<AuditRetentionDecision, StateContractError> {
        self.validate()?;
        let over_limit = evidence.age_days >= self.max_age_days
            || evidence.segment_bytes >= self.max_segment_bytes
            || evidence.record_count >= self.max_records_per_segment;
        if !over_limit {
            return Ok(AuditRetentionDecision::Retain);
        }
        let Some(segment) = evidence.sealed_segment else {
            return Ok(AuditRetentionDecision::SealCurrentSegment);
        };
        segment.validate_internal()?;
        match evidence.previous_checkpoint {
            Some(previous)
                if previous.stream == segment.summary.stream
                    && previous.owner == segment.summary.owner
                    && previous.checkpoint_digest == previous.computed_digest()
                    && previous.segment_digest == segment.summary.previous_segment_digest
                    && previous
                        .through_sequence
                        .checked_add(1)
                        .ok_or(StateContractError::BoundExceeded)?
                        == segment.summary.first_sequence =>
            {
                previous.verify_signature(evidence.signature_verifier)?;
            }
            Some(_) => return Err(StateContractError::AuditCheckpointMismatch),
            None if segment.summary.first_sequence == 1
                && segment
                    .summary
                    .previous_segment_digest
                    .as_str()
                    .bytes()
                    .all(|byte| byte == b'0') => {}
            None => return Err(StateContractError::AuditCheckpointMismatch),
        }
        if segment.summary.encoded_bytes != evidence.segment_bytes
            || segment.records.len() as u32 != evidence.record_count
            || segment.summary.prune_status != PruneStatus::EligibleAfterCheckpoint
        {
            return Err(StateContractError::AuditCheckpointMismatch);
        }
        let Some(checkpoint) = evidence.checkpoint else {
            return Ok(AuditRetentionDecision::CreateCheckpoint);
        };
        checkpoint.verify_for_segment(
            segment,
            evidence.previous_checkpoint,
            evidence.signature_verifier,
        )?;
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
    /// Validates self-contained structure but does not claim inventory completeness.
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

    pub fn validate_complete(
        &self,
        expected_scopes: &[IdentityScope],
    ) -> Result<(), StateContractError> {
        self.validate()?;
        self.storage.validate_complete(expected_scopes)
    }
}
