use d2b_provider_transport_vsock::{EMPTY_STATE_SCHEMA, STATE_LAYOUT_USER, StateVolumeSpec};

#[test]
fn state_volume_spec_matches_canonical_schema() {
    assert_eq!(EMPTY_STATE_SCHEMA, "empty");
    assert!(StateVolumeSpec::default().validate());
}

#[test]
fn state_volume_uses_user_layout_not_component_principal() {
    assert_eq!(STATE_LAYOUT_USER, "User/d2b-transport-vsock");
    assert!(!STATE_LAYOUT_USER.contains("ComponentPrincipal"));
}
