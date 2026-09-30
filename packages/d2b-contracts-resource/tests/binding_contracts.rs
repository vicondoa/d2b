//! Contract coverage for the five primitive binding relationships.
//!
//! Every case here is a failure the graph exists to prevent: a consumer kind
//! the family does not support, a raw host path or numerical principal
//! smuggled through a request, secret material in a spec, a second
//! declaration colliding with a live slot, an admission that never had a
//! grant or a fence, or an execution-parent input whose three meanings were
//! read as one.

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingConsumerKind,
    BindingContractError, BindingEvidence, BindingKey, BindingKind, BindingLifecycleState,
    BindingObservation, BindingRealizationFacet, BindingRealizationSupport, BindingSlot,
    BindingSlotDecision, BindingSlotIndex, BindingSpecFingerprint, BindingSupportEntry, BoundedToken,
    ChildBindingRequest, ChildRequestDefaults, ChildSupportCeiling, CompletionCondition,
    CredentialBindingRequest, CredentialLifetime, CredentialOperation, DefaultedSource, DesiredDigest,
    DesiredRevision, DeviceAttachmentMode, DeviceBindingRequest, DeviceClaimRequest,
    DeviceFunction, EndpointAttachmentKind, EndpointBindingRequest, ExecutionParentInput,
    ExecutionParentInputClass, FreshnessTuple, NetworkBindingRequest, NetworkMembership,
    NetworkPresentation, PortProtocol, PortSpec, RefusalReason, ReleaseOutcome, RequestedRights,
    ResourceRef, ResourceUid, SourceAdmission, SourceReservation, StoreIncarnation, VolumeBindingRequest,
    VolumePresentation, ZoneId, admit_binding_request,
    volume::AttachmentAccess,
};

const ZONE: &str = "work";
const OTHER_ZONE: &str = "personal";
const VOLUME_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const OTHER_VOLUME_UID: &str = "223e4567-e89b-42d3-a456-426614174001";
const CONSUMER_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
const GUEST_UID: &str = "423e4567-e89b-42d3-a456-426614174003";

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("registered resource reference")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical resource uid")
}

fn zone(value: &str) -> ZoneId {
    ZoneId::parse(value).expect("zone")
}

fn slot(value: &str) -> BindingSlot {
    BindingSlot::parse(value).expect("bounded slot token")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn filesystem() -> VolumePresentation {
    VolumePresentation::filesystem("/state").expect("consumer destination")
}

fn volume_request(
    source: &str,
    consumer: &str,
    access: AttachmentAccess,
    presentation: VolumePresentation,
) -> Result<VolumeBindingRequest, BindingContractError> {
    VolumeBindingRequest::new(
        reference(source),
        reference(consumer),
        slot("state"),
        token("default"),
        access,
        presentation,
    )
}

fn freshness(revision: DesiredRevision, digest: &str) -> FreshnessTuple {
    FreshnessTuple::new(
        zone(ZONE),
        StoreIncarnation::parse("store-one").expect("incarnation"),
        reference("Volume/state"),
        uid(VOLUME_UID),
        revision,
        DesiredDigest::parse(digest).expect("desired digest"),
    )
}

fn digest_of(payload: &str) -> String {
    DesiredDigest::of(payload.as_bytes()).as_str().to_owned()
}

fn next_revision() -> DesiredRevision {
    DesiredRevision::INITIAL.try_next().expect("revision")
}

fn support() -> BindingRealizationSupport {
    BindingRealizationSupport::new(vec![
        BindingRealizationFacet::FilesystemPresentation,
        BindingRealizationFacet::ConsumerDeviceSlot,
        BindingRealizationFacet::DeviceAttachment,
        BindingRealizationFacet::NamespaceInterface,
        BindingRealizationFacet::SharedFabric,
        BindingRealizationFacet::EndpointDescriptor,
        BindingRealizationFacet::EndpointPathname,
        BindingRealizationFacet::CredentialDelivery,
    ])
    .expect("support set")
}

fn source_admission(key: &BindingKey, rights: RequestedRights) -> SourceAdmission {
    SourceAdmission::new(key.clone(), vec![rights], BindingArbitration::Shared)
        .expect("source decision")
}

fn reservation(key: &BindingKey) -> SourceReservation {
    SourceReservation::new(key.zone().clone(), key.source_uid().clone(), token("res-state"))
}

fn credential_request(
    consumer: &str,
    operations: Vec<CredentialOperation>,
) -> Result<CredentialBindingRequest, BindingContractError> {
    CredentialBindingRequest::new(
        reference("Credential/vault"),
        reference(consumer),
        slot("vault"),
        token("workload"),
        operations,
        CredentialLifetime::new("5m", "1h").expect("lifetime"),
    )
}

fn device_request(consumer: &str, claim: DeviceClaimRequest) -> Result<DeviceBindingRequest, BindingContractError> {
    DeviceBindingRequest::new(
        reference("Device/tpm"),
        reference(consumer),
        slot("tpm"),
        DeviceFunction::parse("state").expect("device function"),
        claim,
        DeviceAttachmentMode::Descriptor,
    )
}

fn endpoint_request(
    consumer: &str,
    attachment: EndpointAttachmentKind,
) -> Result<EndpointBindingRequest, BindingContractError> {
    EndpointBindingRequest::new(
        reference("Endpoint/compositor"),
        reference(consumer),
        slot("compositor"),
        attachment,
        token("display"),
    )
}

fn network_request(consumer: &str) -> Result<NetworkBindingRequest, BindingContractError> {
    NetworkBindingRequest::new(
        reference("Network/lan"),
        reference(consumer),
        slot("lan"),
        NetworkMembership::new(Vec::new(), true).expect("membership"),
        NetworkPresentation::shared_fabric(),
    )
}

/// Scenario 1: every kind accepts the four consumer kinds it supports and
/// refuses the one it does not.
#[test]
fn consumer_kinds_are_admitted_per_binding_kind() {
    let consumers = [
        ("Process/worker", BindingConsumerKind::Process),
        ("EphemeralProcess/job", BindingConsumerKind::EphemeralProcess),
        ("Host/desktop", BindingConsumerKind::Host),
        ("Guest/browser", BindingConsumerKind::Guest),
    ];

    for (name, kind) in consumers {
        volume_request("Volume/state", name, AttachmentAccess::ReadOnly, filesystem())
            .unwrap_or_else(|error| panic!("{name} is an admitted volume consumer: {error}"));
        device_request(name, DeviceClaimRequest::Shared)
            .unwrap_or_else(|error| panic!("{name} is an admitted device consumer: {error}"));
        network_request(name)
            .unwrap_or_else(|error| panic!("{name} is an admitted network consumer: {error}"));
        if kind != BindingConsumerKind::Host {
            endpoint_request(name, EndpointAttachmentKind::Connect).unwrap_or_else(|error| {
                panic!("{name} is an admitted endpoint consumer: {error}")
            });
            credential_request(name, vec![CredentialOperation::AcquireToken])
                .unwrap_or_else(|error| {
                    panic!("{name} is an admitted credential consumer: {error}")
                });
        }
        assert!(
            BindingKind::ALL
                .iter()
                .any(|binding| binding.admits_consumer(kind)),
            "{kind:?} is the resolved consumer vocabulary member"
        );
    }

    // A Host is a support ceiling for endpoints and credentials, never their
    // consumer: a host-side delivery is an admitted realization leg of the
    // binding whose consumer is that helper.
    assert_eq!(
        endpoint_request("Host/desktop", EndpointAttachmentKind::Connect),
        Err(BindingContractError::UnsupportedConsumerKind)
    );
    assert_eq!(
        credential_request("Host/desktop", vec![CredentialOperation::AcquireToken]),
        Err(BindingContractError::UnsupportedConsumerKind)
    );
    assert!(!BindingKind::Endpoint.admits_consumer(BindingConsumerKind::Host));
    assert!(BindingKind::Volume.admits_consumer(BindingConsumerKind::Host));

    // A resource outside the four consumer kinds is refused before any
    // eligibility rule applies.
    assert_eq!(
        volume_request(
            "Volume/state",
            "Volume/other",
            AttachmentAccess::ReadOnly,
            filesystem(),
        ),
        Err(BindingContractError::WrongResourceType)
    );
    assert_eq!(
        BindingConsumerKind::from_resource_type("Zone/work"),
        None,
        "the consumer vocabulary is closed"
    );
}

/// Scenario 1 through the identity itself: the same source, consumer, and
/// slot produce one key whatever the payload says, and each kind's identity
/// records both sides exactly.
#[test]
fn identity_is_the_zone_source_consumer_kind_and_slot() {
    let request = volume_request(
        "Volume/state",
        "Guest/browser",
        AttachmentAccess::ReadOnly,
        filesystem(),
    )
    .expect("guest storage is admitted");
    let key = request
        .key(zone(ZONE), uid(VOLUME_UID), uid(GUEST_UID))
        .expect("key");
    assert_eq!(key.kind(), BindingKind::Volume);
    assert_eq!(key.zone(), &zone(ZONE));
    assert_eq!(key.source_ref(), &reference("Volume/state"));
    assert_eq!(key.source_uid(), &uid(VOLUME_UID));
    assert_eq!(key.consumer_ref(), &reference("Guest/browser"));
    assert_eq!(key.consumer_uid(), &uid(GUEST_UID));
    assert_eq!(key.slot().as_str(), "state");

    // The identity is derived, never stated: a source of the wrong type and
    // an unsupported consumer are refused at construction.
    assert_eq!(
        BindingKey::new(
            zone(ZONE),
            BindingKind::Volume,
            reference("Device/gpu"),
            uid(VOLUME_UID),
            reference("Guest/browser"),
            uid(GUEST_UID),
            slot("state"),
        ),
        Err(BindingContractError::WrongResourceType)
    );
    assert_eq!(
        BindingKey::new(
            zone(ZONE),
            BindingKind::Endpoint,
            reference("Endpoint/compositor"),
            uid(VOLUME_UID),
            reference("Host/desktop"),
            uid(CONSUMER_UID),
            slot("state"),
        ),
        Err(BindingContractError::UnsupportedConsumerKind)
    );
}

/// Scenario 2: a desired request carries no raw host path, no numerical
/// principal, no secret material, and no unparsed extra.
#[test]
fn desired_requests_refuse_host_authority_and_unknown_fields() {
    assert_eq!(
        volume_request("Device/gpu", "Process/worker", AttachmentAccess::ReadOnly, filesystem()),
        Err(BindingContractError::WrongResourceType),
        "a Device cannot be a storage source"
    );
    assert_eq!(
        device_request("Process/worker", DeviceClaimRequest::Shared).map(|request| request.source_ref().clone()),
        Ok(reference("Device/tpm"))
    );
    assert_eq!(
        NetworkBindingRequest::new(
            reference("Endpoint/compositor"),
            reference("Process/worker"),
            slot("lan"),
            NetworkMembership::new(Vec::new(), true).expect("membership"),
            NetworkPresentation::shared_fabric(),
        ),
        Err(BindingContractError::WrongResourceType),
        "an Endpoint is not a network source"
    );

    // A destination is consumer-side; a host source location has no field at
    // all, so it cannot be smuggled through one.
    assert_eq!(
        VolumePresentation::filesystem("/var/lib/d2b/state").expect("consumer destination"),
        VolumePresentation::Filesystem {
            destination: "/var/lib/d2b/state".to_owned()
        }
    );
    assert_eq!(
        VolumePresentation::filesystem("relative/state"),
        Err(BindingContractError::InvalidField)
    );
    assert_eq!(
        VolumePresentation::block_device(4096),
        Err(BindingContractError::OutOfRange)
    );

    // The wire mirror denies unknown fields, so a host path, a numerical
    // principal, or secret material cannot arrive as an ignored extra.
    let request = volume_request(
        "Volume/state",
        "Process/worker",
        AttachmentAccess::ReadOnly,
        filesystem(),
    )
    .expect("request");
    let mut value = serde_json::to_value(&request).expect("request renders");
    assert!(
        value.get("sourcePath").is_none(),
        "a request renders no host source path"
    );
    value
        .as_object_mut()
        .expect("request is an object")
        .insert("sourcePath".to_owned(), serde_json::json!("/var/lib/d2b/state"));
    assert!(
        serde_json::from_value::<VolumeBindingRequest>(value.clone()).is_err(),
        "a raw host source path is not an accepted field"
    );
    value
        .as_object_mut()
        .expect("request is an object")
        .insert("consumerUid".to_owned(), serde_json::json!(1000));
    assert!(
        serde_json::from_value::<VolumeBindingRequest>(value).is_err(),
        "a numerical principal is not an accepted field"
    );

    let credential = credential_request("Process/worker", vec![CredentialOperation::AcquireToken])
        .expect("credential request");
    let mut credential_value = serde_json::to_value(&credential).expect("request renders");
    assert_eq!(
        credential_value
            .as_object()
            .expect("request is an object")
            .get("audience")
            .and_then(serde_json::Value::as_str),
        Some("workload")
    );
    credential_value
        .as_object_mut()
        .expect("request is an object")
        .insert("token".to_owned(), serde_json::json!("s3cret"));
    assert!(
        serde_json::from_value::<CredentialBindingRequest>(credential_value).is_err(),
        "credential material is not an accepted field"
    );

    assert_eq!(
        CredentialLifetime::new("2h", "1h"),
        Err(BindingContractError::OutOfRange),
        "a lifetime never outlives its expiry"
    );
    assert_eq!(
        credential_request("Process/worker", Vec::new()),
        Err(BindingContractError::InvalidCollection),
        "an empty operation set is refused"
    );
    assert_eq!(
        credential_request(
            "Process/worker",
            vec![
                CredentialOperation::AcquireToken,
                CredentialOperation::AcquireToken,
            ],
        ),
        Err(BindingContractError::InvalidCollection),
        "a duplicated operation class is refused"
    );
}

/// Scenario 3: identity is the slot, a rights update keeps it, and a
/// conflicting declaration for the same slot is refused before any mutation.
#[test]
fn slot_identity_survives_a_rights_update_and_conflicts_are_refused() {
    let read_only = volume_request(
        "Volume/state",
        "Process/worker",
        AttachmentAccess::ReadOnly,
        filesystem(),
    )
    .expect("request");
    let read_write = volume_request(
        "Volume/state",
        "Process/worker",
        AttachmentAccess::ReadWrite,
        filesystem(),
    )
    .expect("request");
    let read_only_key = read_only
        .key(zone(ZONE), uid(VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    let read_write_key = read_write
        .key(zone(ZONE), uid(VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    assert_eq!(
        read_only_key, read_write_key,
        "rights are payload: the relationship keeps one identity"
    );
    assert_ne!(
        read_only.fingerprint(),
        read_write.fingerprint(),
        "a rights change is visible as a different declaration"
    );

    let mut index = BindingSlotIndex::new();
    assert_eq!(
        index.declare(&read_only_key, &read_only.fingerprint()),
        Ok(BindingSlotDecision::Claimed)
    );
    assert_eq!(
        index.declare(&read_only_key, &read_only.fingerprint()),
        Ok(BindingSlotDecision::Coalesced),
        "an identical declaration coalesces at normalization"
    );
    assert_eq!(
        index.declare(&read_write_key, &read_write.fingerprint()),
        Err(BindingContractError::SlotOccupied),
        "a conflicting declaration for a live slot is refused"
    );

    // A different source in the same slot is a replacement, not a second
    // relationship: it waits for the old binding to release.
    let other_source = volume_request(
        "Volume/other",
        "Process/worker",
        AttachmentAccess::ReadOnly,
        filesystem(),
    )
    .expect("request");
    let other_key = other_source
        .key(zone(ZONE), uid(OTHER_VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    assert_eq!(
        index.declare(&other_key, &other_source.fingerprint()),
        Err(BindingContractError::SourceMismatch)
    );

    // The rights update lands only once the old use is blocked.
    assert_eq!(
        index.change_payload(&read_write_key, &read_write.fingerprint()),
        Err(BindingContractError::SlotOccupied),
        "the previous access is still live"
    );
    index
        .observe(&read_only_key, BindingLifecycleState::Revoking)
        .expect("observed");
    assert_eq!(
        index.change_payload(&read_write_key, &read_write.fingerprint()),
        Ok(BindingSlotDecision::PayloadUpdated),
        "the slot keeps its owner across the update"
    );
    assert_eq!(
        index
            .occupant(&read_only_key.address())
            .map(|entry| entry.state()),
        Some(BindingLifecycleState::Revoking)
    );

    index
        .observe(&read_only_key, BindingLifecycleState::Released)
        .expect("observed");
    assert_eq!(
        index.declare(&other_key, &other_source.fingerprint()),
        Ok(BindingSlotDecision::SuccessorClaimed),
        "the successor takes the slot only after release"
    );
    assert_eq!(
        index.declare(&other_key, &other_source.fingerprint()),
        Ok(BindingSlotDecision::Coalesced)
    );

    // A slot is per Zone, consumer, kind, and slot token.
    let personal_key = read_only
        .key(zone(OTHER_ZONE), uid(VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    assert_ne!(read_only_key.address(), personal_key.address());
    assert!(!BindingLifecycleState::Unknown.admits_new_use());
    assert!(!BindingLifecycleState::Degraded.proves_effect());
}

/// Verification, plus R16 and R35: a binding cannot be constructed as
/// admitted from untrusted desired fields alone, and its authority is fenced
/// against the exact dependency revisions it was evaluated under.
#[test]
fn admitted_evidence_needs_a_grant_a_source_decision_support_and_a_fence() {
    let request = volume_request(
        "Volume/state",
        "Process/worker",
        AttachmentAccess::ReadWrite,
        filesystem(),
    )
    .expect("request");
    let key = request
        .key(zone(ZONE), uid(VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    let rights = request.requested_rights();
    assert_eq!(rights, RequestedRights::Mutate);
    let source = source_admission(&key, RequestedRights::Mutate);
    let dependencies = [freshness(next_revision(), &digest_of("{\"access\":\"read-write\"}"))];

    let refusal = admit_binding_request(
        &key,
        rights,
        request.required_facets(),
        &BindingAuthorization::absent(),
        &source,
        &support(),
        &dependencies,
    )
    .expect_err("an unauthorized request is refused");
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    assert_eq!(refusal.reason(), RefusalReason::IdentityNotAuthorized);

    // A grant for a right the source does not admit is still refused, and the
    // decision cannot be carried to another relationship.
    let source_refusal = admit_binding_request(
        &key,
        RequestedRights::Mutate,
        request.required_facets(),
        &BindingAuthorization::granted(),
        &SourceAdmission::new(
            key.clone(),
            vec![RequestedRights::Observe],
            BindingArbitration::Shared,
        )
        .expect("source decision"),
        &support(),
        &dependencies,
    )
    .expect_err("the source refuses this right");
    assert_eq!(source_refusal.stage(), AdmissionStage::Admit);
    assert_eq!(source_refusal.reason(), RefusalReason::SourcePolicyRefused);

    let other_key = request
        .key(zone(ZONE), uid(OTHER_VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    assert_eq!(
        admit_binding_request(
            &other_key,
            rights,
            request.required_facets(),
            &BindingAuthorization::granted(),
            &source,
            &support(),
            &dependencies,
        )
        .expect_err("a decision is scoped to its relationship")
        .reason(),
        RefusalReason::SourcePolicyRefused
    );

    // An admission without dependency evidence is refused: cached readiness
    // cannot remint access.
    assert_eq!(
        admit_binding_request(
            &key,
            rights,
            request.required_facets(),
            &BindingAuthorization::granted(),
            &source,
            &support(),
            &[],
        )
        .expect_err("an unfenced admission is refused")
        .reason(),
        RefusalReason::UnprovenEffect
    );

    let admission = admit_binding_request(
        &key,
        rights,
        request.required_facets(),
        &BindingAuthorization::granted(),
        &source,
        &support(),
        &dependencies,
    )
    .expect("an authorized, fenced request is admitted");
    let evidence = BindingEvidence::admitted(admission, reservation(&key));
    assert_eq!(evidence.state(), BindingLifecycleState::Admitted);
    assert_eq!(evidence.key(), &key);
    assert_eq!(evidence.reservation().source_uid(), &uid(VOLUME_UID));
    assert!(evidence.is_current(&dependencies));

    // An ownership, view, consumer, provider-assignment, or policy change that
    // does not advance a spec generation still invalidates the fence.
    let changed = [freshness(
        next_revision().try_next().expect("revision"),
        &digest_of("{\"access\":\"read-only\"}"),
    )];
    assert!(
        !evidence.is_current(&changed),
        "a newer desired revision is not current"
    );
    assert!(!evidence.is_current(&[]), "an absent dependency is not current");
    assert_eq!(
        evidence.admission().dependencies().len(),
        1,
        "the admission records the versions it was fenced against"
    );

    // An observation never invents authority: it records lifecycle, source
    // preparation, consumer completion, and release separately.
    let observed = evidence.observed(BindingObservation::new(
        BindingLifecycleState::Prepared,
        CompletionCondition::Complete,
        CompletionCondition::Pending,
        ReleaseOutcome::Outstanding,
    ));
    assert_eq!(observed.state(), BindingLifecycleState::Prepared);
    assert!(observed.observation().prepare().is_complete());
    assert_eq!(
        observed.observation().consumer_completion(),
        CompletionCondition::Pending,
        "consumer completion is never inferred from source preparation"
    );
    assert_eq!(observed.observation().release(), ReleaseOutcome::Outstanding);
}

/// R20 and R27: a presentation the selected realization cannot enforce is
/// refused rather than skipped.
#[test]
fn a_realization_that_cannot_enforce_the_presentation_is_refused() {
    let request = volume_request(
        "Volume/state",
        "Process/worker",
        AttachmentAccess::ReadOnly,
        filesystem(),
    )
    .expect("request");
    let key = request
        .key(zone(ZONE), uid(VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    let dependencies = [freshness(next_revision(), &digest_of("{\"access\":\"read-only\"}"))];
    let partial = BindingRealizationSupport::new(vec![BindingRealizationFacet::ConsumerDeviceSlot])
        .expect("support set");
    let refusal = admit_binding_request(
        &key,
        request.requested_rights(),
        request.required_facets(),
        &BindingAuthorization::granted(),
        &source_admission(&key, RequestedRights::Observe),
        &partial,
        &dependencies,
    )
    .expect_err("a filesystem presentation needs a filesystem realization");
    assert_eq!(refusal.stage(), AdmissionStage::Prepare);
    assert_eq!(refusal.reason(), RefusalReason::MandatoryFacetUnsupported);

    // A block presentation asks for the block facet instead, so the same
    // realization admits exactly the presentation it can enforce.
    let block = volume_request(
        "Volume/state",
        "Process/worker",
        AttachmentAccess::ReadOnly,
        VolumePresentation::block_device(7).expect("device slot"),
    )
    .expect("request");
    assert_eq!(
        block.required_facets(),
        &[BindingRealizationFacet::ConsumerDeviceSlot]
    );
    let block_key = block
        .key(zone(ZONE), uid(VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    admit_binding_request(
        &block_key,
        block.requested_rights(),
        block.required_facets(),
        &BindingAuthorization::granted(),
        &source_admission(&block_key, RequestedRights::Observe),
        &partial,
        &dependencies,
    )
    .expect("the block presentation is realizable");
}

/// R21: an exclusive device claim needs the source to be arbitrating it
/// exclusively, and the source-owned decision is what carries that.
#[test]
fn exclusive_rights_need_exclusive_source_arbitration() {
    let request = device_request("Process/worker", DeviceClaimRequest::Exclusive)
        .expect("device request");
    let key = request
        .key(zone(ZONE), uid(VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    assert_eq!(request.requested_rights(), RequestedRights::Exclusive);
    let dependencies = [freshness(next_revision(), &digest_of("{\"claim\":\"exclusive\"}"))];

    assert_eq!(
        admit_binding_request(
            &key,
            request.requested_rights(),
            request.required_facets(),
            &BindingAuthorization::granted(),
            &source_admission(&key, RequestedRights::Exclusive),
            &support(),
            &dependencies,
        )
        .expect_err("a shared source cannot admit an exclusive claim")
        .reason(),
        RefusalReason::SourcePolicyRefused
    );
    let exclusive =
        SourceAdmission::new(key.clone(), vec![RequestedRights::Exclusive], BindingArbitration::Exclusive)
            .expect("source decision");
    let admission = admit_binding_request(
        &key,
        request.requested_rights(),
        request.required_facets(),
        &BindingAuthorization::granted(),
        &exclusive,
        &support(),
        &dependencies,
    )
    .expect("the source arbitrates this claim exclusively");
    assert_eq!(admission.arbitration(), BindingArbitration::Exclusive);
    assert_eq!(
        device_request("Process/worker", DeviceClaimRequest::Shared)
            .expect("shared request")
            .requested_rights(),
        RequestedRights::Share
    );
}

/// R23: an endpoint request reaches exactly one endpoint, and its attachment
/// kind decides which realization facets it needs.
#[test]
fn endpoint_requests_reach_one_exact_endpoint() {
    let attach = endpoint_request("Process/shell", EndpointAttachmentKind::Attach).expect("request");
    assert_eq!(attach.source_ref(), &reference("Endpoint/compositor"));
    assert_eq!(attach.requested_rights(), RequestedRights::Consume);
    assert_eq!(
        attach.required_facets(),
        &[
            BindingRealizationFacet::EndpointDescriptor,
            BindingRealizationFacet::EndpointPathname,
        ],
        "an attachment needs the private binding of the exact socket"
    );

    let listen = endpoint_request("Process/shell", EndpointAttachmentKind::Listen).expect("request");
    assert_eq!(listen.requested_rights(), RequestedRights::Observe);
    assert_eq!(
        listen.required_facets(),
        &[BindingRealizationFacet::EndpointDescriptor],
        "a connect or listen prefers the verified descriptor"
    );
    assert_ne!(
        listen.fingerprint(),
        attach.fingerprint(),
        "a different attachment kind is a different declaration"
    );
}

/// AE31: a child target-support ceiling bounds child admission and creates no
/// binding of its own.
#[test]
fn a_support_ceiling_admits_children_and_creates_no_binding() {
    let input: ExecutionParentInput<VolumeBindingRequest> = ExecutionParentInput::ChildSupportCeiling(
        ChildSupportCeiling::new(vec![
            BindingSupportEntry::new(BindingKind::Volume, vec![RequestedRights::Observe]).expect("entry"),
            BindingSupportEntry::new(BindingKind::Network, vec![RequestedRights::Consume]).expect("entry"),
        ])
        .expect("ceiling"),
    );

    assert_eq!(input.class(), ExecutionParentInputClass::ChildSupportCeiling);
    assert!(!input.yields_binding(), "a ceiling is not a relationship");
    assert!(input.support_ceiling().is_some());
    assert!(input.parent_use().is_none());
    assert!(input.child_defaults().is_none());
    assert_eq!(
        input.admits_child_request(BindingKind::Volume, RequestedRights::Observe),
        Ok(())
    );
    assert_eq!(
        input
            .admits_child_request(BindingKind::Volume, RequestedRights::Mutate)
            .expect_err("a write is outside the ceiling")
            .reason(),
        RefusalReason::TargetSupportMissing
    );
    assert_eq!(
        input
            .admits_child_request(BindingKind::Credential, RequestedRights::Consume)
            .expect_err("the ceiling does not list credentials")
            .reason(),
        RefusalReason::TargetSupportMissing
    );
    assert_eq!(
        BindingSupportEntry::new(BindingKind::Volume, vec![RequestedRights::Exclusive]),
        Err(BindingContractError::UnsupportedRight),
        "a ceiling cannot list a right the kind does not admit"
    );
}

/// AE32: an input that describes the parent's own consumption produces a
/// binding whose consumer is that parent.
#[test]
fn a_parent_use_becomes_a_binding_for_that_parent() {
    let guest = volume_request(
        "Volume/state",
        "Guest/browser",
        AttachmentAccess::ReadOnly,
        filesystem(),
    )
    .expect("request");
    let input: ExecutionParentInput<VolumeBindingRequest> = ExecutionParentInput::ParentUse(guest);
    assert_eq!(input.class(), ExecutionParentInputClass::ParentUse);
    assert!(input.yields_binding(), "a parent use is a relationship");
    let request = input.parent_use().expect("the parent's request");
    let key = request
        .key(zone(ZONE), uid(VOLUME_UID), uid(GUEST_UID))
        .expect("key");
    assert_eq!(
        key.consumer_ref(),
        &reference("Guest/browser"),
        "the consumer is the parent, not a child"
    );
    assert_eq!(
        input
            .admits_child_request(BindingKind::Volume, RequestedRights::Observe)
            .expect_err("a parent use is not a child ceiling")
            .reason(),
        RefusalReason::TargetSupportMissing
    );

    // The same conversion for a Host parent consuming a Device.
    let host = device_request("Host/desktop", DeviceClaimRequest::Shared).expect("request");
    let host_key = host
        .key(zone(ZONE), uid(VOLUME_UID), uid(CONSUMER_UID))
        .expect("key");
    assert_eq!(host_key.consumer_ref(), &reference("Host/desktop"));
    assert_eq!(host_key.kind(), BindingKind::Device);
}

/// AE33: a child default shapes one named child's request and grants the
/// parent nothing.
#[test]
fn a_child_default_reaches_only_its_child_request() {
    let input: ExecutionParentInput<VolumeBindingRequest> =
        ExecutionParentInput::ChildRequestDefaults(
            ChildRequestDefaults::new(
                reference("Process/worker"),
                DefaultedSource::new(BindingKind::Volume, reference("Volume/state"), Some(token("default")))
                    .expect("defaulted source"),
            )
            .expect("defaults"),
        );
    assert_eq!(input.class(), ExecutionParentInputClass::ChildRequestDefaults);
    assert!(!input.yields_binding(), "a default is not a relationship");
    assert!(input.child_defaults().is_some());
    let defaults = input.child_defaults().expect("the defaults");

    let draft = ChildBindingRequest::new(reference("Process/worker"), BindingKind::Volume)
        .expect("draft");
    let applied = draft.apply_defaults(defaults).expect("applied");
    assert_eq!(applied.source_ref(), Some(&reference("Volume/state")));
    assert_eq!(applied.view().map(|view| view.as_str()), Some("default"));
    assert_eq!(applied.rights(), Some(RequestedRights::Observe));
    assert_eq!(applied.consumer_ref(), &reference("Process/worker"));

    // The parent gained nothing: the default names a child, not the parent.
    let parent_draft = ChildBindingRequest::new(reference("Guest/browser"), BindingKind::Volume)
        .expect("draft");
    assert_eq!(
        parent_draft.apply_defaults(defaults).err(),
        Some(BindingContractError::WrongConsumer)
    );

    // A child that declared its own source and rights keeps them.
    let declared = ChildBindingRequest::new(reference("Process/worker"), BindingKind::Volume)
        .expect("draft")
        .declaring(reference("Volume/other"), None, RequestedRights::Mutate)
        .expect("declared");
    let inherited = declared.apply_defaults(defaults).expect("applied");
    assert_eq!(inherited.source_ref(), Some(&reference("Volume/other")));
    assert_eq!(inherited.rights(), Some(RequestedRights::Mutate));

    assert_eq!(
        ChildRequestDefaults::new(
            reference("Guest/browser"),
            DefaultedSource::new(BindingKind::Volume, reference("Volume/state"), None)
                .expect("source"),
        ),
        Err(BindingContractError::UnsupportedConsumerKind),
        "an execution parent is not a child default target"
    );
    assert_eq!(
        DefaultedSource::new(BindingKind::Volume, reference("Device/gpu"), None),
        Err(BindingContractError::WrongResourceType)
    );
}

/// The rights vocabulary stays closed per kind, so no family's right can be
/// traded for another's by spelling a different variant.
#[test]
fn rights_are_closed_per_binding_kind() {
    assert!(!BindingKind::Volume.admits_rights(RequestedRights::Exclusive));
    assert!(!BindingKind::Device.admits_rights(RequestedRights::Mutate));
    assert!(!BindingKind::Network.admits_rights(RequestedRights::Mutate));
    assert!(!BindingKind::Endpoint.admits_rights(RequestedRights::Share));
    assert!(!BindingKind::Credential.admits_rights(RequestedRights::Observe));
    assert_eq!(BindingKind::Volume.source_resource_type(), "Volume");
    assert_eq!(BindingKind::Credential.resource_type(), "CredentialBinding");
    assert_eq!(
        BindingKind::from_source_resource_type("Network"),
        Some(BindingKind::Network)
    );
    assert_eq!(BindingKind::from_source_resource_type("Zone"), None);
    assert!(RequestedRights::Mutate.needs_arbitration());
    assert!(!RequestedRights::Observe.needs_arbitration());
}

/// The wire mirror round-trips an exact request and denies an extra field.
#[test]
fn a_request_round_trips_through_its_wire_mirror() {
    let request = volume_request(
        "Volume/state",
        "Process/worker",
        AttachmentAccess::SharedWrite,
        filesystem(),
    )
    .expect("request");
    let encoded = serde_json::to_string(&request).expect("request renders");
    assert!(
        !encoded.contains("sourcePath"),
        "no host source path is rendered"
    );
    let decoded: VolumeBindingRequest = serde_json::from_str(&encoded).expect("request decodes");
    assert_eq!(decoded, request);
    assert_eq!(decoded.access(), AttachmentAccess::SharedWrite);
    assert_eq!(decoded.requested_rights(), RequestedRights::Share);

    let network = NetworkBindingRequest::new(
        reference("Network/lan"),
        reference("Process/worker"),
        slot("lan"),
        NetworkMembership::new(
            vec![PortSpec::new(8080, PortProtocol::Tcp, "http").expect("port")],
            true,
        )
        .expect("membership"),
        NetworkPresentation::namespace_interface("eth9").expect("interface"),
    )
    .expect("network request");
    let encoded = serde_json::to_string(&network).expect("request renders");
    let decoded: NetworkBindingRequest = serde_json::from_str(&encoded).expect("request decodes");
    assert_eq!(decoded, network);
    assert_eq!(
        decoded
            .presentation()
            .interface_name()
            .map(|name| name.as_str()),
        Some("eth9")
    );
    assert!(decoded.membership().allow_egress());
    assert_eq!(decoded.membership().ports().len(), 1);

    let mut extra = serde_json::to_value(&network).expect("request renders");
    extra
        .as_object_mut()
        .expect("request is an object")
        .insert("interface".to_owned(), serde_json::json!("eth0"));
    assert!(
        serde_json::from_value::<NetworkBindingRequest>(extra).is_err(),
        "an unparsed extra never enters the contract"
    );
}

/// The digest that identifies a declaration is a framed canonical digest, so
/// two equal requests always coalesce and two different ones never do.
#[test]
fn a_declaration_digest_is_stable_and_framed() {
    let request = volume_request(
        "Volume/state",
        "Process/worker",
        AttachmentAccess::ReadOnly,
        filesystem(),
    )
    .expect("request");
    let fingerprint = request.fingerprint();
    assert!(fingerprint.as_str().starts_with("sha256:"));
    assert_eq!(
        BindingSpecFingerprint::parse(fingerprint.as_str()).expect("framed digest"),
        fingerprint
    );
    assert_eq!(
        BindingSpecFingerprint::parse("not-a-digest"),
        Err(BindingContractError::InvalidField)
    );
    assert_eq!(
        request.fingerprint(),
        volume_request(
            "Volume/state",
            "Process/worker",
            AttachmentAccess::ReadOnly,
            filesystem(),
        )
        .expect("identical request")
        .fingerprint()
    );
    assert_ne!(
        request.fingerprint(),
        volume_request(
            "Volume/state",
            "Process/worker",
            AttachmentAccess::ReadWrite,
            filesystem(),
        )
        .expect("changed request")
        .fingerprint()
    );
}