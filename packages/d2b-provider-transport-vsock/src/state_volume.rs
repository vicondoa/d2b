//! Canonical child-local service state volume declaration.

use std::fmt;

/// Empty state schema used by the transport service.
pub const EMPTY_STATE_SCHEMA: &str = "empty";
/// Canonical layout principal.
pub const STATE_LAYOUT_USER: &str = "User/d2b-transport-vsock";

/// Bounded state volume specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateVolumeSpec {
    /// Volume kind.
    pub kind: &'static str,
    /// Persistence policy.
    pub persistence_class: &'static str,
    /// Migration policy.
    pub migration_policy: &'static str,
    /// Sensitivity class.
    pub sensitivity_class: &'static str,
    /// Whether the broker owns the identity marker.
    pub broker_maintained_identity: bool,
}

impl Default for StateVolumeSpec {
    fn default() -> Self {
        Self {
            kind: "state",
            persistence_class: "persistent",
            migration_policy: "none",
            sensitivity_class: "private",
            broker_maintained_identity: true,
        }
    }
}

impl StateVolumeSpec {
    /// Validate the canonical empty state shape.
    pub fn validate(self) -> bool {
        self.kind == "state"
            && self.persistence_class == "persistent"
            && self.migration_policy == "none"
            && self.sensitivity_class == "private"
            && self.broker_maintained_identity
    }
}

impl fmt::Display for StateVolumeSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("transport-vsock-state-volume")
    }
}
