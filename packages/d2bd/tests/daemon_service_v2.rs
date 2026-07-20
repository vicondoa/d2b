use d2b_contracts::v2_component_session::{
    AttachmentPolicyKind, BootstrapPskBinding, EndpointPurpose, EndpointRole,
    GuestBootstrapCredentialV1, GuestBootstrapPsk, GuestIdentityBindingV1,
    GuestSessionCredentialV1, OperationId, ServicePackage, SessionErrorCode,
};
use d2b_contracts::v2_services::{SERVICE_INVENTORY, service_schema_fingerprint};
use d2b_session::{encode_offer, negotiate_offer};
use d2bd::daemon_service::{
    DaemonAdapter, DaemonMethod, daemon_channel_binding, daemon_endpoint_policy,
};

#[test]
fn every_generated_daemon_method_has_one_typed_adapter() {
    let routes = [
        (DaemonMethod::Resolve, DaemonAdapter::Realm),
        (DaemonMethod::ListRealms, DaemonAdapter::Realm),
        (DaemonMethod::ListWorkloads, DaemonAdapter::Realm),
        (DaemonMethod::Inspect, DaemonAdapter::Provider),
        (DaemonMethod::Apply, DaemonAdapter::Allocator),
        (DaemonMethod::Start, DaemonAdapter::Provider),
        (DaemonMethod::Stop, DaemonAdapter::Provider),
        (DaemonMethod::Restart, DaemonAdapter::Provider),
        (DaemonMethod::Exec, DaemonAdapter::Guest),
        (DaemonMethod::Shell, DaemonAdapter::Guest),
        (DaemonMethod::OpenConsole, DaemonAdapter::Guest),
        (DaemonMethod::ExportAudit, DaemonAdapter::Broker),
    ];
    for (method, expected) in routes {
        assert_eq!(method.adapter(), expected, "{}", method.name());
    }
}

#[test]
fn local_daemon_policy_is_fixed_and_has_no_negotiation_or_fd_surface() {
    let binding = daemon_channel_binding(1000, 100);
    let policy = daemon_endpoint_policy(7, binding).expect("daemon endpoint policy");
    assert_eq!(policy.purpose, EndpointPurpose::DaemonLocal);
    assert_eq!(policy.initiator_role, EndpointRole::CommandClient);
    assert_eq!(policy.responder_role, EndpointRole::LocalRootController);
    assert_eq!(policy.service, ServicePackage::DaemonV2);
    assert_eq!(policy.reconnect_generation, 7);
    assert_eq!(policy.transport_binding.channel_binding, binding);
    assert_eq!(
        policy.attachment_policy.kind,
        AttachmentPolicyKind::Disabled
    );
    assert_eq!(
        hex(&policy.schema_fingerprint),
        "4b2834c89162e5a2c17ea879052c066fd546cdc440d1473955a99e2d9521a54a"
    );
    let guest = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.guest.v2")
        .unwrap();
    assert_eq!(
        hex(&service_schema_fingerprint(guest)),
        "e6d2fd47db903deff84b5b9cb58a0aed17e2f6ef43010182925890878a15dd3d"
    );
}

#[test]
fn public_daemon_handshake_rejects_daemon_or_guest_proxy_schema_mismatch() {
    let policy = daemon_endpoint_policy(7, daemon_channel_binding(1000, 100)).unwrap();
    for index in [0, policy.schema_fingerprint.len() - 1] {
        let mut changed = policy.clone();
        changed.schema_fingerprint[index] ^= 0x01;
        let (preface, offer) = encode_offer(&changed).unwrap();
        let error = negotiate_offer(&preface, &offer, &policy).unwrap_err();
        assert_eq!(error.code(), SessionErrorCode::SchemaMismatch);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn daemon_uses_shared_bootstrap_and_enrolled_guest_credential_bindings() {
    let mut psk = [5; 32];
    let bootstrap = GuestBootstrapCredentialV1::new(
        BootstrapPskBinding {
            operation_id: OperationId::new(vec![3; 16]).unwrap(),
            replay_nonce: [4; 32],
            expires_at_unix_ms: 200,
        },
        100,
        GuestBootstrapPsk::copy_from_and_zeroize(&mut psk).unwrap(),
    )
    .unwrap();
    let credential = GuestSessionCredentialV1::new(
        9,
        [1; 32],
        [2; 32],
        GuestIdentityBindingV1::UnboundBootstrap,
        Some(bootstrap),
    )
    .unwrap();
    let encoded = credential.encode().unwrap();
    let decoded = GuestSessionCredentialV1::decode(encoded.as_slice()).unwrap();
    assert_eq!(decoded.session_generation(), 9);
    assert_eq!(decoded.parent_static_public_key(), &[1; 32]);
    assert_eq!(decoded.channel_binding(), &[2; 32]);
    assert!(decoded.guest_identity_is_unbound());
    assert!(decoded.guest_identity_digest().is_none());
    assert!(decoded.guest_static_public_key().is_none());
    decoded.bootstrap().unwrap().admit(100).unwrap();

    let enrolled = GuestSessionCredentialV1::new(
        10,
        [1; 32],
        [2; 32],
        GuestIdentityBindingV1::Enrolled {
            guest_identity_digest: [6; 32],
            guest_static_public_key: [7; 32],
        },
        None,
    )
    .unwrap();
    let encoded = enrolled.encode().unwrap();
    let decoded = GuestSessionCredentialV1::decode(encoded.as_slice()).unwrap();
    assert!(!decoded.guest_identity_is_unbound());
    assert_eq!(decoded.guest_identity_digest(), Some(&[6; 32]));
    assert_eq!(decoded.guest_static_public_key(), Some(&[7; 32]));
    assert_eq!(psk, [0; 32]);
}

#[test]
fn shared_guest_session_credential_rejects_zero_authority() {
    assert!(
        GuestSessionCredentialV1::new(
            0,
            [0; 32],
            [0; 32],
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: [0; 32],
                guest_static_public_key: [0; 32],
            },
            None,
        )
        .is_err()
    );
}

#[test]
fn daemon_guest_paths_do_not_call_broker_signing_or_define_a_private_codec() {
    let sources = [
        include_str!("../src/control_services/daemon.rs"),
        include_str!("../src/guest_control_bridge.rs"),
        include_str!("../src/guest_terminal.rs"),
        include_str!("../src/production_guest_terminal.rs"),
        include_str!("../src/exec_session_real.rs"),
        include_str!("../src/exec_detached.rs"),
        include_str!("../src/lib.rs"),
    ];
    for source in sources {
        assert!(!source.contains(concat!("BrokerRequest::", "Guest", "Control", "Sign")));
        assert!(!source.contains(concat!("GuestSessionMaterial", "Bridge")));
        assert!(!source.contains(concat!("GUEST_SESSION_", "MATERIAL_MAGIC")));
    }
    let daemon = include_str!("../src/lib.rs");
    assert!(daemon.contains("ProductionGuestTerminalConnector::production"));
}
