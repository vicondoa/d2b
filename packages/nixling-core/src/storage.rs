use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contract_id::{ContractId, ContractText, PathTemplate};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageJson {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<StorageRoot>,
    pub paths: Vec<StoragePathSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restart_policies: Vec<ProcessRestartPolicy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_states: Vec<DegradedStateSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remediations: Vec<RemediationSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageRoot {
    pub id: ContractId,
    pub path: PathTemplate,
    pub class: StorageRootClass,
    pub owner: PrincipalRef,
    pub group: PrincipalRef,
    pub mode: String,
    pub authority: StorageAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageRootClass {
    Config,
    Persistent,
    Runtime,
    Cache,
    ExternalObserveOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageAuthority {
    NixModule,
    Daemon,
    Broker,
    Guest,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoragePathSpec {
    pub id: ContractId,
    pub scope: ContractId,
    pub path_template: PathTemplate,
    pub kind: StoragePathKind,
    pub lifecycle: StorageLifecycle,
    pub persistence: StoragePersistence,
    pub owner: PrincipalRef,
    pub group: PrincipalRef,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access_acl: Vec<AclGrant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_acl: Vec<AclGrant>,
    pub creator: ActorRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writers: Vec<ActorRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readers: Vec<ActorRef>,
    pub cleanup_policy: CleanupPolicy,
    pub repair_policy: RepairPolicy,
    pub restart_policy: StorageRestartPolicy,
    pub adoption_policy: StorageAdoptionPolicy,
    pub lease_class: LeaseClass,
    pub sensitivity: SensitivityClass,
    pub no_follow: bool,
    pub recursive: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariants: Vec<StorageInvariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StoragePathKind {
    Directory,
    RegularFile,
    UnixSocket,
    Symlink,
    DeviceNode,
    ExternalGrantOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageLifecycle {
    Config,
    Persistent,
    BootScopedReadoptable,
    BootScopedDisposable,
    ProcessScoped,
    ExternalObserveOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StoragePersistence {
    Persistent,
    BootScoped,
    ProcessScoped,
    Regenerable,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrincipalRef {
    pub kind: PrincipalKind,
    pub value: ContractId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PrincipalKind {
    User,
    Group,
    Uid,
    Gid,
    Role,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRef {
    pub kind: ActorKind,
    pub value: ContractId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ActorKind {
    NixModule,
    Daemon,
    Broker,
    Role,
    Guest,
    Operator,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AclGrant {
    pub principal: PrincipalRef,
    pub permissions: ContractId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupPolicy {
    Never,
    Boot,
    ProcessExitWithProof,
    VmStopWithProof,
    CutoverOnly,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RepairPolicy {
    None,
    NixActivation,
    BrokerReconcile,
    BrokerFailClosed,
    OperatorOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageRestartPolicy {
    PreserveAcrossDaemonRestart,
    RecreateAfterOwnerDeath,
    CleanupAfterOwnerDeath,
    ManualRecovery,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageAdoptionPolicy {
    AdoptWithLiveOwnerProof,
    RecreateFromPersistent,
    QuarantineOnAmbiguity,
    DeleteIfOwnerDead,
    NotAdoptable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LeaseClass {
    None,
    ProcessPidfd,
    CgroupLeaf,
    FileRecord,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SensitivityClass {
    Public,
    Private,
    SecretAdjacent,
    Audit,
    RealmScoped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageInvariant {
    NoSymlink,
    NoMagicLink,
    NoRecursiveMutation,
    SameFilesystem,
    HardlinkFarmNoRecursion,
    BrokerOpaqueIdOnly,
    RootOwnedParent,
    ScopeAuthorizationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessRestartPolicy {
    pub vm: ContractId,
    pub role_id: ContractId,
    pub restart_class: RestartClass,
    pub adoption_inputs: AdoptionInputs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub persistent_state_refs: Vec<ContractId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_state_refs: Vec<ContractId>,
    pub cleanup_before_restart: bool,
    pub degrade_on_failure: DegradedReason,
    pub degrade_scope: DegradeScope,
    pub readiness_after_adopt: ReadinessAfterAdopt,
    pub remediation_id: ContractId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RestartClass {
    Adoptable,
    Recreatable,
    StatefulQuarantine,
    NonResumable,
    ExternalObserved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdoptionInputs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cgroup_leaf: Option<ContractId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_checks: Vec<IdentityCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityCheck {
    CgroupMembership,
    ExecutablePath,
    ProfileId,
    CmdlineShape,
    StartTimeDiagnostic,
    PidfdOpenAfterCandidateRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DegradeScope {
    Role,
    Vm,
    Host,
    Realm,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DegradedReason {
    StorageDrift,
    StorageRepairFailed,
    AdoptionPending,
    AdoptionQuarantined,
    RestartRequired,
    LockOwnerAmbiguous,
    LockAcquireTimeout,
    ExternalDependencyUnhealthy,
    MigrationRequired,
    MigrationFailed,
    ViolationAuditThrottled,
    RoleComponentDegraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadinessAfterAdopt {
    pub kind: ReadinessAfterAdoptKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_ref: Option<ContractId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReadinessAfterAdoptKind {
    ExistingPredicate,
    UnixSocketListening,
    PidfdAlive,
    ExternalProbe,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DegradedStateSpec {
    pub reason: DegradedReason,
    pub scope: DegradeScope,
    pub storage_class: LedgerStorageClass,
    pub remediation_id: ContractId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LedgerStorageClass {
    TamperEvidentSegmented,
    AppendOnlyBounded,
    PlainBoundedDiagnostic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemediationSpec {
    pub id: ContractId,
    pub command: ContractText,
    pub description: ContractText,
}

impl StorageJson {
    pub fn validate_unique_ids(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for path in &self.paths {
            if !ids.insert(path.id.as_str()) {
                return Err(format!("duplicate storage path id {}", path.id));
            }
        }
        let mut restart_ids = BTreeSet::new();
        for restart in &self.restart_policies {
            let key = (restart.vm.as_str(), restart.role_id.as_str());
            if !restart_ids.insert(key) {
                return Err(format!(
                    "duplicate restart policy for {}:{}",
                    restart.vm, restart.role_id
                ));
            }
        }
        let mut reasons = BTreeSet::new();
        for state in &self.degraded_states {
            if !reasons.insert(state.reason) {
                return Err(format!("duplicate degraded reason {:?}", state.reason));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(kind: ActorKind, value: &str) -> ActorRef {
        ActorRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        }
    }

    fn principal(kind: PrincipalKind, value: &str) -> PrincipalRef {
        PrincipalRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        }
    }

    #[test]
    fn storage_json_rejects_duplicate_ids() {
        let path = StoragePathSpec {
            id: ContractId::parse("path:one").unwrap(),
            scope: ContractId::parse("host").unwrap(),
            path_template: PathTemplate::parse("/run/nixling").unwrap(),
            kind: StoragePathKind::Directory,
            lifecycle: StorageLifecycle::BootScopedReadoptable,
            persistence: StoragePersistence::BootScoped,
            owner: principal(PrincipalKind::User, "nixlingd"),
            group: principal(PrincipalKind::Group, "nixlingd"),
            mode: "0750".to_owned(),
            access_acl: Vec::new(),
            default_acl: Vec::new(),
            creator: actor(ActorKind::NixModule, "tmpfiles"),
            writers: vec![actor(ActorKind::Daemon, "nixlingd")],
            readers: vec![actor(ActorKind::Daemon, "nixlingd")],
            cleanup_policy: CleanupPolicy::Boot,
            repair_policy: RepairPolicy::BrokerReconcile,
            restart_policy: StorageRestartPolicy::PreserveAcrossDaemonRestart,
            adoption_policy: StorageAdoptionPolicy::AdoptWithLiveOwnerProof,
            lease_class: LeaseClass::None,
            sensitivity: SensitivityClass::Private,
            no_follow: true,
            recursive: false,
            invariants: vec![StorageInvariant::NoSymlink],
        };
        let contract = StorageJson {
            schema_version: "v2".to_owned(),
            roots: Vec::new(),
            paths: vec![path.clone(), path],
            restart_policies: Vec::new(),
            degraded_states: Vec::new(),
            remediations: Vec::new(),
        };
        assert!(contract.validate_unique_ids().is_err());
    }
}
