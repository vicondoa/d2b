use d2b_contracts_resource::v3::ZoneId;
use d2b_contracts_zone_session::v3::{
    ZoneSpec,
    component_session::{
        AttachmentPolicy, AttachmentPolicyKind, EndpointPolicyIdentity, EndpointPurpose,
        EndpointRole, IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile,
        PurposeClass, ServicePackage, TransportBinding, TransportClass,
    },
};

#[test]
fn zone_session_contracts_preserve_zone_identity() {
    let zone = ZoneId::parse("work").expect("valid zone");
    let _ = std::any::type_name::<ZoneSpec>();
    assert_eq!(zone.as_str(), "work");
}

fn enrolled_guest_discovery_identity() -> EndpointPolicyIdentity {
    EndpointPolicyIdentity {
        purpose: EndpointPurpose::ZoneLink,
        purpose_class: PurposeClass::Enrolled,
        initiator_role: EndpointRole::ZoneController,
        responder_role: EndpointRole::GuestAgent,
        service: ServicePackage::ResourceV3,
        schema_fingerprint: [0x11; 32],
        noise_profile: NoiseProfile::Kk25519ChaChaPolySha256,
        limits: LimitProfile::remote_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::NativeVsock,
            locality: Locality::GuestLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::EnrolledStaticKeys,
        },
        attachment_policy: AttachmentPolicy::disabled(),
    }
}

#[test]
fn generation_discovery_accepts_only_the_exact_enrolled_guest_profile() {
    let identity = enrolled_guest_discovery_identity();
    assert!(identity.validate_generation_discovery().is_ok());

    let mut wrong_role = identity.clone();
    wrong_role.responder_role = EndpointRole::Relay;
    assert!(wrong_role.validate_generation_discovery().is_err());

    let mut wrong_purpose = identity.clone();
    wrong_purpose.purpose = EndpointPurpose::ResourceService;
    assert!(wrong_purpose.validate_generation_discovery().is_err());

    let mut wrong_noise = identity.clone();
    wrong_noise.noise_profile = NoiseProfile::Nn25519ChaChaPolySha256;
    assert!(wrong_noise.validate_generation_discovery().is_err());

    let mut wrong_evidence = identity.clone();
    wrong_evidence.transport_binding.identity_evidence =
        IdentityEvidenceRequirement::DirectionalUnix;
    assert!(wrong_evidence.validate_generation_discovery().is_err());

    let mut wrong_transport = identity.clone();
    wrong_transport.transport_binding.transport = TransportClass::ProviderStream;
    assert!(wrong_transport.validate_generation_discovery().is_err());

    let mut wrong_locality = identity.clone();
    wrong_locality.transport_binding.locality = Locality::Remote;
    assert!(wrong_locality.validate_generation_discovery().is_err());

    let mut attachments = identity.clone();
    attachments.attachment_policy = AttachmentPolicy {
        kind: AttachmentPolicyKind::PacketAtomic,
        max_per_packet: 1,
        max_per_request: 1,
        max_per_operation: 1,
        max_per_session: 1,
        credentials_allowed: false,
    };
    assert!(attachments.validate_generation_discovery().is_err());

    let mut zero_schema = identity.clone();
    zero_schema.schema_fingerprint = [0; 32];
    assert!(zero_schema.validate_generation_discovery().is_err());

    let mut zero_channel_binding = identity;
    zero_channel_binding.transport_binding.channel_binding = [0; 32];
    assert!(
        zero_channel_binding
            .validate_generation_discovery()
            .is_err()
    );
}
