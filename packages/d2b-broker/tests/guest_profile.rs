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
use d2b_contracts::types::{BundleOpId, ScopeId};
#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_contracts_broker::broker_wire::{
    ApplyNftablesRequest, BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, BrokerResponse,
};

#[test]
fn guest_profile_admits_only_local_process_effects() {
    // U10 retired the typed process-family wire variants (SpawnRunner,
    // OpenPidfd, ObserveRunner, SignalRunner, DeregisterRunnerPidfd among
    // them), so the guest catalog no longer admits them; the guest-local
    // effects that remain are the systemd unit ops and the generic
    // envelope surface.
    for operation in [
        "Hello",
        "PublishTrustedContext",
        "ExportBrokerAudit",
        "StartSystemdUnit",
        "CheckSystemdUserManager",
        "ObserveSystemdUnit",
        "OpenSystemdUnitPidfd",
        "StopSystemdUnit",
        "EnvelopeInvoke",
    ] {
        assert!(
            BrokerProfile::Guest.allows_operation(operation),
            "guest profile should admit declared local effect {operation}"
        );
        assert!(
            BrokerProfile::Guest.operations().contains(&operation),
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
    ] {
        assert!(
            !BrokerProfile::Guest.allows_operation(operation),
            "guest profile must not admit retired process-family operation {operation}"
        );
    }
    assert!(
        !BrokerProfile::Guest.allows_operation("ConsumeLifecycleLease"),
        "guest profile must not consume host Guest lifecycle leases"
    );
}

#[test]
fn guest_profile_rejects_every_host_only_effect_class() {
    for operation in [
        "ApplyNftables",
        "ApplyRoute",
        "ApplySysctl",
        "CreateBridge",
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
    let broker = TestBroker::spawn_profile("guest-profile-", "guest-test", "guest", D2BD_UID);
    let client = connect_seqpacket(broker.socket_path()).expect("connect guest broker");
    let envelope = BrokerRequestEnvelope {
        request: BrokerRequest::ApplyNftables(ApplyNftablesRequest {
            bundle_nft_intent_ref: BundleOpId::new("nft:host-only"),
            scope_id: ScopeId::new("env:work"),
            desired_hash: None,
            destroy: false,
            tracing_span_id: None,
        }),
        caller_role: BrokerCallerRole::AdminUid { uid: D2BD_UID },
        test_peer_uid: Some(D2BD_UID),
        audit_join: None,
    };
    send_json_frame(client.as_raw_fd(), &envelope).expect("send host-only request");
    let response: BrokerResponse = recv_json_frame(client.as_raw_fd())
        .expect("receive guest profile response")
        .expect("guest broker response");

    let BrokerResponse::Error(error) = response else {
        panic!("guest profile must return a typed denial");
    };
    assert_eq!(error.kind, "Broker.ProfileOperationDenied");
    assert_eq!(error.operation, "ApplyNftables");
    assert!(broker.audit_contents().contains("profile-operation-denied"));
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
            panic!("guest broker must refuse {} at the wire gate", retired.variant);
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