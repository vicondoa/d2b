//! Host-local storage lifecycle report DTOs.

use std::collections::BTreeSet;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum StorageLifecycleIssue {
    MissingStorageContract,
    MissingSyncContract,
    LegacyBundleContractsUnavailable {
        #[serde(rename = "bundleVersion")]
        bundle_version: u32,
    },
    BundleResolverUnavailable,
    StorageContractInvalid {
        reason: StorageContractValidationReason,
    },
    SyncContractInvalid {
        reason: SyncContractValidationReason,
    },
    MissingRestartPolicy {
        vm: String,
        #[serde(rename = "roleId")]
        role_id: String,
    },
    AdoptableMissingCgroupLeaf {
        vm: String,
        #[serde(rename = "roleId")]
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

pub fn classify_storage_validation_reason(detail: &str) -> StorageContractValidationReason {
    if detail.starts_with("duplicate storage path id ") {
        StorageContractValidationReason::DuplicateStoragePathId
    } else if detail.starts_with("duplicate restart policy for ") {
        StorageContractValidationReason::DuplicateRestartPolicy
    } else if detail.starts_with("duplicate degraded reason ") {
        StorageContractValidationReason::DuplicateDegradedReason
    } else {
        StorageContractValidationReason::Unclassified
    }
}

pub fn classify_sync_validation_reason(detail: &str) -> SyncContractValidationReason {
    if detail.starts_with("duplicate lock id ") {
        SyncContractValidationReason::DuplicateLockId
    } else if detail.starts_with("OFD lock ") && detail.ends_with(" must require O_CLOEXEC") {
        SyncContractValidationReason::OfdLockMissingCloexec
    } else if detail.starts_with("fd-passing lock ")
        && detail.ends_with(" must require a lease transfer record")
    {
        SyncContractValidationReason::FdPassingMissingLeaseTransferRecord
    } else if detail.starts_with("lock ") && detail.contains(" shares acquire order key with ") {
        SyncContractValidationReason::DuplicateAcquireOrder
    } else {
        SyncContractValidationReason::Unclassified
    }
}
