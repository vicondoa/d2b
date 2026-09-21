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
    AuditJoinContext, BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, BrokerResponse,
    CanonicalAuditDigest, EnvelopeInvokeRequest, HelloRequest,
};
#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_contracts_resource::v3::ResourceUid;

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
        "--d2bd-uid",
        "1000",
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
    // U10 retired the process-family wire variants, U12 the network-fds
    // family variants, and U15 the process-systemd family variants: the
    // guest advertises only the remaining broker lifecycle handshakes and
    // never a retired process, network, or systemd operation.
    for retired in [
        "SpawnRunner",
        "OpenPidfd",
        "SignalRunner",
        "ApplyNftables",
        "CreateTapFd",
        "SeedDnsmasqLease",
        "CreateBridge",
        "ApplySysctl",
        "StartSystemdUnit",
        "CheckSystemdUserManager",
        "ObserveSystemdUnit",
        "OpenSystemdUnitPidfd",
        "StopSystemdUnit",
    ] {
        assert!(
            !guest_hello.capabilities.contains(&retired.to_owned()),
            "guest must not advertise the retired operation {retired}"
        );
    }
    assert_ne!(host.audit_path(), guest.audit_path());
}

#[test]
#[cfg(not(feature = "layer1-bootstrap"))]
fn host_executor_consumes_lifecycle_lease_once_through_the_cell_kernels() {
    // U11 retired the typed lease arm with its row: the host executor's
    // lease now rides the generic consume-cell kernel through the
    // EnvelopeInvoke surface, so this test drives the real broker binary
    // with the envelope frame the migrated daemon caller sends. The
    // stop_only/HostShutdownRestricted fence moved caller-side (the
    // kernels are generic and never learn a lease shape), so the broker
    // admits the host-shutdown envelope calls; the caller-side fence is
    // pinned by the d2bd caller test.
    let broker = TestBroker::spawn_profile("lifecycle-lease-", "host-instance", "host", D2BD_UID);
    let uid = |value: &str| ResourceUid::parse(value).expect("valid UID");
    let zone_uid = uid("11111111-1111-4111-8111-111111111111");
    let lease_payload = |operation_id: &str, guest_uid: &str, operation: &str, stop_only: bool| {
        serde_json::json!({
            "zoneUid": zone_uid.as_str(),
            "guestUid": uid(guest_uid).as_str(),
            "guestGeneration": 4,
            "providerAssignmentGeneration": 9,
            "policyRevision": 7,
            "operationId": operation_id,
            "operation": operation,
            "stopOnly": stop_only,
        })
    };
    let request = |operation_id: &str, guest_uid: &str, operation: &str, stop_only: bool| {
        BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
            operation: "consume-cell".to_owned(),
            zone: zone_uid.as_str().to_owned(),
            payload: lease_payload(operation_id, guest_uid, operation, stop_only),
            chain_root_invocation_id: None,
            chain_identities: None,
            fd_indexes: Vec::new(),
            fd_kinds: Vec::new(),
        })
    };
    let send = |request: BrokerRequest, caller_role: BrokerCallerRole| {
        let client = connect_seqpacket(broker.socket_path()).expect("connect host broker");
        let audit_join = request
            .authoritative_audit_join()
            .map(|(zone_id, operation_identity)| AuditJoinContext {
                zone_id: CanonicalAuditDigest::parse(zone_id).expect("audit Zone digest"),
                operation_identity: CanonicalAuditDigest::parse(operation_identity)
                    .expect("audit operation digest"),
            });
        send_json_frame(
            client.as_raw_fd(),
            &BrokerRequestEnvelope {
                request,
                caller_role,
                test_peer_uid: Some(D2BD_UID),
                audit_join,
            },
        )
        .unwrap_or_else(|error| {
            panic!(
                "send lifecycle lease: {error}; broker log: {}",
                broker.server_log()
            )
        });
        recv_json_frame(client.as_raw_fd())
            .expect("receive lifecycle lease")
            .expect("lifecycle lease response")
    };
    let consumed = |response: &BrokerResponse| match response {
        BrokerResponse::EnvelopeInvoke(response) => {
            response.refusal.is_none()
                && response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("consumed"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
        }
        _ => false,
    };

    let response = send(
        request(
            "lease-once",
            "22222222-2222-4222-8222-222222222222",
            "start",
            false,
        ),
        BrokerCallerRole::AdminUid { uid: D2BD_UID },
    );
    assert!(consumed(&response), "first consume wins: {response:?}");

    // The caller's two-phase flow completes the claim before the effect
    // runs; only then is the marker durable and the replay refused.
    let complete = send(
        BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
            operation: "complete-cell".to_owned(),
            zone: zone_uid.as_str().to_owned(),
            payload: lease_payload(
                "lease-once",
                "22222222-2222-4222-8222-222222222222",
                "start",
                false,
            ),
            chain_root_invocation_id: None,
            chain_identities: None,
            fd_indexes: Vec::new(),
            fd_kinds: Vec::new(),
        }),
        BrokerCallerRole::AdminUid { uid: D2BD_UID },
    );
    assert!(matches!(
        complete,
        BrokerResponse::EnvelopeInvoke(response)
            if response.refusal.is_none()
                && response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("completed"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
    ));

    let replay = send(
        request(
            "lease-once",
            "22222222-2222-4222-8222-222222222222",
            "start",
            false,
        ),
        BrokerCallerRole::AdminUid { uid: D2BD_UID },
    );
    assert!(matches!(
        replay,
        BrokerResponse::EnvelopeInvoke(response)
            if response.refusal.as_deref() == Some(d2b_broker::envelope::HANDLER_REFUSED)
                && response.detail.as_deref() == Some("cell-replayed")
    ));

    let same_operation_different_guest = send(
        request(
            "lease-once",
            "33333333-3333-4333-8333-333333333333",
            "start",
            false,
        ),
        BrokerCallerRole::AdminUid { uid: D2BD_UID },
    );
    assert!(
        consumed(&same_operation_different_guest),
        "same operation id with a different guest is a fresh grant: {same_operation_different_guest:?}"
    );

    // The host-shutdown caller is admitted to the envelope (the broker's
    // HostShutdownUid gate admits EnvelopeInvoke, and the kernel rows
    // grant the daemon class); the stop_only fence itself is caller-side
    // now, so the broker's generic kernels grant both shapes.
    let shutdown_start = send(
        request(
            "shutdown-start",
            "22222222-2222-4222-8222-222222222222",
            "start",
            true,
        ),
        BrokerCallerRole::HostShutdownUid { uid: 0 },
    );
    assert!(
        consumed(&shutdown_start),
        "the broker's generic cell kernel admits the host-shutdown caller: {shutdown_start:?}"
    );

    let shutdown_stop = send(
        request(
            "shutdown-stop",
            "22222222-2222-4222-8222-222222222222",
            "stop",
            true,
        ),
        BrokerCallerRole::HostShutdownUid { uid: 0 },
    );
    assert!(
        consumed(&shutdown_stop),
        "the host-shutdown stop lease consumes: {shutdown_stop:?}"
    );
}
