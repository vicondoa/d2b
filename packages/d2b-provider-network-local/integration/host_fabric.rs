//! integration-target: container
//!
//! Host-fabric lifecycle scenario contract.
//!
//! The executable scenario is enabled when the core effect adapter and the
//! closed bridge, persistent-TAP deletion, and ownership-projection broker
//! operations have production handlers. Until then, hermetic tests in this
//! crate prove the projection preservation, marker rejection, bridge-port
//! readback, route readiness, and sysctl ordering used by that adapter. This
//! file intentionally names no alternate host mutation path.

/// Ordered fixture obligations for the provider-system host-fabric lane.
pub const HOST_FABRIC_SCENARIOS: &[&str] = &[
    "bridge-isolation-default",
    "east-west-opt-in",
    "nftables-owned-projection-drift",
    "persistent-tap-create-delete",
    "macvtap-create-delete",
];

#[test]
fn host_fabric_fixture_keeps_every_required_lifecycle_scenario() {
    assert_eq!(
        HOST_FABRIC_SCENARIOS,
        [
            "bridge-isolation-default",
            "east-west-opt-in",
            "nftables-owned-projection-drift",
            "persistent-tap-create-delete",
            "macvtap-create-delete",
        ]
    );
}
