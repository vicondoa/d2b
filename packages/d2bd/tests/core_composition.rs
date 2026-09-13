#[test]
fn u9_component_contracts_keep_clipboard_and_notifications_on_typed_sessions() {
    let clipboard = d2b_provider_clipboard_wayland::clipboard_runner_contract();
    let notification = d2b_provider_notification_desktop::notification_runner_contract();
    assert!(clipboard.component_session_only());
    assert!(notification.component_session_only());
    assert_eq!(clipboard.repair_interval_secs(), 300);
    assert_eq!(notification.repair_interval_secs(), 300);
}

#[test]
fn u9_component_session_policy_binds_service_transport_and_generation() {
    let policy = d2bd::interaction_composition::interaction_endpoint_policy(
        d2b_provider_display_wayland::SERVICE_PACKAGE,
        9,
    )
    .expect("display ComponentSession policy");
    assert_eq!(policy.reconnect_generation, 9);
    assert_eq!(
        policy.service,
        d2b_contracts_zone_session::v3::component_session::ServicePackage::DisplayV3
    );
    assert_eq!(
        policy.transport_binding.transport,
        d2b_contracts_zone_session::v3::component_session::TransportClass::UnixSeqpacket
    );
    assert_eq!(
        policy.noise_profile,
        d2b_contracts_zone_session::v3::component_session::NoiseProfile::Nn25519ChaChaPolySha256
    );
    assert!(
        d2bd::interaction_composition::interaction_endpoint_policy("d2b.unknown.v3", 9)
            .is_none()
    );
}

