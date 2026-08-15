//! System-systemd Provider manifest facts.

use serde::Serialize;

/// Fixed manifest projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemdManifest {
    /// Artifact identity.
    pub artifact_id: &'static str,
    /// Common Process ResourceTypes.
    pub resource_types: [&'static str; 2],
    /// Controller component.
    pub component: &'static str,
    /// Provider state is status/ledger-derived.
    pub declares_state_volume: bool,
}

impl SystemdManifest {
    /// Return the canonical manifest.
    pub const fn canonical() -> Self {
        Self {
            artifact_id: "system-systemd",
            resource_types: ["Process", "EphemeralProcess"],
            component: "systemd-controller",
            declares_state_volume: false,
        }
    }
}
