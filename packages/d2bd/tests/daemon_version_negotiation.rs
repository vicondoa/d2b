use d2b_contracts::v2_component_session::{
    COMPONENT_SESSION_MAJOR, COMPONENT_SESSION_MINOR, PREFACE_MAGIC, ServicePackage,
};
use d2bd::daemon_service::{daemon_channel_binding, daemon_endpoint_policy};

#[test]
fn daemon_protocol_is_component_session_v2_without_semver_negotiation() {
    assert_eq!(PREFACE_MAGIC, *b"D2BCS2\r\n");
    assert_eq!(COMPONENT_SESSION_MAJOR, 2);
    assert_eq!(COMPONENT_SESSION_MINOR, 0);
    let policy =
        daemon_endpoint_policy(1, daemon_channel_binding(1000, 100)).expect("fixed daemon policy");
    assert_eq!(policy.service, ServicePackage::DaemonV2);
}

#[test]
fn old_public_hello_is_absent_from_daemon_wire_source() {
    let source = include_str!("../src/wire.rs");
    assert!(!source.contains("parse_hello"));
    assert!(!source.contains("hello_ok"));
    assert!(!source.contains("negotiate_version"));
}
