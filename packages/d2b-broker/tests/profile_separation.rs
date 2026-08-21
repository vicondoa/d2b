use d2b_broker::runtime::{BrokerMode, parse_command};

#[path = "common/mod.rs"]
#[cfg(not(feature = "layer1-bootstrap"))]
mod common;

#[cfg(not(feature = "layer1-bootstrap"))]
use std::os::fd::AsRawFd;

#[cfg(not(feature = "layer1-bootstrap"))]
use common::{D2BD_UID, TestBroker};
#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_broker::protocol::{connect_seqpacket, recv_json_frame, send_json_frame};
#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, BrokerResponse, HelloRequest,
};

fn parse(args: &[&str]) -> BrokerMode {
    parse_command(args.iter().map(|arg| (*arg).to_owned())).expect("profile command should parse")
}

#[test]
fn profile_is_selected_at_process_start() {
    let host = parse(&["host", "--test-mode"]);
    let guest = parse(&["guest", "--test-mode"]);

    let (host_config, guest_config) = match (host, guest) {
        (BrokerMode::Host(host), BrokerMode::Guest(guest)) => (host, guest),
        other => panic!("unexpected broker modes: {other:?}"),
    };

    assert_eq!(host_config.profile.as_str(), "host");
    assert_eq!(guest_config.profile.as_str(), "guest");
    assert_ne!(host_config.profile, guest_config.profile);
}

#[test]
fn each_profile_requires_its_own_instance_bindings() {
    let host = parse(&[
        "host",
        "--test-mode",
        "--authority-id",
        "host-authority",
        "--socket-path",
        "/run/d2b/host-broker.sock",
        "--state-dir",
        "/var/lib/d2b/host-broker",
        "--audit-dir",
        "/var/lib/d2b/host-audit",
    ]);
    let guest = parse(&[
        "guest",
        "--test-mode",
        "--authority-id",
        "guest-authority",
        "--d2bd-uid",
        "1001",
        "--socket-path",
        "/run/d2b/guest-broker.sock",
        "--state-dir",
        "/var/lib/d2b/guest-broker",
        "--audit-dir",
        "/var/lib/d2b/guest-audit",
    ]);

    let (host, guest) = match (host, guest) {
        (BrokerMode::Host(host), BrokerMode::Guest(guest)) => (host, guest),
        other => panic!("unexpected broker modes: {other:?}"),
    };

    assert_ne!(host.authority_id, guest.authority_id);
    assert_ne!(host.socket_path, guest.socket_path);
    assert_ne!(host.state_dir, guest.state_dir);
    assert_ne!(host.audit_dir, guest.audit_dir);
    assert_ne!(host.d2bd_uid, guest.d2bd_uid);
}

#[test]
fn requests_cannot_select_a_profile() {
    let error = parse_command([
        "host".to_owned(),
        "guest".to_owned(),
        "--test-mode".to_owned(),
    ])
    .expect_err("profile must be a process-start argument, not a request-like trailing token");
    assert!(format!("{error:?}").contains("unknown host flag"));
}

#[test]
#[cfg(not(feature = "layer1-bootstrap"))]
fn host_and_guest_instances_keep_separate_runtime_bindings() {
    let host = TestBroker::spawn_profile("host-instance-", "host-instance", "host", D2BD_UID);
    let guest = TestBroker::spawn_profile("guest-instance-", "guest-instance", "guest", D2BD_UID);

    assert_ne!(host.pid(), guest.pid());
    assert_ne!(host.socket_path(), guest.socket_path());

    let host_client = connect_seqpacket(host.socket_path()).expect("connect host broker");
    let guest_client = connect_seqpacket(guest.socket_path()).expect("connect guest broker");
    let envelope = |request| BrokerRequestEnvelope {
        request,
        caller_role: BrokerCallerRole::AdminUid { uid: D2BD_UID },
        test_peer_uid: Some(D2BD_UID),
        audit_join: None,
    };
    send_json_frame(
        host_client.as_raw_fd(),
        &envelope(BrokerRequest::Hello(HelloRequest {
            client_version: "test-0".to_owned(),
            supported_features: vec![],
        })),
    )
    .expect("send host hello");
    send_json_frame(
        guest_client.as_raw_fd(),
        &envelope(BrokerRequest::Hello(HelloRequest {
            client_version: "test-0".to_owned(),
            supported_features: vec![],
        })),
    )
    .expect("send guest hello");

    let host_response: BrokerResponse = recv_json_frame(host_client.as_raw_fd())
        .expect("receive host hello")
        .expect("host hello response");
    let guest_response: BrokerResponse = recv_json_frame(guest_client.as_raw_fd())
        .expect("receive guest hello")
        .expect("guest hello response");
    let BrokerResponse::Hello(host_hello) = host_response else {
        panic!("host should return Hello");
    };
    let BrokerResponse::Hello(guest_hello) = guest_response else {
        panic!("guest should return Hello");
    };
    assert!(host_hello.capabilities.contains(&"Hello".to_owned()));
    assert!(guest_hello.capabilities.contains(&"SpawnRunner".to_owned()));
    assert!(
        !guest_hello
            .capabilities
            .contains(&"ApplyNftables".to_owned())
    );
    assert_ne!(host.audit_path(), guest.audit_path());
}
