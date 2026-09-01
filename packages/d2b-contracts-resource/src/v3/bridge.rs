//! Provider-neutral display and clipboard bridge contracts.
//!
//! A bridge frame carries an immutable Zone/resource identity and bounded
//! transfer metadata. It never carries a path, a legacy target, or a
//! caller-selected authority.

pub use d2b_contracts::workload::WorkloadProviderKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{execution_policy::BoundedText, identity::ZoneResourceIdentity};

/// Attribution quality shared by display and clipboard bridges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BridgeAttributionQuality {
    /// The bridge authenticated the exact producing client.
    ExactClient,
    /// The bridge inferred the producer from the focused window.
    FocusedWindowGuess,
    /// The bridge used a stale focused-window observation.
    CacheStaleFocusedWindowGuess,
    /// Trusted broker diagnostics injected the attribution.
    BrokerInjectedDebug,
}

/// Provider-neutral endpoint identity for a display or clipboard bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeEndpointIdentity {
    /// Immutable Zone/resource identity.
    pub resource: ZoneResourceIdentity,
    /// Runtime/provider family associated with the endpoint.
    pub provider_kind: WorkloadProviderKind,
}

impl BridgeEndpointIdentity {
    /// Construct an endpoint identity from its immutable resource fence.
    pub const fn new(resource: ZoneResourceIdentity, provider_kind: WorkloadProviderKind) -> Self {
        Self {
            resource,
            provider_kind,
        }
    }
}

/// One bounded bridge transfer frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BridgeFrame {
    /// Request that a destination consume one clipboard selection.
    PasteRequest {
        endpoint: BridgeEndpointIdentity,
        mime_type: BoundedText,
        source_id: u64,
        source_attribution: BridgeAttributionQuality,
    },
    /// Publish one clipboard selection to a destination.
    CopySelection {
        endpoint: BridgeEndpointIdentity,
        mime_type: BoundedText,
        source_id: u64,
        source_attribution: BridgeAttributionQuality,
    },
}
