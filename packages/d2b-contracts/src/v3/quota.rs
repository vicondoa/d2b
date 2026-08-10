//! Zone-wide Quota ResourceType contract.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{ResourceTypeName, execution_policy::redacted_debug};

/// Canonical Quota ResourceType name.
pub const QUOTA_RESOURCE_TYPE: &str = "Quota";
/// Maximum per-type quota entries.
pub const MAX_QUOTA_PER_TYPE_ENTRIES: usize = 64;
/// Maximum resources in one Zone quota.
pub const MAX_QUOTA_RESOURCES: u64 = 65_536;
/// Maximum owner depth admitted by a quota.
pub const MAX_QUOTA_OWNER_DEPTH: u32 = 32;
/// Core finalizer used while quota dependents drain.
pub const QUOTA_DRAIN_FINALIZER: &str = "core.quota-drain";

/// Hard or warning-only quota enforcement.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum QuotaEnforcementPolicy {
    Hard,
    Soft,
}

/// The only v3 quota scope.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum QuotaScope {
    Zone,
}

/// Quota contract failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaContractError {
    ZeroOrOverLimit,
    PerTypeBoundExceeded,
    DuplicateResourceType,
    InvalidResourceType,
    InvalidScope,
}

impl core::fmt::Display for QuotaContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::ZeroOrOverLimit => "quota-ceiling-invalid",
            Self::PerTypeBoundExceeded => "quota-per-type-bound-exceeded",
            Self::DuplicateResourceType => "quota-duplicate-resource-type",
            Self::InvalidResourceType => "quota-resource-type-invalid",
            Self::InvalidScope => "quota-scope-invalid",
        })
    }
}

impl std::error::Error for QuotaContractError {}

/// Aggregate quota ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaCeilings {
    max_resources: u64,
    max_resources_per_type: u64,
    max_owner_depth: u32,
    max_cpu: Option<u64>,
    max_memory_mib: Option<u64>,
    max_storage_gib: Option<u64>,
}

impl QuotaCeilings {
    /// Construct bounded ceilings.
    pub fn new(
        max_resources: u64,
        max_resources_per_type: u64,
        max_owner_depth: u32,
        max_cpu: Option<u64>,
        max_memory_mib: Option<u64>,
        max_storage_gib: Option<u64>,
    ) -> Result<Self, QuotaContractError> {
        if max_resources == 0
            || max_resources > MAX_QUOTA_RESOURCES
            || max_resources_per_type == 0
            || max_resources_per_type > MAX_QUOTA_RESOURCES
            || max_owner_depth == 0
            || max_owner_depth > MAX_QUOTA_OWNER_DEPTH
            || max_cpu.is_some_and(|value| value == 0)
            || max_memory_mib.is_some_and(|value| value == 0)
            || max_storage_gib.is_some_and(|value| value == 0)
        {
            return Err(QuotaContractError::ZeroOrOverLimit);
        }
        Ok(Self {
            max_resources,
            max_resources_per_type,
            max_owner_depth,
            max_cpu,
            max_memory_mib,
            max_storage_gib,
        })
    }

    /// Defaults from the normative resource contract.
    pub const fn default_values() -> Self {
        Self {
            max_resources: 4096,
            max_resources_per_type: 512,
            max_owner_depth: 8,
            max_cpu: None,
            max_memory_mib: None,
            max_storage_gib: None,
        }
    }

    /// Return the total resource ceiling.
    pub const fn max_resources(self) -> u64 {
        self.max_resources
    }

    /// Return the per-type resource ceiling.
    pub const fn max_resources_per_type(self) -> u64 {
        self.max_resources_per_type
    }

    /// Return the owner-chain ceiling.
    pub const fn max_owner_depth(self) -> u32 {
        self.max_owner_depth
    }

    /// Return optional CPU ceiling.
    pub const fn max_cpu(self) -> Option<u64> {
        self.max_cpu
    }

    /// Return optional memory ceiling.
    pub const fn max_memory_mib(self) -> Option<u64> {
        self.max_memory_mib
    }

    /// Return optional storage ceiling.
    pub const fn max_storage_gib(self) -> Option<u64> {
        self.max_storage_gib
    }
}

impl Default for QuotaCeilings {
    fn default() -> Self {
        Self::default_values()
    }
}

impl<'de> Deserialize<'de> for QuotaCeilings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default = "default_resources")]
            max_resources: u64,
            #[serde(default = "default_per_type")]
            max_resources_per_type: u64,
            #[serde(default = "default_owner_depth")]
            max_owner_depth: u32,
            #[serde(default)]
            max_cpu: Option<u64>,
            #[serde(default)]
            max_memory_mib: Option<u64>,
            #[serde(default)]
            max_storage_gib: Option<u64>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.max_resources,
            wire.max_resources_per_type,
            wire.max_owner_depth,
            wire.max_cpu,
            wire.max_memory_mib,
            wire.max_storage_gib,
        )
        .map_err(serde::de::Error::custom)
    }
}

const fn default_resources() -> u64 {
    4096
}
const fn default_per_type() -> u64 {
    512
}
const fn default_owner_depth() -> u32 {
    8
}

/// Per-ResourceType quota ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaTypeCeiling {
    max_resources: Option<u64>,
    max_cpu: Option<u64>,
    max_memory_mib: Option<u64>,
    max_storage_gib: Option<u64>,
}

impl QuotaTypeCeiling {
    /// Construct an optional per-type ceiling; present values are positive.
    pub fn new(
        max_resources: Option<u64>,
        max_cpu: Option<u64>,
        max_memory_mib: Option<u64>,
        max_storage_gib: Option<u64>,
    ) -> Result<Self, QuotaContractError> {
        if max_resources.is_some_and(|value| value == 0 || value > MAX_QUOTA_RESOURCES)
            || max_cpu.is_some_and(|value| value == 0)
            || max_memory_mib.is_some_and(|value| value == 0)
            || max_storage_gib.is_some_and(|value| value == 0)
        {
            return Err(QuotaContractError::ZeroOrOverLimit);
        }
        Ok(Self {
            max_resources,
            max_cpu,
            max_memory_mib,
            max_storage_gib,
        })
    }

    /// Return the resource count ceiling.
    pub const fn max_resources(self) -> Option<u64> {
        self.max_resources
    }

    /// Return the optional per-type CPU ceiling.
    pub const fn max_cpu(self) -> Option<u64> {
        self.max_cpu
    }

    /// Return the optional per-type memory ceiling.
    pub const fn max_memory_mib(self) -> Option<u64> {
        self.max_memory_mib
    }

    /// Return the optional per-type storage ceiling.
    pub const fn max_storage_gib(self) -> Option<u64> {
        self.max_storage_gib
    }
}

impl<'de> Deserialize<'de> for QuotaTypeCeiling {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            max_resources: Option<u64>,
            #[serde(default)]
            max_cpu: Option<u64>,
            #[serde(default)]
            max_memory_mib: Option<u64>,
            #[serde(default)]
            max_storage_gib: Option<u64>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.max_resources,
            wire.max_cpu,
            wire.max_memory_mib,
            wire.max_storage_gib,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Complete Quota desired state.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSpec {
    ceilings: QuotaCeilings,
    per_type_ceilings: BTreeMap<ResourceTypeName, QuotaTypeCeiling>,
    scope: QuotaScope,
    enforcement_policy: QuotaEnforcementPolicy,
}

impl QuotaSpec {
    /// Construct and validate a Quota spec.
    pub fn new(
        ceilings: QuotaCeilings,
        per_type_ceilings: BTreeMap<ResourceTypeName, QuotaTypeCeiling>,
        scope: QuotaScope,
        enforcement_policy: QuotaEnforcementPolicy,
    ) -> Result<Self, QuotaContractError> {
        if per_type_ceilings.len() > MAX_QUOTA_PER_TYPE_ENTRIES {
            return Err(QuotaContractError::PerTypeBoundExceeded);
        }
        Ok(Self {
            ceilings,
            per_type_ceilings,
            scope,
            enforcement_policy,
        })
    }

    /// Borrow global ceilings.
    pub const fn ceilings(&self) -> QuotaCeilings {
        self.ceilings
    }

    /// Borrow per-type ceilings.
    pub fn per_type_ceilings(&self) -> &BTreeMap<ResourceTypeName, QuotaTypeCeiling> {
        &self.per_type_ceilings
    }

    /// Return scope.
    pub const fn scope(&self) -> QuotaScope {
        self.scope
    }

    /// Return enforcement policy.
    pub const fn enforcement_policy(&self) -> QuotaEnforcementPolicy {
        self.enforcement_policy
    }
}

redacted_debug!(QuotaSpec);

impl<'de> Deserialize<'de> for QuotaSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            ceilings: QuotaCeilings,
            #[serde(default)]
            per_type_ceilings: BTreeMap<ResourceTypeName, QuotaTypeCeiling>,
            #[serde(default = "default_scope")]
            scope: QuotaScope,
            #[serde(default = "default_enforcement")]
            enforcement_policy: QuotaEnforcementPolicy,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.ceilings,
            wire.per_type_ceilings,
            wire.scope,
            wire.enforcement_policy,
        )
        .map_err(serde::de::Error::custom)
    }
}

const fn default_scope() -> QuotaScope {
    QuotaScope::Zone
}
const fn default_enforcement() -> QuotaEnforcementPolicy {
    QuotaEnforcementPolicy::Hard
}

/// Closed Quota condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum QuotaConditionType {
    CeilingsValid,
    OverQuota,
    QuotaDrainPending,
}

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
    last_checked_at: Option<super::Timestamp>,
    dependent_count: u32,
}

impl QuotaStatusResource {
    /// Construct a bounded Quota status projection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        used_resources: u32,
        used_cpu: Option<u32>,
        used_memory_mib: Option<u32>,
        used_storage_gib: Option<u32>,
        over_quota: bool,
        mut over_quota_types: Vec<ResourceTypeName>,
        last_checked_at: Option<super::Timestamp>,
        dependent_count: u32,
    ) -> Result<Self, QuotaContractError> {
        if over_quota_types.len() > 16 {
            return Err(QuotaContractError::PerTypeBoundExceeded);
        }
        over_quota_types.sort();
        if over_quota_types.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(QuotaContractError::DuplicateResourceType);
        }
        Ok(Self {
            used_resources,
            used_cpu,
            used_memory_mib,
            used_storage_gib,
            over_quota,
            over_quota_types,
            last_checked_at,
            dependent_count,
        })
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_quota_defaults_and_bounds_are_exact() {
        let spec: QuotaSpec = serde_json::from_slice(br#"{}"#).unwrap();
        assert_eq!(spec.ceilings().max_resources(), 4096);
        assert_eq!(spec.enforcement_policy(), QuotaEnforcementPolicy::Hard);
        assert!(QuotaCeilings::new(0, 1, 1, None, None, None).is_err());
        assert!(QuotaCeilings::new(1, 1, 33, None, None, None).is_err());
    }

    #[test]
    fn per_type_entries_are_bounded() {
        let mut entries = BTreeMap::new();
        for index in 0..65 {
            entries.insert(
                ResourceTypeName::parse(format!("p{index}.d2bus.org.Type")).unwrap(),
                QuotaTypeCeiling::new(Some(1), None, None, None).unwrap(),
            );
        }
        assert_eq!(
            QuotaSpec::new(
                QuotaCeilings::default(),
                entries,
                QuotaScope::Zone,
                QuotaEnforcementPolicy::Hard,
            )
            .unwrap_err(),
            QuotaContractError::PerTypeBoundExceeded
        );
    }
}
