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
use d2b_contracts::types::{BundleOpId, RoleId, ScopeId, VmId};
#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_contracts_broker::broker_wire::{
    ApplyNftablesRequest, BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, BrokerResponse,
    RunnerRole, SpawnRunnerRequest,
};
#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_contracts_resource::v3::ResourceRef;

#[test]
fn guest_profile_admits_only_local_process_effects() {
    for operation in [
        "Hello",
        "ValidateBundle",
        "ExportBrokerAudit",
        "SpawnRunner",
        "OpenPidfd",
        "ObserveRunner",
        "SignalRunner",
        "DeregisterRunnerPidfd",
        "StartSystemdUnit",
        "StopSystemdUnit",
    ] {
        assert!(
            BrokerProfile::Guest.allows_operation(operation),
            "guest profile should admit declared local effect {operation}"
        );
    }
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
        "OpenZoneStore",
        "StoreSync",
        "RunHostInstall",
        "RunMigrate",
        "CutoverEffect",
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
fn guest_binary_rejects_spawn_runner_before_bundle_mutation() {
    let broker =
        TestBroker::spawn_profile("guest-runner-profile-", "guest-test", "guest", D2BD_UID);

    for execution_ref in ["Guest/guest-vm", "Host/host"] {
        let client = connect_seqpacket(broker.socket_path()).expect("connect guest broker");
        let envelope = BrokerRequestEnvelope {
            request: BrokerRequest::SpawnRunner(SpawnRunnerRequest {
                vm_id: VmId::new("guest-vm"),
                role_id: RoleId::new("cloud-hypervisor"),
                resource_ref: None,
                resource_uid: None,
                bundle_content_identity: None,
                provider_identity: None,
                template_identity: None,
                generation: None,
                activation_input: None,
                sandbox_plan: None,
                role: RunnerRole::CloudHypervisor,
                bundle_runner_intent_ref: BundleOpId::new("runner:test"),
                execution_ref: Some(
                    ResourceRef::parse(execution_ref).expect("valid execution ref"),
                ),
                execution_domain: None,
                user_ref: None,
                guest_execution: None,
                runtime_allocations: Vec::new(),
                tracing_span_id: None,
                workload_identity: None,
            }),
            caller_role: BrokerCallerRole::AdminUid { uid: D2BD_UID },
            test_peer_uid: Some(D2BD_UID),
            audit_join: None,
        };
        send_json_frame(client.as_raw_fd(), &envelope).expect("send SpawnRunner request");
        let response: BrokerResponse = recv_json_frame(client.as_raw_fd())
            .expect("receive guest profile response")
            .expect("guest broker response");

        let BrokerResponse::Error(error) = response else {
            panic!("guest broker must refuse SpawnRunner before bundle dispatch");
        };
        assert_eq!(error.kind, "Broker.ProfileOperationDenied");
        assert_eq!(error.operation, "SpawnRunner");
    }

    assert!(broker.audit_contents().contains("profile-operation-denied"));
}
