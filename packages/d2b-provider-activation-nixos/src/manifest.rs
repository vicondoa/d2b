//! Activation-nixos Provider manifest facts.

use serde::Serialize;

/// Fixed manifest projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationManifest {
    /// Artifact identity.
    pub artifact_id: &'static str,
    /// Exported ResourceType.
    pub resource_type: &'static str,
    /// Controller component.
    pub controller: &'static str,
    /// Target-local runner component.
    pub runner: &'static str,
    /// Provider state is status/ledger-derived.
    pub declares_state_volume: bool,
}

impl ActivationManifest {
    /// Return the canonical manifest.
    pub const fn canonical() -> Self {
        Self {
            artifact_id: "activation-nixos",
            resource_type: d2b_contracts_zone_session::v3::NIXOS_GENERATION_RESOURCE_TYPE,
            controller: "activation-controller",
            runner: "activation-runner",
            declares_state_volume: false,
        }
    }
}
