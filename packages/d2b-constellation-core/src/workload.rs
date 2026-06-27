//! Workload summaries and lifecycle state (ADR 0032). A workload is a VM,
//! provider session, or sandbox addressed by a stable id/alias.

use crate::capability::CapabilitySet;
use crate::ids::{NodeId, WorkloadId};
use crate::realm::RealmPath;
use serde::{Deserialize, Serialize};

/// Coarse workload lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadState {
    /// Declared/known but not running.
    Stopped,
    /// Allocation/start in progress.
    Starting,
    /// Running.
    Running,
    /// Stop in progress.
    Stopping,
    /// Terminal failure.
    Failed,
}

/// A selector for listing workloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadSelector {
    /// All workloads on the node.
    All,
    /// A single workload by id.
    One(WorkloadId),
}

/// A workload's advertised summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkloadSummary {
    /// Stable operator-facing alias/id.
    pub id: WorkloadId,
    /// Realm this workload belongs to. Inventory is realm-scoped; this does
    /// not imply a host-global workload registry.
    pub realm: RealmPath,
    /// Node that owns this workload.
    pub node: NodeId,
    /// Current state.
    pub state: WorkloadState,
    /// Capabilities this workload can present.
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
    fn workload_summary_carries_realm_node_and_capabilities() {
        let summary = WorkloadSummary {
            id: WorkloadId::parse("demo").unwrap(),
            realm: realm("work"),
            node: NodeId::parse("gateway").unwrap(),
            state: WorkloadState::Running,
            capabilities: CapabilitySet::empty().with(Capability::Exec),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"realm\":[\"work\"]"));
        let back: WorkloadSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.realm.target_form(), "work");
        assert_eq!(back.node.as_str(), "gateway");
        assert!(back.capabilities.has(Capability::Exec));
    }

    #[test]
    fn workload_summary_rejects_unknown_fields() {
        let json = "{\"id\":\"demo\",\"realm\":[\"work\"],\"node\":\"gateway\",\
                    \"state\":\"running\",\"capabilities\":[],\"unexpected\":\"redacted\"}";
        assert!(serde_json::from_str::<WorkloadSummary>(json).is_err());
    }
}
