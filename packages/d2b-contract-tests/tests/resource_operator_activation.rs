//! Layer-1 contract coverage for the authenticated Wave 6 operator path.
//!
//! The daemon integration test exercises the real effects.  This policy
//! contract keeps the production composition fail-closed if the public
//! Resource API, dependency sequence, or removal/adoption boundary is later
//! reduced to a direct-controller-only test.

#[test]
fn wave6_operator_path_contains_public_api_and_provider_boundary() {
    let runtime = include_str!("../../d2bd/src/resource_runtime.rs");
    let boundary = include_str!("../../d2bd/src/resource_operator_activation.rs");
    let daemon = include_str!("../../d2bd/src/composition.rs");
    let integration = include_str!("../../d2bd/tests/resource_operator_activation.rs");
    let effects = include_str!("../../d2bd/tests/zone_provider_acceptance.rs");

    for required in [
        "bind_operator_resource_client",
        "reconcile_wave6_operator_acceptance",
        "select_wave6_resources",
        "Wave6ProviderBoundary",
        "ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>",
        "guest-provider-binding",
        "d2b_provider_runtime_cloud_hypervisor::PROVIDER_REF",
        "adopt_node",
        "NetworkReconciler::new",
        "ReconcileProgress::Ready",
        "ensure_guest_networks_reconciled",
        "ResourceGetFailed",
    ] {
        assert!(
            runtime.contains(required) || boundary.contains(required) || daemon.contains(required),
            "Wave 6 operator contract lost required production marker: {required}"
        );
    }
    for required in [
        "Wave6ResourceKind::Volume",
        "Wave6ResourceKind::Network",
        "Wave6ResourceKind::DeviceTpm",
        "Wave6ResourceKind::CloudHypervisorGuest",
        "adopt_after_restart",
        "remove_cloud_hypervisor_guest",
        "remove_network",
        "remove_device_tpm",
        "remove_volume",
    ] {
        assert!(
            boundary.contains(required) || runtime.contains(required),
            "Wave 6 operator contract lost lifecycle marker: {required}"
        );
    }
    for required in [
        "authenticated_operator_drives_wave6_resources_through_production_boundary",
        "Wave6RealBoundary",
        "assert!(report.adopted_after_restart)",
        "assert!(report.device_state_retained)",
    ] {
        assert!(
            integration.contains(required),
            "Wave 6 daemon acceptance lost required evidence marker: {required}"
        );
    }
    for required in [
        "VolumeLocalController",
        "NetworkReconciler",
        "TpmResourceController",
        "CloudHypervisorController",
        "pidfd_open",
        "current_identity",
    ] {
        assert!(
            effects.contains(required),
            "Wave 6 acceptance no longer reaches the real effect boundary: {required}"
        );
    }
    assert!(
        !integration.contains("Mock") && !integration.contains("mock_"),
        "operator acceptance must not regress to mock-only coverage"
    );
}
