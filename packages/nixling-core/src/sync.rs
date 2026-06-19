use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contract_id::{ContractId, PathTemplate};
use crate::storage::{ActorRef, DegradeScope, DegradedReason};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncJson {
    pub schema_version: String,
    pub locks: Vec<LockSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockSpec {
    pub id: ContractId,
    pub scope: ContractId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_template: Option<PathTemplate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<ContractId>,
    pub kind: LockKind,
    pub owner_process: ActorRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_holders: Vec<ActorRef>,
    pub inheritance_policy: InheritancePolicy,
    pub fd_passing_policy: FdPassingPolicy,
    pub acquire_order: LockAcquireOrder,
    pub timeout_policy: LockTimeoutPolicy,
    pub stale_policy: LockStalePolicy,
    pub adoption_policy: LockAdoptionPolicy,
    pub degrade_scope: DegradeScope,
    pub release_authority: ActorRef,
    pub cloexec_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LockKind {
    Ofd,
    FileRecord,
    InProcess,
    KernelObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum InheritancePolicy {
    CloseOnExec,
    ExplicitFdMappingOnly,
    ScmRightsOnly,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FdPassingPolicy {
    pub mechanism: FdPassingMechanism,
    pub lease_transfer_record_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FdPassingMechanism {
    None,
    ScmRights,
    ExplicitFdMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockAcquireOrder {
    pub scope_class: LockScopeClass,
    pub anchored_root: ContractId,
    pub normalized_path: ContractId,
    pub lock_id: ContractId,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum LockScopeClass {
    Global,
    Host,
    Vm,
    Role,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockTimeoutPolicy {
    pub kind: LockTimeoutKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LockTimeoutKind {
    FailFast,
    BoundedWait,
    NoWait,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockStalePolicy {
    pub kind: LockStaleKind,
    pub degraded_reason: DegradedReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LockStaleKind {
    PidfdProofRequired,
    CgroupEmptyProofRequired,
    FileRecordOwnerMatch,
    CutoverOnly,
    ManualRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LockAdoptionPolicy {
    ReacquireAfterProof,
    TransferWithLeaseRecord,
    QuarantineOnAmbiguity,
    NotAdoptable,
}

impl SyncJson {
    pub fn validate_lock_order(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        let mut order_keys: BTreeMap<(&LockScopeClass, &str, &str, &str), &ContractId> =
            BTreeMap::new();
        for lock in &self.locks {
            if !ids.insert(lock.id.as_str()) {
                return Err(format!("duplicate lock id {}", lock.id));
            }
            if lock.kind == LockKind::Ofd && !lock.cloexec_required {
                return Err(format!("OFD lock {} must require O_CLOEXEC", lock.id));
            }
            if lock.fd_passing_policy.mechanism != FdPassingMechanism::None
                && !lock.fd_passing_policy.lease_transfer_record_required
            {
                return Err(format!(
                    "fd-passing lock {} must require a lease transfer record",
                    lock.id
                ));
            }
            let key = (
                &lock.acquire_order.scope_class,
                lock.acquire_order.anchored_root.as_str(),
                lock.acquire_order.normalized_path.as_str(),
                lock.acquire_order.lock_id.as_str(),
            );
            if let Some(existing) = order_keys.insert(key, &lock.id) {
                return Err(format!(
                    "lock {} shares acquire order key with {}",
                    lock.id, existing
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{ActorKind, ActorRef};

    fn actor(kind: ActorKind, value: &str) -> ActorRef {
        ActorRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        }
    }

    fn order(id: &str) -> LockAcquireOrder {
        LockAcquireOrder {
            scope_class: LockScopeClass::Global,
            anchored_root: ContractId::parse("run").unwrap(),
            normalized_path: ContractId::parse(id).unwrap(),
            lock_id: ContractId::parse(id).unwrap(),
        }
    }

    #[test]
    fn ofd_locks_require_cloexec() {
        let lock = LockSpec {
            id: ContractId::parse("lock:daemon").unwrap(),
            scope: ContractId::parse("host").unwrap(),
            path_template: Some(PathTemplate::parse("/run/nixling/daemon.lock").unwrap()),
            resource_id: None,
            kind: LockKind::Ofd,
            owner_process: actor(ActorKind::Daemon, "nixlingd"),
            allowed_holders: vec![actor(ActorKind::Daemon, "nixlingd")],
            inheritance_policy: InheritancePolicy::CloseOnExec,
            fd_passing_policy: FdPassingPolicy {
                mechanism: FdPassingMechanism::None,
                lease_transfer_record_required: false,
            },
            acquire_order: order("lock:daemon"),
            timeout_policy: LockTimeoutPolicy {
                kind: LockTimeoutKind::FailFast,
                timeout_ms: None,
            },
            stale_policy: LockStalePolicy {
                kind: LockStaleKind::PidfdProofRequired,
                degraded_reason: DegradedReason::LockOwnerAmbiguous,
            },
            adoption_policy: LockAdoptionPolicy::ReacquireAfterProof,
            degrade_scope: DegradeScope::Host,
            release_authority: actor(ActorKind::Daemon, "nixlingd"),
            cloexec_required: false,
        };
        assert!(
            SyncJson {
                schema_version: "v2".to_owned(),
                locks: vec![lock],
            }
            .validate_lock_order()
            .is_err()
        );
    }
}
