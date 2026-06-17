//! Node summaries (ADR 0032). A node is a host, gateway, or
//! provider-managed execution environment within a realm.

use crate::capability::CapabilitySet;
use crate::ids::NodeId;
use serde::{Deserialize, Serialize};

/// What kind of node this is, and therefore what nixling can own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    /// A full nixling host (KVM, broker, vsock, device control).
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
    /// Node kind.
    pub kind: NodeKind,
    /// Advertised capabilities.
    pub capabilities: CapabilitySet,
}
