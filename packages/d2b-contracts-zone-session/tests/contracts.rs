use d2b_contracts_zone_session::v3::{
    component_session::{
        AttachmentPolicy, AttachmentPolicyKind, EndpointPolicy, EndpointPolicyIdentity,
        EndpointPurpose, EndpointRole, IdentityEvidenceRequirement, LimitProfile, Locality,
        NoiseProfile, PurposeClass, ServicePackage, TransportBinding, TransportClass,
    },
};

fn enrolled_guest_policy(purpose: EndpointPurpose) -> EndpointPolicy {
    EndpointPolicy {
        purpose,
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
        reconnect_generation: 1,
        attachment_policy: AttachmentPolicy::disabled(),
    }
}

/// The generic Guest target-control session is named for what it is (R20):
/// it is an enrolled guest ComponentSession, and only that purpose validates
/// it. ZoneLink's own profiles stay exactly where they were.
#[test]
fn the_generic_guest_target_session_is_a_component_session_not_a_zone_link() {
    let control = enrolled_guest_policy(EndpointPurpose::ComponentSession);
    assert!(control.validate_enrolled_guest_session().is_ok());
    assert!(
        control.validate_zone_link().is_err(),
        "generic target traffic must never validate as ZoneLink traffic"
    );
    assert!(
        EndpointPolicyIdentity::from(&control)
            .validate_generation_discovery()
            .is_ok(),
        "the generic target-control session discovers reconnect generations"
    );

    let zone_link = enrolled_guest_policy(EndpointPurpose::ZoneLink);
    assert!(zone_link.validate_zone_link().is_ok());
    assert!(
        zone_link.validate_enrolled_guest_session().is_err(),
        "a ZoneLink carriage policy is not the generic target-control profile"
    );

    for mutate in [
        |policy: &mut EndpointPolicy| policy.purpose = EndpointPurpose::ZoneLink,
        |policy: &mut EndpointPolicy| policy.purpose_class = PurposeClass::Local,
        |policy: &mut EndpointPolicy| policy.reconnect_generation = 0,
        |policy: &mut EndpointPolicy| policy.responder_role = EndpointRole::Component,
        |policy: &mut EndpointPolicy| {
            policy.transport_binding.locality = Locality::Remote;
        },
        |policy: &mut EndpointPolicy| {
            policy.transport_binding.identity_evidence =
                IdentityEvidenceRequirement::DirectionalUnix;
        },
        |policy: &mut EndpointPolicy| {
            policy.service = ServicePackage::ControllerV3;
        },
    ] {
        let mut policy = enrolled_guest_policy(EndpointPurpose::ComponentSession);
        mutate(&mut policy);
        assert!(
            policy.validate_enrolled_guest_session().is_err(),
            "the generic target-control profile is exact"
        );
    }
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
