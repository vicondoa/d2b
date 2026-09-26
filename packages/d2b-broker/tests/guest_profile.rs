use d2b_contracts_broker::broker_wire::BrokerProfile;

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
    BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, BrokerResponse, EnvelopeInvokeRequest,
};

#[test]
fn guest_profile_admits_only_local_process_effects() {
    // U10 retired the typed process-family wire variants (SpawnRunner,
    // OpenPidfd, ObserveRunner, SignalRunner, DeregisterRunnerPidfd among
    // them), so the guest catalog no longer admits them; the guest-local
    // effects that remain are the broker lifecycle handshakes and the
    // generic envelope surface.
    for operation in [
        "Hello",
        "PublishTrustedContext",
        "ExportBrokerAudit",
        "EnvelopeInvoke",
    ] {
        assert!(
            BrokerProfile::Guest.allows_operation(operation),
            "guest profile should admit declared local effect {operation}"
        );
        assert!(
            BrokerProfile::Guest
                .operations()
                .iter()
                .any(|item| item.as_str() == operation),
            "guest profile lost the committed local effect {operation}"
        );
    }
    for operation in [
        "SpawnRunner",
        "OpenPidfd",
        "ObserveRunner",
        "SignalRunner",
        "DeregisterRunnerPidfd",
        "OpenPeerPidfdFromAcceptedSocket",
        "PollChildReaped",
        // U11 retired the typed lease arm with its row: the lease now
        // rides the generic consume-cell/complete-cell kernels through
        // the envelope, so the typed variant is gone from every catalog.
        "ConsumeLifecycleLease",
        // U12 retired the thirteen network-fds family wire variants the
        // same way: their privileged cores are the broker-generic network
        // kernels served through the envelope, so the typed variants are
        // gone from every catalog.
        "ApplyNftables",
        "ApplyNftablesProjection",
        "ApplyNmUnmanaged",
        "ApplyRoute",
        "ApplySysctl",
        "CreateBridge",
        "DeleteBridge",
        "CreatePersistentTap",
        "DeletePersistentTap",
        "CreateTapFd",
        "SetBridgePortFlags",
        "UpdateHostsFile",
        "SeedDnsmasqLease",
        // U15 retired the five process-systemd family wire variants the
        // same way: their privileged cores are the family handlers served
        // through the broker's forward seam, so the typed variants are
        // gone from every catalog.
        "StartSystemdUnit",
        "CheckSystemdUserManager",
        "ObserveSystemdUnit",
        "OpenSystemdUnitPidfd",
        "StopSystemdUnit",
    ] {
        assert!(
            !BrokerProfile::Guest.allows_operation(operation),
            "guest profile must not admit retired operation {operation}"
        );
    }
}

#[test]
fn guest_profile_rejects_every_host_only_effect_class() {
    // The retired network-family variants (ApplyNftables, ApplyRoute,
    // ApplySysctl, CreateBridge among them) are covered by the retired
    // negative list above; the host-only classes that remain are the
    // device, storage, realm, and allocator operations.
    for operation in [
        "OpenKvm",
        "OpenHidrawSecurityKey",
        "StoreSync",
        "RunHostInstall",
        "RunMigrate",
        "UsbipBind",
        "SecurityKeyOpenDevice",
    ] {
        assert!(
            !BrokerProfile::Guest.allows_operation(operation),
            "guest profile must reject host-only operation {operation}"
        );
    }
}

#[test]
#[cfg(not(feature = "layer1-bootstrap"))]
fn guest_binary_rejects_host_effects_before_bundle_mutation() {
    // U12 retired the typed ApplyNftables arm: the host-only effect now
    // rides the broker-generic apply-nftables kernel through the
    // EnvelopeInvoke surface, and the guest profile refuses it inside the
    // envelope response - the guest envelope commits only rows that admit
    // the Guest profile, and the network kernels admit Host alone, so the
    // call is refused as uncommitted before any payload validation or
    // bundle mutation. The refusal is audited as the envelope's chain
    // record under the kernel's own operation name.
    let broker = TestBroker::spawn_profile("guest-profile-", "guest-test", "guest", D2BD_UID);
    let client = connect_seqpacket(broker.socket_path()).expect("connect guest broker");
    let envelope = BrokerRequestEnvelope {
        request: BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
            operation: "apply-nftables".to_owned(),
            zone: "env:work".to_owned(),
            payload: serde_json::json!({ "family": "inet", "table": "d2b" }),
            chain_root_invocation_id: None,
            chain_identities: None,
            fd_indexes: Vec::new(),
            fd_kinds: Vec::new(),
        }),
        caller_role: BrokerCallerRole::AdminUid { uid: D2BD_UID },
        audit_join: None,
    };
    send_json_frame(
        client.as_raw_fd(),
        &d2b_broker::runtime::test_peer_uid_frame(envelope, Some(D2BD_UID)),
    )
    .expect("send host-only request");
    let response: BrokerResponse = recv_json_frame(client.as_raw_fd())
        .expect("receive guest profile response")
        .expect("guest broker response");

    let BrokerResponse::EnvelopeInvoke(response) = response else {
        panic!("guest profile must answer the envelope surface");
    };
    assert_eq!(response.operation, "apply-nftables");
    assert_eq!(
        response.refusal.as_deref(),
        Some(d2b_broker::envelope::UNCOMMITTED_OPERATION),
        "the guest envelope must refuse the host-only kernel as uncommitted: {response:?}"
    );
    assert!(response.result.is_none());
    let audit = broker.audit_contents();
    assert!(
        audit.contains(r#""operation":"apply-nftables""#),
        "the refusal's chain record carries the kernel op name: {audit}"
    );
    assert!(audit.contains(r#""outcome":"refused""#), "{audit}");
    assert!(
        audit.contains(r#""code":"uncommitted-operation""#),
        "{audit}"
    );
}

#[test]
#[cfg(not(feature = "layer1-bootstrap"))]
fn guest_binary_refuses_an_old_runner_frame_at_the_wire_gate() {
    use d2b_broker::runtime::RETIRED_WIRE_VARIANTS;

    // The retired `SpawnRunner` guest admission question is gone with the
    // arm: a straggler frame for any retired process-family variant is
    // refused by the retired-wire gate (KTD10) before any profile decision,
    // so a retired runner op can never reach the guest admission logic.
    // The gate answers the typed stale-wire-version refusal and appends an
    // audit record under the retired variant's own operation name.
    let broker =
        TestBroker::spawn_profile("guest-runner-profile-", "guest-test", "guest", D2BD_UID);
    for retired in RETIRED_WIRE_VARIANTS {
        let client = connect_seqpacket(broker.socket_path()).expect("connect guest broker");
        send_json_frame(
            client.as_raw_fd(),
            &serde_json::json!({
                "request": { "kind": retired.variant, "payload": {} },
            }),
        )
        .expect("send the straggler frame");
        let response: BrokerResponse = recv_json_frame(client.as_raw_fd())
            .expect("receive the gate reply")
            .expect("the guest broker wrote a reply");
        let BrokerResponse::Error(error) = response else {
            panic!(
                "guest broker must refuse {} at the wire gate",
                retired.variant
            );
        };
        assert_eq!(error.kind, d2b_broker::envelope::STALE_WIRE_VERSION);
        assert_eq!(error.operation, retired.variant);
    }

    let audit = broker.audit_contents();
    for retired in RETIRED_WIRE_VARIANTS {
        assert!(
            audit.contains(&format!(r#""op":"{}""#, retired.variant)),
            "the gate's audit record carries the retired op name {}: {audit}",
            retired.variant
        );
    }
    assert!(
        audit.contains(r#""disposition":"stale-wire-version""#),
        "{audit}"
    );
    assert!(audit.contains(r#""outcome":"refused""#), "{audit}");
}
