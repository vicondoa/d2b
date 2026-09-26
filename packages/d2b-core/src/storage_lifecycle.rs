//! Host-local storage lifecycle report DTOs.

use std::collections::BTreeSet;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Host-local daemon startup report for storage/restart/sync contract posture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StorageLifecycleReport {
    pub schema_version: String,
    pub storage_contract_present: bool,
    pub sync_contract_present: bool,
    pub path_count: usize,
    pub restart_policy_count: usize,
    pub lock_count: usize,
    pub issues: Vec<StorageLifecycleIssue>,
}

impl StorageLifecycleReport {
    pub fn is_degraded(&self) -> bool {
        !self.issues.is_empty()
    }

    pub fn issue_kinds_csv(&self) -> String {
        self.issues
            .iter()
            .map(StorageLifecycleIssue::kind_name)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn has_only_legacy_contract_issue(&self) -> bool {
        matches!(
            self.issues.as_slice(),
            [StorageLifecycleIssue::LegacyBundleContractsUnavailable { .. }]
        )
    }
}

// The published schema must name these fields the way `serde` writes them.
// schemars 0.8 reads `rename_all` on a variant, where it renames that
// variant's fields, but not the container-level `rename_all_fields`, so every
// struct variant repeats the field casing as a `schemars` attribute. Dropping
// one of them leaves the published schema requiring a key the daemon never
// writes; the schema-vs-bytes test in `tests/storage_lifecycle_schema.rs` is
// what makes that loss fail loudly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", rename_all_fields = "camelCase", tag = "kind")]
pub enum StorageLifecycleIssue {
    MissingStorageContract,
    MissingSyncContract,
    #[schemars(rename_all = "camelCase")]
    LegacyBundleContractsUnavailable {
        bundle_version: u32,
    },
    BundleResolverUnavailable,
    #[schemars(rename_all = "camelCase")]
    StorageContractInvalid {
        contract_id: String,
        reason: StorageContractValidationReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        offending_id: Option<String>,
    },
    #[schemars(rename_all = "camelCase")]
    SyncContractInvalid {
        contract_id: String,
        reason: SyncContractValidationReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        offending_id: Option<String>,
    },
    #[schemars(rename_all = "camelCase")]
    MissingRestartPolicy {
        vm: String,
        role_id: String,
    },
    #[schemars(rename_all = "camelCase")]
    AdoptableMissingCgroupLeaf {
        vm: String,
        role_id: String,
    },
}

impl StorageLifecycleIssue {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::MissingStorageContract => "missing-storage-contract",
            Self::MissingSyncContract => "missing-sync-contract",
            Self::LegacyBundleContractsUnavailable { .. } => "legacy-bundle-contracts-unavailable",
            Self::BundleResolverUnavailable => "bundle-resolver-unavailable",
            Self::StorageContractInvalid { .. } => "storage-contract-invalid",
            Self::SyncContractInvalid { .. } => "sync-contract-invalid",
            Self::MissingRestartPolicy { .. } => "missing-restart-policy",
            Self::AdoptableMissingCgroupLeaf { .. } => "adoptable-missing-cgroup-leaf",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageContractValidationReason {
    DuplicateStoragePathId,
    DuplicateRestartPolicy,
    DuplicateDegradedReason,
    Unclassified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SyncContractValidationReason {
    DuplicateLockId,
    OfdLockMissingCloexec,
    FdPassingMissingLeaseTransferRecord,
    DuplicateAcquireOrder,
    Unclassified,
}

/// Typed failure from [`StorageJson::validate_unique_ids`](crate::storage::StorageJson::validate_unique_ids).
///
/// Carries the offending id payload so a `StorageContractInvalid` issue can
/// be built without string-matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageValidationError {
    DuplicateStoragePathId {
        offending_id: String,
    },
    DuplicateRestartPolicy {
        offending_id: String,
    },
    DuplicateDegradedReason {
        offending_id: String,
    },
}

impl StorageValidationError {
    /// The wire reason for the persisted lifecycle report.
    pub fn reason(&self) -> StorageContractValidationReason {
        match self {
            Self::DuplicateStoragePathId { .. } => {
                StorageContractValidationReason::DuplicateStoragePathId
            }
            Self::DuplicateRestartPolicy { .. } => {
                StorageContractValidationReason::DuplicateRestartPolicy
            }
            Self::DuplicateDegradedReason { .. } => {
                StorageContractValidationReason::DuplicateDegradedReason
            }
        }
    }
}

impl fmt::Display for StorageValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateStoragePathId { offending_id } => {
                write!(f, "duplicate storage path id {offending_id}")
            }
            Self::DuplicateRestartPolicy { offending_id } => {
                write!(f, "duplicate restart policy for {offending_id}")
            }
            Self::DuplicateDegradedReason { offending_id } => {
                write!(f, "duplicate degraded reason {offending_id}")
            }
        }
    }
}

impl std::error::Error for StorageValidationError {}

/// Typed failure from [`SyncJson::validate_lock_order`](crate::sync::SyncJson::validate_lock_order).
///
/// Carries the offending id payload so a `SyncContractInvalid` issue can be
/// built without string-matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncValidationError {
    DuplicateLockId {
        offending_id: String,
    },
    OfdLockMissingCloexec {
        offending_id: String,
    },
    FdPassingMissingLeaseTransferRecord {
        offending_id: String,
    },
    DuplicateAcquireOrder {
        offending_id: String,
        existing_id: String,
    },
}

impl SyncValidationError {
    /// The wire reason for the persisted lifecycle report.
    pub fn reason(&self) -> SyncContractValidationReason {
        match self {
            Self::DuplicateLockId { .. } => SyncContractValidationReason::DuplicateLockId,
            Self::OfdLockMissingCloexec { .. } => {
                SyncContractValidationReason::OfdLockMissingCloexec
            }
            Self::FdPassingMissingLeaseTransferRecord { .. } => {
                SyncContractValidationReason::FdPassingMissingLeaseTransferRecord
            }
            Self::DuplicateAcquireOrder { .. } => {
                SyncContractValidationReason::DuplicateAcquireOrder
            }
        }
    }
}

impl fmt::Display for SyncValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateLockId { offending_id } => {
                write!(f, "duplicate lock id {offending_id}")
            }
            Self::OfdLockMissingCloexec { offending_id } => {
                write!(f, "OFD lock {offending_id} must require O_CLOEXEC")
            }
            Self::FdPassingMissingLeaseTransferRecord { offending_id } => write!(
                f,
                "fd-passing lock {offending_id} must require a lease transfer record"
            ),
            Self::DuplicateAcquireOrder {
                offending_id,
                existing_id,
            } => write!(
                f,
                "lock {offending_id} shares acquire order key with {existing_id}"
            ),
        }
    }
}

impl std::error::Error for SyncValidationError {}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn issue_variant_fields_serialize_with_schema_casing() {
        let legacy =
            serde_json::to_value(StorageLifecycleIssue::LegacyBundleContractsUnavailable {
                bundle_version: 5,
            })
            .expect("serialize legacy issue");
        assert_eq!(
            legacy,
            json!({
                "kind": "legacy-bundle-contracts-unavailable",
                "bundleVersion": 5
            })
        );

        let missing_restart = serde_json::to_value(StorageLifecycleIssue::MissingRestartPolicy {
            vm: "corp-vm".to_owned(),
            role_id: "cloud-hypervisor".to_owned(),
        })
        .expect("serialize missing restart issue");
        assert_eq!(
            missing_restart,
            json!({
                "kind": "missing-restart-policy",
                "vm": "corp-vm",
                "roleId": "cloud-hypervisor"
            })
        );

        let adoptable_missing_cgroup =
            serde_json::to_value(StorageLifecycleIssue::AdoptableMissingCgroupLeaf {
                vm: "corp-vm".to_owned(),
                role_id: "cloud-hypervisor".to_owned(),
            })
            .expect("serialize adoptable missing cgroup issue");
        assert_eq!(
            adoptable_missing_cgroup,
            json!({
                "kind": "adoptable-missing-cgroup-leaf",
                "vm": "corp-vm",
                "roleId": "cloud-hypervisor"
            })
        );

        let invalid_storage = serde_json::to_value(StorageLifecycleIssue::StorageContractInvalid {
            contract_id: "storage.json".to_owned(),
            reason: StorageContractValidationReason::DuplicateStoragePathId,
            offending_id: Some("path:run-root".to_owned()),
        })
        .expect("serialize invalid storage issue");
        assert_eq!(
            invalid_storage,
            json!({
                "kind": "storage-contract-invalid",
                "contractId": "storage.json",
                "reason": "duplicate-storage-path-id",
                "offendingId": "path:run-root"
            })
        );
    }

    #[test]
    fn report_accepts_future_top_level_fields() {
        let report = serde_json::from_value::<StorageLifecycleReport>(json!({
            "schemaVersion": "v2",
            "storageContractPresent": true,
            "syncContractPresent": true,
            "pathCount": 1,
            "restartPolicyCount": 1,
            "lockCount": 1,
            "issues": [],
            "futureField": "ignored"
        }))
        .expect("top-level report is forward-compatible");

        assert!(!report.is_degraded());
    }

    #[test]
    fn issue_kinds_are_deduped_and_stable() {
        let report = StorageLifecycleReport {
            schema_version: "v2".to_owned(),
            storage_contract_present: false,
            sync_contract_present: false,
            path_count: 0,
            restart_policy_count: 0,
            lock_count: 0,
            issues: vec![
                StorageLifecycleIssue::MissingRestartPolicy {
                    vm: "corp-vm".to_owned(),
                    role_id: "cloud-hypervisor".to_owned(),
                },
                StorageLifecycleIssue::AdoptableMissingCgroupLeaf {
                    vm: "another-vm".to_owned(),
                    role_id: "vhost-device-sound".to_owned(),
                },
                StorageLifecycleIssue::LegacyBundleContractsUnavailable { bundle_version: 5 },
                StorageLifecycleIssue::MissingRestartPolicy {
                    vm: "different-vm".to_owned(),
                    role_id: "swtpm".to_owned(),
                },
            ],
        };

        assert_eq!(
            report.issue_kinds_csv(),
            "adoptable-missing-cgroup-leaf,legacy-bundle-contracts-unavailable,missing-restart-policy"
        );
    }

    #[test]
    fn validation_errors_map_to_wire_reasons() {
        let storage_errors = [
            (
                StorageValidationError::DuplicateStoragePathId {
                    offending_id: "path:run-root".to_owned(),
                },
                StorageContractValidationReason::DuplicateStoragePathId,
            ),
            (
                StorageValidationError::DuplicateRestartPolicy {
                    offending_id: "corp-vm:cloud-hypervisor".to_owned(),
                },
                StorageContractValidationReason::DuplicateRestartPolicy,
            ),
            (
                StorageValidationError::DuplicateDegradedReason {
                    offending_id: "StorageDrift".to_owned(),
                },
                StorageContractValidationReason::DuplicateDegradedReason,
            ),
        ];
        for (error, reason) in storage_errors {
            assert_eq!(error.reason(), reason);
        }

        let sync_errors = [
            (
                SyncValidationError::DuplicateLockId {
                    offending_id: "lock:daemon".to_owned(),
                },
                SyncContractValidationReason::DuplicateLockId,
            ),
            (
                SyncValidationError::OfdLockMissingCloexec {
                    offending_id: "lock:daemon".to_owned(),
                },
                SyncContractValidationReason::OfdLockMissingCloexec,
            ),
            (
                SyncValidationError::FdPassingMissingLeaseTransferRecord {
                    offending_id: "lock:daemon".to_owned(),
                },
                SyncContractValidationReason::FdPassingMissingLeaseTransferRecord,
            ),
            (
                SyncValidationError::DuplicateAcquireOrder {
                    offending_id: "lock:second".to_owned(),
                    existing_id: "lock:first".to_owned(),
                },
                SyncContractValidationReason::DuplicateAcquireOrder,
            ),
        ];
        for (error, reason) in sync_errors {
            assert_eq!(error.reason(), reason);
        }

        assert_eq!(
            SyncValidationError::DuplicateAcquireOrder {
                offending_id: "lock:second".to_owned(),
                existing_id: "lock:first".to_owned(),
            }
            .to_string(),
            "lock lock:second shares acquire order key with lock:first"
        );
    }
}
