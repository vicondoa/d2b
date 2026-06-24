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
        #[serde(rename = "contractId")]
        contract_id: String,
        reason: StorageContractValidationReason,
        #[serde(rename = "offendingId", skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        offending_id: Option<String>,
    },
    SyncContractInvalid {
        #[serde(rename = "contractId")]
        contract_id: String,
        reason: SyncContractValidationReason,
        #[serde(rename = "offendingId", skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        offending_id: Option<String>,
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

pub fn storage_validation_offending_id(detail: &str) -> Option<String> {
    if let Some(id) = detail.strip_prefix("duplicate storage path id ") {
        bounded_contract_detail(id)
    } else if let Some(id) = detail.strip_prefix("duplicate restart policy for ") {
        bounded_contract_detail(id)
    } else if let Some(id) = detail.strip_prefix("duplicate degraded reason ") {
        bounded_contract_detail(id)
    } else {
        None
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

pub fn sync_validation_offending_id(detail: &str) -> Option<String> {
    if let Some(id) = detail.strip_prefix("duplicate lock id ") {
        bounded_contract_detail(id)
    } else if let Some(rest) = detail.strip_prefix("OFD lock ") {
        rest.strip_suffix(" must require O_CLOEXEC")
            .and_then(bounded_contract_detail)
    } else if let Some(rest) = detail.strip_prefix("fd-passing lock ") {
        rest.strip_suffix(" must require a lease transfer record")
            .and_then(bounded_contract_detail)
    } else if let Some(rest) = detail.strip_prefix("lock ") {
        rest.split_once(" shares acquire order key with ")
            .and_then(|(id, _)| bounded_contract_detail(id))
    } else {
        None
    }
}

fn bounded_contract_detail(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.len() > 128
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.'))
    {
        return None;
    }
    Some(trimmed.to_owned())
}

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
}
