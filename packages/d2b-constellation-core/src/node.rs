//! Node summaries (ADR 0032). A node is a host, gateway, or
//! provider-managed execution environment within a realm.

use crate::capability::CapabilitySet;
use crate::ids::NodeId;
use crate::realm::RealmPath;
use serde::{Deserialize, Serialize};

/// What kind of node this is, and therefore what d2b can own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    /// A full d2b host (KVM, broker, vsock, device control).
    FullHost,
    /// A realm gateway guest.
    Gateway,
    /// A provider-managed, limited-capability node (no broker/KVM).
    ProviderManaged,
}

/// A node's advertised summary. Capabilities are positive assertions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeSummary {
    /// Stable node id.
    pub id: NodeId,
    /// Realm this node belongs to. Inventory is realm-scoped; this is not a
    /// host-global registry key.
    pub realm: RealmPath,
    /// Node kind.
    pub kind: NodeKind,
    /// Advertised capabilities.
    pub capabilities: CapabilitySet,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;
    use crate::ids::RealmId;

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    #[test]
    fn node_summary_carries_realm_and_capabilities() {
        let summary = NodeSummary {
            id: NodeId::parse("gateway").unwrap(),
            realm: realm("work"),
            kind: NodeKind::Gateway,
            capabilities: CapabilitySet::empty().with(Capability::Lifecycle),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"realm\":[\"work\"]"));
        let back: NodeSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.realm.target_form(), "work");
        assert!(back.capabilities.has(Capability::Lifecycle));
    }

    #[test]
    fn node_summary_rejects_unknown_fields() {
        let json = "{\"id\":\"gateway\",\"realm\":[\"work\"],\"kind\":\"gateway\",\
                    \"capabilities\":[],\"unexpected\":\"redacted\"}";
        assert!(serde_json::from_str::<NodeSummary>(json).is_err());
    }
}
