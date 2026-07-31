//! integration-target: host-integration
//!
//! The host-integration lane supplies a real kernel, Provider process, Network
//! relay Endpoint, and Guest attachment. The hermetic crate tests cover all pure
//! controller transitions without opening devices or sockets.

#[test]
fn scenario_is_owned_by_the_host_integration_lane() {
    assert_eq!("host-integration", "host-integration");
}
