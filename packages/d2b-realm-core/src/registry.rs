//! Provider and workload registry summaries for realm-native placement.

use crate::capability::CapabilitySet;
use crate::ids::{NodeId, ProviderId, WorkloadId};
use crate::realm::{RealmControllerPlacement, RealmPath};
use crate::token::ProtocolToken;
use serde::{Deserialize, Serialize};

/// Coarse workload placement kind for inventory and preflight output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkloadPlacement {
    /// Workload runs on a host-local runtime provider.
    HostLocal {
        /// Node that currently owns the workload.
        node: NodeId,
    },
    /// Workload runs inside a gateway VM realm.
    GatewayVm {
        /// Gateway node that currently owns the workload.
        gateway: NodeId,
    },
    /// Workload runs on a cloud full-host realm.
    CloudFullHost {
        /// Cloud node id.
        node: NodeId,
        /// Non-secret provider region/class token.
        region: Option<ProtocolToken>,
    },
    /// Provider-managed sandbox/session.
    ProviderManaged {
        /// Provider that owns the sandbox/session.
        provider: ProviderId,
        /// Non-secret provider placement class.
        placement: ProtocolToken,
    },
}

/// Provider registry entry owned by a realm controller. Capabilities are
/// positive assertions; absent capabilities must be treated as denial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRegistryEntry {
    /// Provider id.
    pub provider: ProviderId,
    /// Realm that owns this registry row.
    pub realm: RealmPath,
    /// Controller placement through which the provider is managed.
    pub controller_placement: RealmControllerPlacement,
    /// Provider implementation/class token.
    pub provider_kind: ProtocolToken,
    /// Positive provider capability advertisement.
    pub capabilities: CapabilitySet,
}

/// Workload placement summary for machine-readable inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadPlacementSummary {
    /// Workload id within the realm.
    pub workload: WorkloadId,
    /// Owning realm.
    pub realm: RealmPath,
    /// Resolved placement metadata.
    pub placement: WorkloadPlacement,
    /// Provider registry row, when provider-backed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub provider: Option<ProviderId>,
    /// Positive workload capability advertisement.
    pub capabilities: CapabilitySet,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;
    use crate::ids::RealmId;
    use schemars::schema_for;

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    #[test]
    fn provider_entry_advertises_positive_capabilities() {
        let entry = ProviderRegistryEntry {
            provider: ProviderId::parse("aca").unwrap(),
            realm: realm("work"),
            controller_placement: RealmControllerPlacement::ProviderController {
                provider: ProviderId::parse("aca").unwrap(),
            },
            provider_kind: ProtocolToken::parse("aca-v1").unwrap(),
            capabilities: CapabilitySet::empty().with(Capability::Exec),
        };
        assert!(entry.capabilities.has(Capability::Exec));
        assert!(!entry.capabilities.has(Capability::WindowForwarding));
        let json = serde_json::to_string(&entry).unwrap();
        let back: ProviderRegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider.as_str(), "aca");
    }

    #[test]
    fn workload_placement_rejects_unknown_fields() {
        let json = "{\"workload\":\"api\",\"realm\":[\"work\"],\
            \"placement\":{\"kind\":\"provider-managed\",\"provider\":\"aca\",\
            \"placement\":\"sandbox\",\"extra\":true},\"provider\":\"aca\",\
            \"capabilities\":[\"exec\"]}";
        assert!(serde_json::from_str::<WorkloadPlacementSummary>(json).is_err());
    }

    #[test]
    fn registry_schemas_are_generated() {
        let provider_schema = schema_for!(ProviderRegistryEntry);
        let workload_schema = schema_for!(WorkloadPlacementSummary);
        assert!(provider_schema.schema.metadata.is_some());
        assert!(workload_schema.schema.metadata.is_some());
    }
}
