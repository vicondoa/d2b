//! The committed `VolumeBinding` row is the consumer's own canonical
//! declaration plus the source provider's accepted decision.
//!
//! KTD2 makes the consumer's desired request the one declaration site and KTD3
//! makes its key the relationship's identity.  The row the source mints is
//! that declaration in one closed encoding, so the manager's relation index
//! and the registered serving driver read the same bytes: what the consumer
//! authored is what a reader decodes, indexes, serves, and admits.  These
//! cases are the failures that would follow if it did not - a row shape that
//! guesses a relationship out of an attachment list, a slot two sources can
//! both occupy, a reader and a writer that collapse into one right, and a
//! writable host path smuggled in beside the consumer-side destination.

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::AttachmentAccess;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingConsumerKind, BindingKind,
    BindingRealizationFacet, BindingRealizationSupport, BindingRefusal, BindingSlot,
    BindingSlotDecision, BindingSlotIndex, BindingSourceDecision, BindingSpecFingerprint,
    CanonicalJsonValue, RefusalReason, DesiredDigest, DesiredRevision, FreshnessTuple,
    RequestedRights, ResourceRef, ResourceUid, SourceAdmission, StoreIncarnation, StoredResource,
    VolumeBindingRequest, VolumeBindingSpec, VolumePresentation, ZoneId, admit_binding_request,
    canonical_json_bytes,
};
use d2b_provider_volume_binding::{parsed_binding_spec, parsed_consumer_request};
use d2b_resource_runtime::relations::DecodedBindingRequest;

const ZONE: &str = "work";
const VOLUME_UID: &str = "6f9619ff-8b86-4d01-b42d-00cf4fc964ff";
const OTHER_VOLUME_UID: &str = "7f9619ff-8b86-4d01-b42d-00cf4fc964ff";

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("registered resource reference")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical resource uid")
}

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone")
}

fn slot(value: &str) -> BindingSlot {
    BindingSlot::parse(value).expect("bounded slot token")
}

fn filesystem(destination: &str) -> VolumePresentation {
    VolumePresentation::filesystem(destination).expect("consumer destination")
}

fn consumer_request(
    source: &str,
    consumer: &str,
    consumer_slot: &str,
    access: AttachmentAccess,
    presentation: VolumePresentation,
) -> VolumeBindingRequest {
    VolumeBindingRequest::new(
        reference(source),
        reference(consumer),
        slot(consumer_slot),
        BoundedToken::parse("controller").expect("view token"),
        access,
        presentation,
    )
    .expect("canonical consumer request")
}

/// Every consumer kind the Volume binding admits, with the reference it is.
const CONSUMERS: [(&str, &str); 4] = [
    ("Process/worker", "323e4567-e89b-42d3-a456-426614174002"),
    ("EphemeralProcess/task", "423e4567-e89b-42d3-a456-426614174003"),
    ("Host/host-system", "223e4567-e89b-42d3-a456-426614174001"),
    ("Guest/work-vm", "123e4567-e89b-42d3-a456-426614174000"),
];

/// The committed row the source mints for one admitted declaration: the
/// consumer's own request plus the source provider's accepted decision.
fn committed_row(request: &VolumeBindingRequest) -> VolumeBindingSpec {
    VolumeBindingSpec::new(
        request.source_ref().clone(),
        request.consumer_ref().clone(),
        request.view().as_str(),
        request.access(),
        request.presentation().clone(),
        request.slot().as_str(),
        BindingSourceDecision::new(
            vec![RequestedRights::Consume],
            BindingArbitration::Shared,
            request.required_facets().to_vec(),
        )
        .expect("source decision"),
    )
    .expect("the committed row this family admits")
}

/// The exact bytes the source commits for one declaration.
fn committed_bytes(request: &VolumeBindingRequest) -> Vec<u8> {
    canonical_json_bytes(&committed_row(request)).expect("canonical row bytes")
}

fn key_for(request: &VolumeBindingRequest, consumer_uid: ResourceUid) -> d2b_contracts_resource::v3::BindingKey {
    request
        .key(zone(), uid(VOLUME_UID), consumer_uid)
        .expect("the committed identities describe an admitted relationship")
}

/// The dependency fence an admission is evaluated against: the source row and
/// the consumer row it was read beside.  An admission without one is refused
/// as unproven, so every case below carries a real fence.
fn fence(source_uid: &str, consumer: (&str, &str)) -> Vec<FreshnessTuple> {
    [
        ("Volume/state", source_uid),
        (consumer.0, consumer.1),
    ]
    .into_iter()
    .map(|(name, identity)| {
        FreshnessTuple::new(
            zone(),
            StoreIncarnation::parse("store-one").expect("store incarnation"),
            reference(name),
            uid(identity),
            DesiredRevision::INITIAL,
            DesiredDigest::of(name.as_bytes()),
        )
    })
    .collect()
}

fn support() -> BindingRealizationSupport {
    BindingRealizationSupport::new(vec![
        BindingRealizationFacet::FilesystemPresentation,
        BindingRealizationFacet::ConsumerDeviceSlot,
    ])
    .expect("support set")
}

fn stored_binding_row(request: &VolumeBindingRequest) -> StoredResource {
    let value = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "VolumeBinding",
        "metadata": {
            "name": "vol-binding-row",
            "zone": ZONE,
            "ownerRef": request.source_ref().to_canonical_string(),
            "labels": {},
            "annotations": {},
            "finalizers": [],
            "managedBy": "controller",
            "deletionRequestedAt": null,
            "createdAt": "2026-08-19T00:00:00.000Z",
            "updatedAt": "2026-08-19T00:00:00.000Z",
            "generation": 1,
            "revision": 1,
            "uid": "a23e4567-e89b-42d3-a456-426614174000"
        },
        "spec": serde_json::from_slice::<serde_json::Value>(&committed_bytes(request))
            .expect("canonical row value"),
        "status": { "resource": {} }
    });
    StoredResource {
        resource_ref: reference("VolumeBinding/vol-binding-row"),
        zone: zone(),
        uid: uid("a23e4567-e89b-42d3-a456-426614174000"),
        owner_uid: None,
        owner_generation: None,
        generation: d2b_contracts_resource::v3::ResourceGeneration::new(1)
            .expect("canonical generation"),
        revision: d2b_contracts_resource::v3::ZoneRevision::new(1),
        canonical_json: CanonicalJsonValue::parse(
            &serde_json::to_vec(&value).expect("resource serialization"),
        )
        .expect("canonical resource")
        .to_canonical_bytes(),
        payload_digest: d2b_contracts_resource::v3::StateDigest::parse(format!(
            "sha256:{}",
            "0".repeat(64)
        ))
        .expect("a zero digest is a valid state digest"),
    }
}

#[test]
fn every_consumer_kind_and_presentation_round_trips_through_one_row_shape() {
    for (consumer, consumer_uid_value) in CONSUMERS {
        for (label, presentation) in [
            ("filesystem", filesystem("/state")),
            (
                "block-device",
                VolumePresentation::block_device(3).expect("device slot"),
            ),
        ] {
            let request = consumer_request(
                "Volume/state",
                consumer,
                "state",
                AttachmentAccess::ReadWrite,
                presentation,
            );
            let bytes = committed_bytes(&request);
            let decoded = DecodedBindingRequest::decode("VolumeBinding", &bytes)
                .unwrap_or_else(|| panic!("{consumer} {label} did not commit as a canonical row"));
            assert_eq!(decoded.kind(), BindingKind::Volume);
            assert_eq!(decoded.source_ref(), request.source_ref());
            assert_eq!(decoded.consumer_ref(), request.consumer_ref());
            assert_eq!(decoded.slot(), request.slot());
            assert_eq!(decoded.rights(), RequestedRights::Mutate);
            assert_eq!(
                decoded.required_facets(),
                request.required_facets(),
                "{consumer} {label} must commit the facets its presentation needs"
            );
            assert_eq!(decoded.fingerprint(), &committed_row(&request).fingerprint());
            // The consumer slot named a relationship the graph can key, for
            // every consumer kind and both presentations alike.
            let key = decoded
                .key(zone(), uid(VOLUME_UID), uid(consumer_uid_value))
                .expect("keyable relationship");
            assert_eq!(key.consumer_ref(), &reference(consumer));
            assert_eq!(
                BindingConsumerKind::from_resource_type(consumer.split('/').next().unwrap_or_default()),
                Some(BindingConsumerKind::from_resource_type(
                    consumer.split('/').next().unwrap_or_default()
                )
                .expect("consumer kind"))
            );
        }
    }
}

#[test]
fn a_committed_row_reads_back_as_the_request_the_consumer_authored() {
    let request = consumer_request(
        "Volume/state",
        "Guest/work-vm",
        "state",
        AttachmentAccess::SharedWrite,
        filesystem("/srv/state"),
    );
    let row = stored_binding_row(&request);
    let read_back = parsed_consumer_request(&row).expect("canonical row reads back");
    assert_eq!(read_back, request);
    assert_eq!(read_back.access(), AttachmentAccess::SharedWrite);
    assert_eq!(
        read_back.presentation().destination(),
        Some("/srv/state")
    );
}

#[test]
fn a_row_that_is_not_the_committed_shape_declares_no_relationship() {
    // The pre-cutover attachment row is a different spec, not the committed
    // row.  It declares no indexed consumption relationship rather than a
    // guessed one, and the reader says so instead of translating it.
    let legacy = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "VolumeBinding",
        "metadata": {
            "name": "vol-binding-legacy",
            "zone": ZONE,
            "ownerRef": "Volume/state",
            "labels": {},
            "annotations": {},
            "finalizers": [],
            "managedBy": "controller",
            "deletionRequestedAt": null,
            "createdAt": "2026-08-19T00:00:00.000Z",
            "updatedAt": "2026-08-19T00:00:00.000Z",
            "generation": 1,
            "revision": 1,
            "uid": "a23e4567-e89b-42d3-a456-426614174000"
        },
        "spec": {
            "volumeRef": "Volume/state",
            "executionRef": "Guest/work-vm",
            "view": "controller",
            "access": "read-write",
            "mountPath": "/srv/state"
        },
        "status": { "resource": {} }
    });
    let row = StoredResource {
        resource_ref: reference("VolumeBinding/vol-binding-legacy"),
        zone: zone(),
        uid: uid("a23e4567-e89b-42d3-a456-426614174000"),
        owner_uid: None,
        owner_generation: None,
        generation: d2b_contracts_resource::v3::ResourceGeneration::new(1)
            .expect("canonical generation"),
        revision: d2b_contracts_resource::v3::ZoneRevision::new(1),
        canonical_json: CanonicalJsonValue::parse(
            &serde_json::to_vec(&legacy).expect("resource serialization"),
        )
        .expect("canonical resource")
        .to_canonical_bytes(),
        payload_digest: d2b_contracts_resource::v3::StateDigest::parse(format!(
            "sha256:{}",
            "0".repeat(64)
        ))
        .expect("a zero digest is a valid state digest"),
    };
    assert!(
        parsed_consumer_request(&row).is_none(),
        "an attachment-shaped row is not the committed row"
    );
    // The row contract carries the source provider's accepted decision and the
    // consumer-side presentation, so a row that predates both is refused
    // rather than read with its authority silently absent or a destination
    // invented for it.
    assert!(
        parsed_binding_spec(&row).is_none(),
        "a row carrying no committed source decision declares no relationship"
    );
    assert!(
        DecodedBindingRequest::decode(
            "VolumeBinding",
            &canonical_json_bytes_from_value(
                &serde_json::from_slice::<serde_json::Value>(&row.canonical_json)
                    .expect("canonical resource")["spec"],
            ),
        )
        .is_none()
    );
}

#[test]
fn a_writable_path_grant_is_not_a_field_a_committed_row_accepts() {
    let request = consumer_request(
        "Volume/state",
        "Process/worker",
        "work",
        AttachmentAccess::ReadWrite,
        filesystem("/srv/work"),
    );
    let mut granted = serde_json::from_slice::<serde_json::Value>(&committed_bytes(&request))
        .expect("canonical row value");
    // A host-side path next to the consumer-side destination would be a
    // second, writable-path grant living in the same row.
    granted["hostPath"] = serde_json::Value::String("/var/lib/d2b/volumes/state".to_owned());
    assert!(
        DecodedBindingRequest::decode(
            "VolumeBinding",
            &canonical_json_bytes_from_value(&granted),
        )
        .is_none(),
        "a host path beside the destination must not decode as a committed row"
    );
    assert!(
        serde_json::from_value::<VolumeBindingSpec>(granted).is_err(),
        "a host path is not an accepted field of the committed row"
    );
    // The destination the row does carry is the consumer's, not the source's.
    assert_eq!(request.presentation().destination(), Some("/srv/work"));
}

fn canonical_json_bytes_from_value(value: &serde_json::Value) -> Vec<u8> {
    CanonicalJsonValue::parse(&serde_json::to_vec(value).expect("serializable value"))
        .expect("canonical value")
        .to_canonical_bytes()
}

#[test]
fn one_consumer_slot_holds_one_relationship() {
    let mut slots = BindingSlotIndex::new();
    let request = consumer_request(
        "Volume/state",
        "Process/worker",
        "work",
        AttachmentAccess::ReadWrite,
        filesystem("/srv/work"),
    );
    let key = key_for(&request, uid(CONSUMERS[0].1));
    assert_eq!(
        slots.declare(&key, &request.fingerprint()),
        Ok(BindingSlotDecision::Claimed)
    );
    // The same declaration again is the same relationship, not a second one.
    assert_eq!(
        slots.declare(&key, &request.fingerprint()),
        Ok(BindingSlotDecision::Coalesced)
    );
    // A second source in the same slot is a replacement, and both are refused
    // while the slot is live.
    let other_source = consumer_request(
        "Volume/other",
        "Process/worker",
        "work",
        AttachmentAccess::ReadOnly,
        filesystem("/srv/other"),
    );
    let other_key = other_source
        .key(zone(), uid(OTHER_VOLUME_UID), uid(CONSUMERS[0].1))
        .expect("keyable relationship");
    assert!(
        slots.declare(&other_key, &other_source.fingerprint()).is_err(),
        "a live slot cannot be taken by a second source"
    );
    // A second slot on the same consumer is a distinct relationship.
    let sibling_slot = consumer_request(
        "Volume/state",
        "Process/worker",
        "mirror",
        AttachmentAccess::ReadOnly,
        filesystem("/srv/mirror"),
    );
    let sibling_key = key_for(&sibling_slot, uid(CONSUMERS[0].1));
    assert_eq!(
        slots.declare(&sibling_key, &sibling_slot.fingerprint()),
        Ok(BindingSlotDecision::Claimed)
    );
    assert_eq!(slots.entries().count(), 2);
}

#[test]
fn a_payload_change_is_an_update_of_one_relationship_not_a_second_one() {
    let mut slots = BindingSlotIndex::new();
    let read_only = consumer_request(
        "Volume/state",
        "Process/worker",
        "work",
        AttachmentAccess::ReadOnly,
        filesystem("/srv/work"),
    );
    let mutating = consumer_request(
        "Volume/state",
        "Process/worker",
        "work",
        AttachmentAccess::ReadWrite,
        filesystem("/srv/work"),
    );
    let key = key_for(&read_only, uid(CONSUMERS[0].1));
    assert_eq!(
        slots.declare(&key, &read_only.fingerprint()),
        Ok(BindingSlotDecision::Claimed)
    );
    assert_ne!(read_only.fingerprint(), mutating.fingerprint());
    // Widening a live relationship is refused rather than admitted beside it.
    assert!(
        slots.change_payload(&key, &mutating.fingerprint()).is_err(),
        "a live relationship cannot be widened in place"
    );
    // With the relationship released, the same slot takes the new payload.
    slots
        .observe(&key, d2b_contracts_resource::v3::BindingLifecycleState::Released)
        .expect("release observation");
    assert_eq!(
        slots.change_payload(&key, &mutating.fingerprint()),
        Ok(BindingSlotDecision::PayloadUpdated)
    );
    assert_eq!(slots.entries().count(), 1);
}

#[test]
fn a_committed_row_never_widens_the_right_it_declares() {
    let cases = [
        (AttachmentAccess::ReadOnly, RequestedRights::Observe),
        (AttachmentAccess::ReadWrite, RequestedRights::Mutate),
        (AttachmentAccess::SharedWrite, RequestedRights::Share),
    ];
    for (access, rights) in cases {
        let request = consumer_request(
            "Volume/state",
            "Guest/work-vm",
            "state",
            access,
            filesystem("/srv/state"),
        );
        let bytes = committed_bytes(&request);
        let decoded = DecodedBindingRequest::decode("VolumeBinding", &bytes)
            .expect("the committed row decodes");
        assert_eq!(decoded.rights(), rights, "{access:?} must commit its own right");
        // The source decides which of these it admits; a read-only
        // relationship is never admitted as a writer by its own row.
        let key = decoded
            .key(zone(), uid(VOLUME_UID), uid(CONSUMERS[3].1))
            .expect("keyable relationship");
        let granted = vec![RequestedRights::Observe];
        let decision = SourceAdmission::new(
            key.clone(),
            granted,
            if rights == RequestedRights::Mutate {
                BindingArbitration::Exclusive
            } else {
                BindingArbitration::Shared
            },
        )
        .expect("source decision");
        let outcome = admit_binding_request(
            &key,
            rights,
            decoded.required_facets(),
            &BindingAuthorization::granted(),
            &decision,
            &support(),
            &fence(VOLUME_UID, CONSUMERS[3]),
        );
        if rights == RequestedRights::Observe {
            assert!(outcome.is_ok(), "a read-only row is admissible as one");
        } else {
            assert_eq!(
                outcome.unwrap_err(),
                BindingRefusal::new(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused),
                "a row cannot widen itself past what the source admits"
            );
        }
    }
}

#[test]
fn a_presentation_the_selected_realization_cannot_enforce_is_refused() {
    let request = consumer_request(
        "Volume/state",
        "Host/host-system",
        "host-state",
        AttachmentAccess::ReadOnly,
        VolumePresentation::block_device(1).expect("device slot"),
    );
    let key = key_for(&request, uid(CONSUMERS[2].1));
    let decision = SourceAdmission::new(key.clone(), vec![RequestedRights::Observe], BindingArbitration::Shared)
        .expect("source decision");
    let filesystem_only =
        BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
            .expect("support set");
    assert_eq!(
        admit_binding_request(
            &key,
            RequestedRights::Observe,
            request.required_facets(),
            &BindingAuthorization::granted(),
            &decision,
            &filesystem_only,
            &fence(VOLUME_UID, CONSUMERS[2]),
        )
        .unwrap_err(),
        BindingRefusal::new(
            AdmissionStage::Prepare,
            RefusalReason::MandatoryFacetUnsupported
        )
    );
    assert!(
        admit_binding_request(
            &key,
            RequestedRights::Observe,
            request.required_facets(),
            &BindingAuthorization::granted(),
            &decision,
            &support(),
            &fence(VOLUME_UID, CONSUMERS[2]),
        )
        .is_ok()
    );
}

#[test]
fn a_changed_desired_payload_gets_its_own_fingerprint() {
    let first = consumer_request(
        "Volume/state",
        "Process/worker",
        "work",
        AttachmentAccess::ReadOnly,
        filesystem("/srv/work"),
    );
    let second = consumer_request(
        "Volume/state",
        "Process/other",
        "work",
        AttachmentAccess::ReadOnly,
        filesystem("/srv/work"),
    );
    let fingerprint = BindingSpecFingerprint::from_request(&first);
    assert_eq!(fingerprint, first.fingerprint());
    assert_ne!(fingerprint, second.fingerprint());
    assert_eq!(
        BindingSpecFingerprint::parse(fingerprint.as_str()),
        Ok(fingerprint)
    );
}
