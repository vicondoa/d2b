mod common;

use d2b_contracts::v2_component_session::{
    EndpointRole, IdentityEvidenceRequirement, Locality, TransportClass,
};

#[test]
fn broker_endpoint_requires_host_local_directional_identity() {
    let policy = common::local_root_policy();
    assert_eq!(policy.transport_binding.locality, Locality::HostLocal);
    assert_eq!(policy.initiator_role, EndpointRole::LocalRootController);
    assert_eq!(
        policy.transport_binding.transport,
        TransportClass::UnixSeqpacket
    );
    assert_eq!(
        policy.transport_binding.identity_evidence,
        IdentityEvidenceRequirement::DirectionalUnix
    );
}
