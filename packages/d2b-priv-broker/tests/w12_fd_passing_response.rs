use d2b_contracts::v2_component_session::{
    AttachmentPolicyKind, EndpointRole, IdentityEvidenceRequirement, NoiseProfile, ServicePackage,
    TransportClass,
};
use d2b_priv_broker::service_v2::{BrokerPeerRole, broker_channel_binding, broker_endpoint_policy};

#[test]
fn broker_policy_requires_authenticated_atomic_attachment_packets() {
    let binding = broker_channel_binding(100, 200, EndpointRole::LocalRootBroker);
    let policy = broker_endpoint_policy(
        BrokerPeerRole::LocalRootController,
        EndpointRole::LocalRootBroker,
        7,
        binding,
    )
    .expect("broker policy");

    assert_eq!(policy.service, ServicePackage::BrokerV2);
    assert_eq!(policy.noise_profile, NoiseProfile::Nn25519ChaChaPolySha256);
    assert_eq!(
        policy.transport_binding.transport,
        TransportClass::UnixSeqpacket
    );
    assert_eq!(
        policy.transport_binding.identity_evidence,
        IdentityEvidenceRequirement::DirectionalUnix
    );
    assert_eq!(
        policy.attachment_policy.kind,
        AttachmentPolicyKind::PacketAtomic
    );
    assert!(!policy.attachment_policy.credentials_allowed);
    assert!(policy.attachment_policy.max_per_request > 0);
}

#[test]
fn channel_binding_changes_with_admitted_identity_and_broker_role() {
    let local = broker_channel_binding(100, 200, EndpointRole::LocalRootBroker);
    assert_ne!(
        local,
        broker_channel_binding(101, 200, EndpointRole::LocalRootBroker)
    );
    assert_ne!(
        local,
        broker_channel_binding(100, 201, EndpointRole::LocalRootBroker)
    );
    assert_ne!(
        local,
        broker_channel_binding(100, 200, EndpointRole::RealmBroker)
    );
}
