//! Zone-wide Quota ResourceType contract.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use d2b_contracts_resource::v3::ResourceTypeName;

/// ResourceType-common Quota status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaStatusResource {
    used_resources: u32,
    used_cpu: Option<u32>,
    used_memory_mib: Option<u32>,
    used_storage_gib: Option<u32>,
    over_quota: bool,
    over_quota_types: Vec<ResourceTypeName>,
    last_checked_at: Option<d2b_contracts_resource::v3::Timestamp>,
    dependent_count: u32,
}

impl QuotaStatusResource {
    /// Return resources currently counted.
    pub const fn used_resources(&self) -> u32 {
        self.used_resources
    }

    /// Return dependent resource count.
    pub const fn dependent_count(&self) -> u32 {
        self.dependent_count
    }

    /// Whether a soft quota is currently exceeded.
    pub const fn over_quota(&self) -> bool {
        self.over_quota
    }
}

/// Alias used by generic status adapters.
pub type QuotaStatus = QuotaStatusResource;
