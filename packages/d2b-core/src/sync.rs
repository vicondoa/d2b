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
            if matches!(lock.kind, LockKind::Ofd | LockKind::FileRecord)
                && lock.resource_id.is_none()
            {
                return Err(format!(
                    "{:?} lock {} must be paired with a storage resourceId",
                    lock.kind, lock.id
                ));
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

    /// Deterministic global total-order rank for `lock_id`.
    ///
    /// The generated contract has no separate stored `global_order` field:
    /// [`validate_lock_order`] already enforces that every lock's
    /// `(scope_class, anchored_root, normalized_path, lock_id)` acquire-order
    /// key is unique across the whole `SyncJson`. Sorting the full lock set
    /// by that key yields a strict, reproducible total order; this method
    /// returns the 0-based position of `lock_id` in that order, or `None` if
    /// `lock_id` is absent. A runtime that only ever acquires locks in
    /// non-decreasing rank order (never a lower rank while holding a guard
    /// with an equal-or-greater rank) satisfies acquire-after ordering for
    /// every lock, which is a strict superset of any partial-order DAG:
    /// today's generated locks declare no cross-lock dependency, so the
    /// total order is the entire ordering contract there is to enforce.
    pub fn global_order_rank(&self, lock_id: &ContractId) -> Option<usize> {
        type OrderKey<'a> = (LockScopeClass, &'a str, &'a str, &'a str);
        let mut keyed: Vec<(OrderKey<'_>, &ContractId)> = self
            .locks
            .iter()
            .map(|lock| {
                (
                    (
                        lock.acquire_order.scope_class,
                        lock.acquire_order.anchored_root.as_str(),
                        lock.acquire_order.normalized_path.as_str(),
                        lock.acquire_order.lock_id.as_str(),
                    ),
                    &lock.id,
                )
            })
            .collect();
        keyed.sort_by(|left, right| left.0.cmp(&right.0));
        keyed.iter().position(|(_, id)| *id == lock_id)
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

    /// A schema-valid, fully-paired OFD lock baseline for `id`, so individual
    /// tests only need to override the single field they're exercising.
    fn valid_lock(id: &str) -> LockSpec {
        LockSpec {
            id: ContractId::parse(id).unwrap(),
            scope: ContractId::parse("host").unwrap(),
            path_template: Some(PathTemplate::parse(format!("/run/d2b/{id}.lock")).unwrap()),
            resource_id: Some(ContractId::parse(format!("path:{id}")).unwrap()),
            kind: LockKind::Ofd,
            owner_process: actor(ActorKind::Daemon, "d2bd"),
            allowed_holders: vec![actor(ActorKind::Daemon, "d2bd")],
            inheritance_policy: InheritancePolicy::CloseOnExec,
            fd_passing_policy: FdPassingPolicy {
                mechanism: FdPassingMechanism::None,
                lease_transfer_record_required: false,
            },
            acquire_order: order(id),
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
            release_authority: actor(ActorKind::Daemon, "d2bd"),
            cloexec_required: true,
        }
    }

    #[test]
    fn ofd_locks_require_cloexec() {
        let mut lock = valid_lock("lock:daemon");
        lock.cloexec_required = false;
        assert!(
            SyncJson {
                schema_version: "v2".to_owned(),
                locks: vec![lock],
            }
            .validate_lock_order()
            .is_err()
        );
    }

    #[test]
    fn ofd_locks_require_resource_id() {
        let mut lock = valid_lock("lock:daemon");
        lock.resource_id = None;
        let err = SyncJson {
            schema_version: "v2".to_owned(),
            locks: vec![lock],
        }
        .validate_lock_order()
        .unwrap_err();
        assert!(err.contains("resourceId"), "unexpected error: {err}");
    }

    #[test]
    fn valid_paired_lock_passes() {
        let lock = valid_lock("lock:daemon");
        assert!(
            SyncJson {
                schema_version: "v2".to_owned(),
                locks: vec![lock],
            }
            .validate_lock_order()
            .is_ok()
        );
    }

    #[test]
    fn global_order_rank_is_deterministic_and_unique() {
        // Deliberately inserted out of natural sort order to prove the rank
        // is derived from the acquire-order key, not from array position.
        let locks = vec![
            valid_lock("lock:zebra"),
            valid_lock("lock:apple"),
            valid_lock("lock:mango"),
        ];
        let doc = SyncJson {
            schema_version: "v2".to_owned(),
            locks,
        };
        assert!(doc.validate_lock_order().is_ok());
        let apple = doc.global_order_rank(&ContractId::parse("lock:apple").unwrap());
        let mango = doc.global_order_rank(&ContractId::parse("lock:mango").unwrap());
        let zebra = doc.global_order_rank(&ContractId::parse("lock:zebra").unwrap());
        assert_eq!(apple, Some(0));
        assert_eq!(mango, Some(1));
        assert_eq!(zebra, Some(2));
        assert_eq!(
            doc.global_order_rank(&ContractId::parse("lock:missing").unwrap()),
            None
        );
    }
}
