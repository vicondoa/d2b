use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_guest_qemu_media::{
    DeviceAdmission, DeviceObservation, DevicePhase, PlatformClass, ProcessSpec, RuntimeVolumeSpec,
    build_process_spec, runtime_volume_name, validate_process_spec,
};

fn guest() -> ResourceRef {
    ResourceRef::parse("Guest/media-vm").unwrap()
}

#[test]
fn runtime_volume_is_ephemeral_and_waits_for_process_proof() {
    let volume = RuntimeVolumeSpec::new(guest(), "corp", 10 * 1024 * 1024, 1024).unwrap();
    assert_eq!(volume.cleanup_policy(), "vm-stop-with-proof");
    assert_eq!(volume.owner_ref(), &guest());
    assert_eq!(
        volume.provider_ref.to_canonical_string(),
        "Provider/volume-local"
    );
    assert_eq!(volume.views.len(), 2);
    assert_eq!(
        volume.layout[0].restart_policy,
        "preserve-across-controller-restart"
    );
    assert_eq!(volume.layout[1].restart_policy, "clear-on-runner-restart");
    assert_eq!(volume.layout[2].restart_policy, "clear-on-runner-restart");
    assert!(volume.validate().is_ok());
    assert!(serde_json::to_string(&volume).unwrap().contains("qmp.sock"));
}

#[test]
fn runtime_volume_name_follows_the_declared_shape() {
    assert_eq!(runtime_volume_name("media-vm"), "media-vm-runtime");
    assert_eq!(runtime_volume_name("a"), "a-runtime");
    // The controller's runtime Volume row and the daemon's dependency lookup
    // both resolve the name from this declaration: the guest name plus the
    // declared suffix.
    let volume = RuntimeVolumeSpec::new(guest(), "corp", 10 * 1024 * 1024, 1024).unwrap();
    assert!(volume.name.ends_with("-runtime"));
    assert!(volume.validate().is_ok());
}

#[test]
fn device_admission_requires_the_guest_owner() {
    let observation = DeviceObservation {
        device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
        phase: DevicePhase::Ready,
        owner_ref: Some(ResourceRef::parse("Guest/other").unwrap()),
        platform: PlatformClass::X86_64Linux,
        authority_key: [7_u8; 32],
        process_identity: Some("process-a".to_owned()),
        media_contract: "qemu-media/v1".to_owned(),
    };
    assert!(
        DeviceAdmission::validate(&guest(), &observation, "process-a", "qemu-media/v1").is_err()
    );
    let owned = DeviceObservation {
        owner_ref: Some(guest()),
        ..observation
    };
    assert!(DeviceAdmission::validate(&guest(), &owned, "process-a", "qemu-media/v1").is_ok());
    let shared = DeviceObservation {
        owner_ref: None,
        ..owned
    };
    assert!(DeviceAdmission::validate(&guest(), &shared, "process-a", "qemu-media/v1").is_ok());
}

#[test]
fn process_spec_contains_only_opaque_attachments() {
    let process = build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        [ResourceRef::parse("Network/corp-net").unwrap()],
    )
    .unwrap();
    let json = serde_json::to_string(&process).unwrap();
    assert!(json.contains("qemu-media-runner"));
    assert!(!json.contains("argv"));
    assert!(!json.contains("/nix/store"));
    assert!(!json.contains("broker"));
}

#[test]
fn process_spec_uses_canonical_contract_and_rejects_shadow_fields() {
    let process = build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        [],
    )
    .unwrap();
    validate_process_spec(&process).unwrap();
    assert!(
        serde_json::from_str::<ProcessSpec>(
            r#"{"providerRef":"Provider/runtime-qemu-media","template":"qemu-media-runner"}"#
        )
        .is_err()
    );
}

#[test]
fn process_validation_checks_the_canonical_execution_template() {
    let process = build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        [],
    )
    .unwrap();
    let mut json = serde_json::to_value(&process).unwrap();
    json["template"] = serde_json::json!("wrong-template");
    let changed: ProcessSpec = serde_json::from_value(json).unwrap();
    assert!(validate_process_spec(&changed).is_err());
}

// ---------------------------------------------------------------------------
// Admitted relationship projection
// ---------------------------------------------------------------------------

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingLifecycleState,
    BindingRealizationFacet, BindingSlot, BoundedToken, DeviceAttachmentMode,
    DeviceBindingRequest, DeviceClaimRequest, DeviceFunction, EndpointAttachmentKind,
    EndpointBindingRequest, RefusalReason, RequestedRights, SourceAdmission, SourceReservation,
    volume::AttachmentAccess,
    volume_binding::{VolumeBindingRequest, VolumePresentation},
};
use d2b_provider_guest_qemu_media::{
    AdmittedRelationship, AttachmentKind, GuestMediaBindings, ImplementationLeg, MediaAdmissionError,
    MediaRequest, RelationshipEvidence, SlotRequirements, qemu_media_realization_support,
    test_fixtures,
};
use d2b_provider_guest_qemu_media::{KVM_FUNCTION, RUNTIME_VOLUME_SLOT};

fn kvm_uid() -> d2b_contracts_resource::v3::ResourceUid {
    test_fixtures::uid(2)
}

fn tap_uid() -> d2b_contracts_resource::v3::ResourceUid {
    test_fixtures::uid(3)
}

fn media_uid(index: u16) -> d2b_contracts_resource::v3::ResourceUid {
    test_fixtures::uid(4 + u8::try_from(index).unwrap())
}

fn display_uid() -> d2b_contracts_resource::v3::ResourceUid {
    test_fixtures::uid(9)
}

fn slot(value: &str) -> BindingSlot {
    BindingSlot::parse(value).unwrap()
}

fn requirements() -> SlotRequirements {
    SlotRequirements {
        kvm: true,
        tap: true,
        media: 2,
        display: true,
    }
}

fn full_bindings() -> GuestMediaBindings {
    let guest = guest();
    let consumer = test_fixtures::guest_uid();
    GuestMediaBindings::new(
        test_fixtures::zone(),
        consumer.clone(),
        [
            test_fixtures::admitted(test_fixtures::kvm_request(&guest), &kvm_uid(), &consumer),
            test_fixtures::admitted(test_fixtures::tap_request(&guest), &tap_uid(), &consumer),
            test_fixtures::admitted(
                test_fixtures::media_request(&guest, "boot", 0),
                &media_uid(0),
                &consumer,
            ),
            test_fixtures::admitted(
                test_fixtures::media_request(&guest, "data", 1),
                &media_uid(1),
                &consumer,
            ),
            test_fixtures::admitted(
                test_fixtures::display_request(&guest),
                &display_uid(),
                &consumer,
            ),
        ],
    )
}

/// Scenario 1: every projected descriptor matches an admitted source,
/// consumer, and access mode, and nothing outside the admitted set projects.
#[test]
fn every_slot_matches_an_admitted_source_consumer_and_right() {
    let guest = guest();
    let projections = full_bindings().project(&guest, requirements()).unwrap();

    assert_eq!(projections.labels(), ["kvm", "tap-0", "media-0", "media-1", "display"]);

    let kvm = projections.kvm().unwrap();
    assert_eq!(kvm.kind(), AttachmentKind::Kvm);
    assert_eq!(kvm.key().source_ref(), &ResourceRef::parse("Device/host-kvm").unwrap());
    assert_eq!(kvm.key().consumer_ref(), &guest);
    assert_eq!(kvm.right(), RequestedRights::Share);
    assert_eq!(kvm.device_slot(), None);

    let tap = projections.tap().unwrap();
    assert_eq!(tap.kind(), AttachmentKind::Tap);
    assert_eq!(tap.key().source_ref(), &ResourceRef::parse("Network/corp-net").unwrap());
    assert_eq!(tap.key().consumer_ref(), &guest);
    assert_eq!(tap.right(), RequestedRights::Consume);

    let media: Vec<_> = projections.media().collect();
    assert_eq!(
        media.iter()
            .map(|attachment| (
                attachment.key().source_ref().clone(),
                attachment.device_slot(),
                attachment.right(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (ResourceRef::parse("Volume/boot").unwrap(), Some(0), RequestedRights::Mutate),
            (ResourceRef::parse("Volume/data").unwrap(), Some(1), RequestedRights::Mutate),
        ]
    );

    let display = projections.display().unwrap();
    assert_eq!(display.kind(), AttachmentKind::Display);
    assert_eq!(display.key().source_ref(), &ResourceRef::parse("Endpoint/display").unwrap());
    assert_eq!(display.right(), RequestedRights::Consume);

    // Every descriptor is also still current against the rows its source
    // decision was evaluated against, so a cached Ready cannot remint it.
    for attachment in projections.attachments() {
        assert!(attachment.evidence().is_current(attachment.evidence().admission().dependencies()));
    }
}

/// The private list cannot be assembled from references the graph never
/// admitted: a source whose own decision admitted a different right, a
/// relationship the source decision was made for another slot, and a Device
/// delivered as a mediated attachment all project nothing at all.
#[test]
fn a_private_list_cannot_select_an_unadmitted_resource() {
    let guest = guest();
    let consumer = test_fixtures::guest_uid();

    let unadmitted_right = full_bindings_without(
        test_fixtures::admitted_with(
            test_fixtures::kvm_request(&guest),
            &kvm_uid(),
            &consumer,
            Some(vec![RequestedRights::Observe]),
            BindingLifecycleState::Admitted,
        ),
        4,
    );
    assert!(matches!(
        unadmitted_right.project(&guest, requirements()),
        Err(MediaAdmissionError::Admission(_))
    ));

    let mediated = test_fixtures::admitted(
        MediaRequest::Kvm(
            DeviceBindingRequest::new(
                ResourceRef::parse("Device/host-kvm").unwrap(),
                guest.clone(),
                slot("acceleration"),
                DeviceFunction::parse(KVM_FUNCTION).unwrap(),
                DeviceClaimRequest::Shared,
                DeviceAttachmentMode::Mediated,
            )
            .unwrap()
        ),
        &kvm_uid(),
        &consumer,
    );
    assert!(matches!(
        full_bindings_without(mediated, 4).project(&guest, requirements()),
        Err(MediaAdmissionError::UnsupportedAccess)
    ));

    let filesystem = test_fixtures::admitted(
        MediaRequest::Media(
            VolumeBindingRequest::new(
                ResourceRef::parse("Volume/boot").unwrap(),
                guest.clone(),
                slot("boot"),
                BoundedToken::parse("root").unwrap(),
                AttachmentAccess::ReadWrite,
                VolumePresentation::filesystem("/mnt/boot").unwrap(),
            )
            .unwrap()
        ),
        &media_uid(0),
        &consumer,
    );
    assert!(matches!(
        full_bindings_without(filesystem, 4).project(&guest, requirements()),
        Err(MediaAdmissionError::UnsupportedAccess)
    ));

    let wrong_purpose = test_fixtures::admitted(
        MediaRequest::Display(
            EndpointBindingRequest::new(
                ResourceRef::parse("Endpoint/display").unwrap(),
                guest.clone(),
                slot("display"),
                EndpointAttachmentKind::Connect,
                BoundedToken::parse("clipboard").unwrap(),
            )
            .unwrap()
        ),
        &display_uid(),
        &consumer,
    );
    assert!(matches!(
        full_bindings_without(wrong_purpose, 4).project(&guest, requirements()),
        Err(MediaAdmissionError::UnsupportedAccess)
    ));
}

fn full_bindings_without(replacement: AdmittedRelationship, skip: u8) -> GuestMediaBindings {
    let guest = guest();
    let consumer = test_fixtures::guest_uid();
    let mut relationships = vec![
        test_fixtures::admitted(test_fixtures::tap_request(&guest), &tap_uid(), &consumer),
        test_fixtures::admitted(
            test_fixtures::media_request(&guest, "boot", 0),
            &media_uid(0),
            &consumer,
        ),
        test_fixtures::admitted(
            test_fixtures::media_request(&guest, "data", 1),
            &media_uid(1),
            &consumer,
        ),
        test_fixtures::admitted(
            test_fixtures::display_request(&guest),
            &display_uid(),
            &consumer,
        ),
    ];
    relationships.insert(skip as usize, replacement);
    GuestMediaBindings::new(test_fixtures::zone(), consumer, relationships)
}

/// Scenario 2 in the projection: a relationship the Guest's spec requires but
/// the graph did not admit, a relationship another Zone or another consumer
/// admitted, and a relationship whose committed rows have moved on.
#[test]
fn missing_or_stale_bindings_produce_no_descriptor_list() {
    let guest = guest();
    let consumer = test_fixtures::guest_uid();

    let absent = GuestMediaBindings::new(
        test_fixtures::zone(),
        consumer.clone(),
        [test_fixtures::admitted(test_fixtures::tap_request(&guest), &tap_uid(), &consumer)],
    );
    assert!(matches!(
        absent.project(&guest, requirements()),
        Err(MediaAdmissionError::MissingBinding)
    ));
    // A Guest whose own spec requires nothing still gets the descriptors its
    // admitted set holds and no others: the tap it declared, not a KVM slot
    // nobody admitted.
    let unrequired = absent
        .project(&guest, SlotRequirements::default())
        .unwrap();
    assert_eq!(unrequired.labels(), ["tap-0"]);
    assert!(unrequired.kvm().is_none());

    let foreign_consumer = GuestMediaBindings::new(
        test_fixtures::zone(),
        consumer.clone(),
        [test_fixtures::admitted(
            test_fixtures::kvm_request(&guest),
            &kvm_uid(),
            &test_fixtures::uid(8),
        )],
    );
    assert!(matches!(
        foreign_consumer.project(&guest, SlotRequirements::default()),
        Err(MediaAdmissionError::ForeignConsumer)
    ));

    let released = GuestMediaBindings::new(
        test_fixtures::zone(),
        consumer.clone(),
        [test_fixtures::admitted_with(
            test_fixtures::kvm_request(&guest),
            &kvm_uid(),
            &consumer,
            None,
            BindingLifecycleState::Released,
        )],
    );
    assert!(matches!(
        released.project(&guest, SlotRequirements::default()),
        Err(MediaAdmissionError::StaleAuthority)
    ));
}

/// Scenario 3: the VMM leg is a view of the Guest's own reservation, and a
/// helper asking for a right the Guest was not admitted for is refused.
#[test]
fn the_vmm_leg_shares_the_guests_reservation_and_refuses_widening() {
    let guest = guest();
    let projections = full_bindings().project(&guest, requirements()).unwrap();
    let legs = projections.legs().unwrap();
    assert_eq!(legs.len(), projections.attachments().len());

    let kvm_leg: &ImplementationLeg = &projections
        .kvm()
        .expect("the acceleration Device was projected")
        .leg()
        .expect("the acceleration Device admits a helper leg")
        .expect("a shared Device admits a helper right");
    assert_eq!(kvm_leg.parent, *projections.kvm().unwrap().key());
    assert_eq!(kvm_leg.source_uid, *projections.kvm().unwrap().key().source_uid());
    assert_eq!(
        kvm_leg.reservation.source_uid(),
        projections.kvm().unwrap().reservation().source_uid()
    );
    assert_eq!(kvm_leg.reservation, *projections.kvm().unwrap().reservation());
    assert_eq!(kvm_leg.rights, RequestedRights::Share);

    // The shared Device is a `Share`; a helper that asks for the arbitrating
    // right is refused before any leg exists for it.
    let kvm = projections.kvm().unwrap();
    assert!(matches!(
        ImplementationLeg::with_rights(kvm, RequestedRights::Exclusive),
        Err(MediaAdmissionError::UnattenuatedRight)
    ));
    assert!(ImplementationLeg::with_rights(kvm, RequestedRights::Consume).is_err());
    assert!(ImplementationLeg::with_rights(kvm, RequestedRights::Share).is_ok());

    // Every leg names its own helper identity and no two legs collide, so a
    // helper cannot claim a leg that belongs to another relationship.
    let identities: std::collections::BTreeSet<&str> =
        legs.iter().map(|leg| leg.identity.as_str()).collect();
    assert_eq!(identities.len(), legs.len());
}

/// The two relationships a source arbitrates as exclusive hold distinct legs,
/// and a read-only media Volume admits no helper leg at all because
/// observation has no attenuated form.
#[test]
fn leg_identity_is_derived_per_relationship_not_per_source() {
    let guest = guest();
    let consumer = test_fixtures::guest_uid();
    let bindings = GuestMediaBindings::new(
        test_fixtures::zone(),
        consumer.clone(),
        [
            test_fixtures::admitted(
                test_fixtures::media_request(&guest, "boot", 0),
                &media_uid(0),
                &consumer,
            ),
            test_fixtures::admitted(
                test_fixtures::media_request(&guest, "data", 1),
                &media_uid(0),
                &consumer,
            ),
        ],
    );
    let projections = bindings
        .project(
            &guest,
            SlotRequirements {
                media: 2,
                ..SlotRequirements::default()
            },
        )
        .unwrap();
    let legs = projections.legs().unwrap();
    assert_eq!(legs.len(), 2);
    assert_ne!(legs[0].identity, legs[1].identity);
    // Both relationships name the same source, so the shared source still
    // arbitrates them: neither leg widened into a second claim.
    assert_eq!(legs[0].source_uid, legs[1].source_uid);

    let read_only = test_fixtures::admitted(
        MediaRequest::Media(
            VolumeBindingRequest::new(
                ResourceRef::parse("Volume/iso").unwrap(),
                guest.clone(),
                slot("iso"),
                BoundedToken::parse("root").unwrap(),
                AttachmentAccess::ReadOnly,
                VolumePresentation::block_device(0).unwrap(),
            )
            .unwrap()
        ),
        &media_uid(0),
        &consumer,
    );
    let read_only = GuestMediaBindings::new(test_fixtures::zone(), consumer, [read_only])
        .project(
            &guest,
            SlotRequirements {
                media: 1,
                ..SlotRequirements::default()
            },
        )
        .unwrap();
    let attachment = read_only.media().next().unwrap();
    assert_eq!(attachment.right(), RequestedRights::Observe);
    assert!(matches!(attachment.leg(), Ok(None)));
    assert!(matches!(
        ImplementationLeg::derive(attachment),
        Err(MediaAdmissionError::UnattenuatedRight)
    ));
}

/// The runtime Volume is an ordinary source: the runner's use of it is an
/// ordinary `VolumeBinding` request naming this row, its named view, and the
/// declared destination.
#[test]
fn the_runtime_volume_realizes_as_a_typed_binding_request() {
    let volume = RuntimeVolumeSpec::new(guest(), "corp", 10 * 1024 * 1024, 1024).unwrap();
    let source = ResourceRef::parse("Volume/media-vm-runtime").unwrap();
    let request = volume
        .runner_request(
            source.clone(),
            ResourceRef::parse("Process/qemu-media").unwrap(),
        )
        .unwrap();
    assert_eq!(request.source_ref(), &source);
    assert!(
        volume
            .runner_request(
                ResourceRef::parse("Device/host-kvm").unwrap(),
                ResourceRef::parse("Process/qemu-media").unwrap(),
            )
            .is_err()
    );
    assert_eq!(request.consumer_ref().resource_type().as_str(), "Process");
    assert_eq!(request.slot().as_str(), RUNTIME_VOLUME_SLOT);
    assert_eq!(request.view().as_str(), "runner");
    assert_eq!(request.access(), AttachmentAccess::ReadWrite);
    assert_eq!(request.presentation().destination(), Some("/run/qemu"));
    assert_eq!(request.requested_rights(), RequestedRights::Mutate);
    // The runtime volume reaches the runner as a mount, so its realization
    // facet is the source-side filesystem presentation - not one of the
    // private-descriptor facets this Provider hands to the VMM. The runner's
    // QMP and serial sockets are therefore the same admitted relationship as
    // any other storage use, not a mount the Process spec hard-coded.
    assert_eq!(
        request.required_facets(),
        &[BindingRealizationFacet::FilesystemPresentation]
    );
}

/// The realization support this Provider declares covers exactly the four
/// attachment classes it realizes, so a Credential delivery or a filesystem
/// presentation it does not hand to a runner cannot pass the shared evaluator.
#[test]
fn the_declared_realization_support_covers_only_what_the_runner_takes() {
    let support = qemu_media_realization_support();
    assert!(support.realizes(BindingRealizationFacet::DeviceAttachment));
    assert!(support.realizes(BindingRealizationFacet::ConsumerDeviceSlot));
    assert!(support.realizes(BindingRealizationFacet::NamespaceInterface));
    assert!(support.realizes(BindingRealizationFacet::EndpointDescriptor));
    assert!(!support.realizes(BindingRealizationFacet::CredentialDelivery));
    assert!(!support.realizes(BindingRealizationFacet::FilesystemPresentation));
}
/// A relationship whose consumer the graph never authorized projects nothing,
/// and the refusal names the enforcing stage the shared evaluator stopped at.
#[test]
fn an_unauthorized_consumer_projects_nothing() {
    let guest = guest();
    let consumer = test_fixtures::guest_uid();
    let request = test_fixtures::kvm_request(&guest);
    let key = request
        .key(test_fixtures::zone(), kvm_uid(), consumer.clone())
        .unwrap();
    let rows = vec![test_fixtures::freshness(key.source_ref(), &kvm_uid())];
    let unauthorized = AdmittedRelationship::new(
        request,
        RelationshipEvidence {
            authorization: BindingAuthorization::absent(),
            source: SourceAdmission::new(
                key.clone(),
                vec![RequestedRights::Share],
                BindingArbitration::Shared,
            )
            .unwrap(),
            support: qemu_media_realization_support(),
            reservation: SourceReservation::new(
                test_fixtures::zone(),
                kvm_uid(),
                BoundedToken::parse("fixture-reservation").unwrap(),
            ),
            dependencies: rows.clone(),
            observed: rows,
            state: BindingLifecycleState::Admitted,
        },
    );
    let error = GuestMediaBindings::new(test_fixtures::zone(), consumer, [unauthorized])
        .project(&guest, SlotRequirements::default())
        .unwrap_err();
    assert!(matches!(
        &error,
        MediaAdmissionError::Admission(refusal)
            if refusal.reason() == RefusalReason::IdentityNotAuthorized
    ));
    assert_eq!(error.stage(), Some(AdmissionStage::Authorize));
}

/// The staging path the unchanged daemon composition still drives projects the
/// same private slot labels the admitted model produces, so converting the
/// caller is a wiring change and not a behaviour change. U34 deletes the
/// declared arm and this comparison with it.
#[test]
fn the_staging_declared_path_projects_the_same_private_slots() {
    let guest = guest();
    let process = build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        [ResourceRef::parse("Network/corp-net").unwrap()],
    )
    .unwrap();
    let declared = d2b_provider_guest_qemu_media::LaunchTicket::declared(
        process.clone(),
        [
            ResourceRef::parse("Volume/boot").unwrap(),
            ResourceRef::parse("Volume/data").unwrap(),
        ],
        Some(ResourceRef::parse("Endpoint/display").unwrap()),
    )
    .unwrap();
    assert_eq!(
        declared.labels(),
        ["kvm", "tap-0", "media-0", "media-1", "display"]
    );

    let consumer = test_fixtures::guest_uid();
    let admitted = GuestMediaBindings::new(
        test_fixtures::zone(),
        consumer.clone(),
        [
            test_fixtures::admitted(
                test_fixtures::kvm_request(&guest),
                &test_fixtures::uid(2),
                &consumer,
            ),
            test_fixtures::admitted(
                test_fixtures::tap_request(&guest),
                &test_fixtures::uid(3),
                &consumer,
            ),
            test_fixtures::admitted(
                test_fixtures::media_request(&guest, "boot", 0),
                &test_fixtures::uid(4),
                &consumer,
            ),
            test_fixtures::admitted(
                test_fixtures::media_request(&guest, "data", 1),
                &test_fixtures::uid(5),
                &consumer,
            ),
            test_fixtures::admitted(
                test_fixtures::display_request(&guest),
                &test_fixtures::uid(6),
                &consumer,
            ),
        ],
    )
    .project(
        &guest,
        SlotRequirements {
            tap: true,
            media: 2,
            display: true,
            ..SlotRequirements::default()
        },
    )
    .unwrap();
    let admitted =
        d2b_provider_guest_qemu_media::LaunchTicket::admitted(process, admitted).unwrap();
    assert_eq!(admitted.labels(), declared.labels());
}
