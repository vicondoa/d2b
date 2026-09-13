//! Fixed system-minijail Provider manifest projection.

use serde::Serialize;

/// Bootstrap Provider manifest facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinijailManifest {
    /// Artifact identity.
    pub artifact_id: &'static str,
    /// Fixed controller component.
    pub component: &'static str,
    /// Process ResourceTypes owned.
    pub resource_types: [&'static str; 2],
    /// No Provider-owned state Volume.
    pub declares_state_volume: bool,
    /// Mandatory Linux kernel floor.
    pub minimum_kernel: (u16, u16),
}

impl MinijailManifest {
    /// Return the canonical manifest.
    pub const fn canonical() -> Self {
        Self {
            artifact_id: "system-minijail",
            component: "minijail-controller",
            resource_types: ["Process", "EphemeralProcess"],
            declares_state_volume: false,
            minimum_kernel: (5, 14),
        }
    }
}
