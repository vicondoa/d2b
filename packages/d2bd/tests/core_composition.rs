use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ZoneId, identity::ReconnectGeneration,
};
use d2bd::resource_runtime::{U6_SHARED_PROVIDER_RUNNERS, compose_shared_guest_runner_descriptors};

#[test]
fn guest_composition_builds_one_filtered_runner_per_runtime_provider() {
    let generations = U6_SHARED_PROVIDER_RUNNERS
        .iter()
        .map(|registration| {
            (
                ResourceRef::parse(registration.provider_ref).unwrap(),
                ResourceGeneration::new(7).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let descriptors = compose_shared_guest_runner_descriptors(
        U6_SHARED_PROVIDER_RUNNERS,
        ZoneId::parse("work").unwrap(),
        ControllerGeneration::new(3).unwrap(),
        &generations,
        ReconnectGeneration::new(5).unwrap(),
    )
    .expect("U6 descriptors");

    assert_eq!(descriptors.len(), 4);
    let controllers = descriptors
        .iter()
        .map(|(_, descriptor)| descriptor.identity().controller_ref().clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(controllers.len(), 4);
    for (registration, descriptor) in descriptors {
        assert_eq!(descriptor.resource_types().next().unwrap().as_str(), "Guest");
        assert_eq!(
            descriptor
                .watch_selectors()
                .iter()
                .find(|selector| {
                    selector.field() == d2b_core_controller::SelectorField::Spec
                })
                .and_then(|selector| selector.exact_value()),
            Some(registration.provider_ref)
        );
        assert!(
            descriptor
                .dependency_selectors()
                .iter()
                .any(|selector| selector.resource_type().as_str() == "Process")
        );
        assert_eq!(
            descriptor.execution().resync().observe_interval_ticks(),
            Some(registration.repair_interval_ticks)
        );
        assert_eq!(
            descriptor.execution().resync().resync_interval_ticks(),
            registration.repair_interval_ticks
        );
    }
}

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

