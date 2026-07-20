use d2b_contracts::v2_component_session::ServicePackage;
use d2bd::daemon_service::{daemon_channel_binding, daemon_endpoint_policy};

#[test]
fn public_status_uses_the_fixed_daemon_component_service() {
    let policy = daemon_endpoint_policy(44, daemon_channel_binding(1000, 100))
        .expect("daemon endpoint policy");
    assert_eq!(policy.service, ServicePackage::DaemonV2);
    assert_eq!(policy.attachment_policy.max_per_packet, 0);
    assert!(!policy.attachment_policy.credentials_allowed);
}

#[test]
fn legacy_json_status_request_is_not_dispatched() {
    let source = include_str!("../src/lib.rs");
    let daemon_service = include_str!("../src/control_services/daemon.rs");
    assert!(!source.contains("fn handle_connection_authorized"));
    assert!(!source.contains("serde_json::from_slice::<wire::Request>"));
    assert!(!daemon_service.contains("crate::dispatch_request"));
}
