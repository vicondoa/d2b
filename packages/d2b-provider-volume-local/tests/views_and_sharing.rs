//! Named views, right intersection, and sharing admission.

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::{AttachmentAccess, SourceKind, VolumeSpec};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAuthorization, BindingConsumerKind, BindingKind,
    BindingRealizationFacet, BindingRealizationSupport, BindingRefusal, BindingSlot,
    BindingSupportEntry, ChildBindingRequest, ChildRequestDefaults, ChildSupportCeiling,
    DefaultedSource, DesiredDigest, DesiredRevision, FreshnessTuple, RefusalReason,
    RequestedRights, ResourceRef, ResourceUid, StoreIncarnation, VolumeBindingRequest,
    VolumePresentation, ZoneId,
};
use d2b_provider_volume_local::testing::{ScriptedPort, fixtures};
use d2b_provider_volume_local::{
    SourceReleaseDecision, SourceReleaseObservation, VolumeAdmissionGrant, VolumeConsumerRequest,
    VolumeLocalController, VolumeLocalError, VolumeLocalProfile, admit_access, admit_attachments,
    decide_source_release, normalize_consumer_request, resolve_view,
};

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("valid fixture token")
}

fn spec_with_attachments(attachments: serde_json::Value) -> VolumeSpec {
    serde_json::from_value(serde_json::json!({
        "source": {
            "executionRef": "Host/host-system",
            "settings": { "kind": "local-path", "sourcePolicyId": "state-root" },
        },
        "kind": "durable",
        "layout": [],
        "views": {
            "controller": { "path": "", "rights": ["read", "write", "traverse"] },
            "reader": { "path": "data", "rights": ["read", "traverse"] },
        },
        "attachments": attachments,
    }))
    .expect("conformant fixture Volume spec")
}

fn attachment(mount_path: &str, execution_ref: &str, view: &str, access: &str) -> serde_json::Value {
    serde_json::json!({
        "executionRef": execution_ref,
        "transport": "virtiofs",
        "view": view,
        "access": access,
        "mountPath": mount_path,
    })
}

#[test]
fn a_view_that_is_not_declared_is_rejected() {
    let spec = fixtures::state_volume();
    assert_eq!(
        resolve_view(&spec, &token("absent")).unwrap_err(),
        VolumeLocalError::ViewNotFound
    );
    assert!(resolve_view(&spec, &token("controller")).is_ok());
}

#[test]
fn write_access_requires_the_view_to_grant_the_write_right() {
    let spec = spec_with_attachments(serde_json::json!([]));
    let reader = resolve_view(&spec, &token("reader")).expect("declared view");
    assert!(admit_access(reader, AttachmentAccess::ReadOnly).is_ok());
    assert_eq!(
        admit_access(reader, AttachmentAccess::ReadWrite).unwrap_err(),
        VolumeLocalError::ViewRightsInsufficient
    );
    let controller = resolve_view(&spec, &token("controller")).expect("declared view");
    assert!(admit_access(controller, AttachmentAccess::ReadWrite).is_ok());
}

#[test]
fn many_readers_share_one_volume() {
    let spec = spec_with_attachments(serde_json::json!([
        attachment("/state", "Guest/work-vm", "reader", "read-only"),
        attachment("/docs", "Guest/personal-vm", "reader", "read-only"),
        attachment("/export", "Host/host-system", "controller", "read-write"),
    ]));
    let plans = admit_attachments(&spec, false).expect("admitted");
    assert_eq!(plans.len(), 3);
    assert_eq!(
        plans
            .iter()
            .filter(|plan| plan.access == AttachmentAccess::ReadWrite)
            .count(),
        1
    );
}

#[test]
fn a_second_simultaneous_writer_is_rejected() {
    let spec = spec_with_attachments(serde_json::json!([attachment(
        "/state",
        "Guest/work-vm",
        "controller",
        "read-write"
    ),]));
    assert!(admit_attachments(&spec, false).is_ok());

    // The base contract rejects two `read-write` attachments outright, so
    // the second-writer case reaches the Provider only as `shared-write`.
    let shared = spec_with_attachments(serde_json::json!([
        attachment("/state", "Guest/work-vm", "controller", "read-write"),
        attachment("/docs", "Guest/personal-vm", "controller", "shared-write"),
    ]));
    assert_eq!(
        admit_attachments(&shared, false).unwrap_err(),
        VolumeLocalError::SharedWriteUnsupported
    );
    assert!(admit_attachments(&shared, true).is_ok());
}

#[test]
fn guest_mount_paths_collide_only_within_a_guest() {
    // The base contract already rejects duplicate execution targets, so two
    // attachments from one guest never reach admission through valid specs;
    // the DuplicateMountPath rule below guards the (guest, path) pair for
    // specs arriving through schema-only paths. The same path on different
    // guests stays admitted: mount points live per guest (AE5).
    let cross_guest = spec_with_attachments(serde_json::json!([
        attachment("/state", "Guest/work-vm", "controller", "read-only"),
        attachment("/state", "Guest/personal-vm", "reader", "read-only"),
    ]));
    assert!(admit_attachments(&cross_guest, false).is_ok());

    // Distinct guest mount paths stay admitted.
    let distinct = spec_with_attachments(serde_json::json!([
        attachment("/state", "Guest/work-vm", "controller", "read-write"),
        attachment("/docs", "Guest/personal-vm", "reader", "read-only"),
    ]));
    assert!(admit_attachments(&distinct, false).is_ok());
}
#[test]
fn non_frozen_serving_settings_are_rejected_instead_of_silently_ignored() {
    // Bindings serve the frozen default only: a virtiofs attachment
    // declaring live tuning is rejected with a visible reason.
    let mut tuned = attachment("/state", "Guest/work-vm", "controller", "read-only");
    tuned["settings"] = serde_json::json!({
        "posixAcl": false,
        "xattr": false,
        "cache": "always",
        "inodeFileHandles": "never",
        "threadPoolSize": null,
        "socketGroup": null,
    });
    let spec = spec_with_attachments(serde_json::json!([tuned]));
    let error = admit_attachments(&spec, false).unwrap_err();
    assert_eq!(error, VolumeLocalError::AttachmentSettingsUnsupported);
    assert_eq!(error.code(), "volume-attachment-settings-unsupported");
}

#[test]
fn the_shipped_provider_does_not_declare_shared_write() {
    use d2b_provider_volume_local::VolumeLocalProfile;
    assert!(!VolumeLocalProfile::shipped().supports_shared_write());
}

#[test]
fn every_admitted_attachment_keeps_its_typed_reference_and_view() {
    let spec = fixtures::attached_state_volume();
    let plans = admit_attachments(&spec, false).expect("admitted");
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].execution_ref.to_canonical_string(),
        "Guest/work-vm"
    );
    assert_eq!(plans[0].view.as_str(), "controller");
    assert_eq!(plans[0].access, AttachmentAccess::ReadWrite);
}

// ---------------------------------------------------------------------------
// The canonical source-side admission path.
//
// Every case below is a decision the source makes once, for every consumer
// kind and both presentations, rather than one each consumer's call site
// restates.  They exercise the path through the controller, which is where a
// Provider's own profile - its single-writer rule and its shared-write
// declaration - is applied.
// ---------------------------------------------------------------------------

const ZONE: &str = "work";
const VOLUME_UID: &str = "6f9619ff-8b86-4d01-b42d-00cf4fc964ff";
const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const HOST_UID: &str = "223e4567-e89b-42d3-a456-426614174001";
const PROCESS_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
const EPHEMERAL_UID: &str = "423e4567-e89b-42d3-a456-426614174003";

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone identifier")
}

fn volume_uid() -> ResourceUid {
    ResourceUid::parse(VOLUME_UID).expect("canonical resource uid")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("registered resource reference")
}

fn slot(value: &str) -> BindingSlot {
    BindingSlot::parse(value).expect("bounded slot token")
}

fn freshness(resource: &ResourceRef, uid: ResourceUid) -> FreshnessTuple {
    FreshnessTuple::new(
        zone(),
        StoreIncarnation::parse("store-one").expect("store incarnation"),
        resource.clone(),
        uid,
        DesiredRevision::INITIAL,
        DesiredDigest::of(resource.to_canonical_string().as_bytes()),
    )
}

/// The dependency fence every admission below is evaluated against: the
/// source itself plus the consumers it names.
fn fence(consumers: &[(&str, ResourceUid)]) -> Vec<FreshnessTuple> {
    let volume_ref = reference("Volume/state");
    let mut fence = vec![freshness(&volume_ref, volume_uid())];
    fence.extend(
        consumers
            .iter()
            .map(|(name, uid)| freshness(&reference(name), uid.clone())),
    );
    fence
}

/// The authorization evidence every ordinary case below carries.
static AUTHORIZED: std::sync::LazyLock<BindingAuthorization> =
    std::sync::LazyLock::new(BindingAuthorization::granted);
/// The absence of authorization evidence, which is refused before the source
/// decides anything.
static ABSENT_AUTHORIZATION: std::sync::LazyLock<BindingAuthorization> =
    std::sync::LazyLock::new(BindingAuthorization::absent);

/// A Volume declaring a writable root view and a read-only `data` subtree.
fn two_view_volume() -> VolumeSpec {
    serde_json::from_value(serde_json::json!({
        "source": {
            "executionRef": "Host/host-system",
            "settings": { "kind": "local-path", "sourcePolicyId": "state-root" },
        },
        "kind": "durable",
        "layout": [],
        "views": {
            "controller": { "path": "", "rights": ["read", "write", "traverse"] },
            "reader": { "path": "data", "rights": ["read", "traverse"] },
        },
    }))
    .expect("conformant fixture Volume spec")
}

fn filesystem(destination: &str) -> VolumePresentation {
    VolumePresentation::filesystem(destination).expect("consumer destination")
}

fn request(
    consumer: &str,
    consumer_slot: &str,
    view: &str,
    access: AttachmentAccess,
    presentation: VolumePresentation,
) -> VolumeBindingRequest {
    VolumeBindingRequest::new(
        reference("Volume/state"),
        reference(consumer),
        slot(consumer_slot),
        BoundedToken::parse(view).expect("view token"),
        access,
        presentation,
    )
    .expect("canonical consumer request")
}

fn full_support() -> BindingRealizationSupport {
    BindingRealizationSupport::new(vec![
        BindingRealizationFacet::FilesystemPresentation,
        BindingRealizationFacet::ConsumerDeviceSlot,
    ])
    .expect("support set")
}

fn grant<'a>(
    support: &'a BindingRealizationSupport,
    authorization: &'a BindingAuthorization,
    dependencies: &'a [FreshnessTuple],
) -> VolumeAdmissionGrant<'a> {
    VolumeAdmissionGrant::new(support, authorization, dependencies)
}

/// A controller whose ports are never touched: the admission path is pure, so
/// the scripted doubles only satisfy the constructor.
fn shipped_controller(
    ports: &ScriptedPort,
) -> VolumeLocalController<&ScriptedPort, &ScriptedPort> {
    VolumeLocalController::new(VolumeLocalProfile::shipped(), ports, ports)
}

fn shared_write_controller(
    ports: &ScriptedPort,
) -> VolumeLocalController<&ScriptedPort, &ScriptedPort> {
    let profile = VolumeLocalProfile::new(
        BoundedToken::parse("volume-local").expect("frozen provider name"),
        [SourceKind::LocalPath].into_iter().collect(),
        true,
    )
    .expect("a profile declaring a source kind");
    VolumeLocalController::new(profile, ports, ports)
}

#[test]
fn one_source_side_path_admits_every_consumer_kind_and_presentation() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let support = full_support();
    let requests = [
        VolumeConsumerRequest::new(
            ResourceUid::parse(GUEST_UID).expect("uid"),
            request(
                "Guest/work-vm",
                "state",
                "controller",
                AttachmentAccess::ReadOnly,
                filesystem("/state"),
            ),
        ),
        VolumeConsumerRequest::new(
            ResourceUid::parse(EPHEMERAL_UID).expect("uid"),
            request(
                "EphemeralProcess/task",
                "scratch",
                "reader",
                AttachmentAccess::ReadOnly,
                filesystem("/task/input"),
            ),
        ),
        VolumeConsumerRequest::new(
            ResourceUid::parse(HOST_UID).expect("uid"),
            request(
                "Host/host-system",
                "host-state",
                "reader",
                AttachmentAccess::ReadOnly,
                VolumePresentation::block_device(2).expect("device slot"),
            ),
        ),
        VolumeConsumerRequest::new(
            ResourceUid::parse(PROCESS_UID).expect("uid"),
            request(
                "Process/worker",
                "work",
                "controller",
                AttachmentAccess::ReadWrite,
                filesystem("/srv/work"),
            ),
        ),
    ];
    let dependencies = fence(&[
        ("Guest/work-vm", ResourceUid::parse(GUEST_UID).expect("uid")),
        (
            "EphemeralProcess/task",
            ResourceUid::parse(EPHEMERAL_UID).expect("uid"),
        ),
        ("Host/host-system", ResourceUid::parse(HOST_UID).expect("uid")),
        ("Process/worker", ResourceUid::parse(PROCESS_UID).expect("uid")),
    ]);
    let admitted = shipped_controller(&ports)
        .admit_bindings(
            &zone(),
            &reference("Volume/state"),
            &volume_uid(),
            &spec,
            &grant(&support, &AUTHORIZED, &dependencies),
            &requests,
        )
        .expect("one path admits every consumer kind and presentation");

    assert_eq!(admitted.len(), requests.len());
    // Every admitted relationship kept the exact consumer, view, and
    // destination its request declared.
    for (relationship, input) in admitted.iter().zip(&requests) {
        let request = input.request();
        assert_eq!(
            relationship.request().consumer_ref(),
            request.consumer_ref()
        );
        assert_eq!(relationship.request().view().as_str(), request.view().as_str());
        assert_eq!(
            relationship.request().presentation(),
            request.presentation()
        );
        assert_eq!(relationship.key().consumer_uid(), input.consumer_uid());
    }
    // Exactly one relationship holds the writer, and it is the one that asked.
    assert_eq!(
        admitted
            .iter()
            .filter(|relationship| relationship.holds_writer())
            .count(),
        1
    );
    assert_eq!(
        admitted
            .iter()
            .find(|relationship| relationship.holds_writer())
            .map(|relationship| relationship.request().consumer_ref().to_canonical_string()),
        Some("Process/worker".to_owned())
    );
    // Every consumer kind a Volume binding delivers to was exercised.
    for kind in BindingConsumerKind::ALL {
        assert!(
            admitted.iter().any(|relationship| {
                BindingConsumerKind::from_resource_type(
                    relationship.request().consumer_ref().resource_type().as_str(),
                ) == Some(kind)
            }),
            "{kind:?} was not admitted through the source-side path"
        );
    }
}

#[test]
fn a_view_that_grants_no_write_refuses_a_mutating_request() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let support = full_support();
    let dependencies = fence(&[("Process/worker", ResourceUid::parse(PROCESS_UID).expect("uid"))]);
    let read_only = [VolumeConsumerRequest::new(
        ResourceUid::parse(PROCESS_UID).expect("uid"),
        request(
            "Process/worker",
            "work",
            "reader",
            AttachmentAccess::ReadOnly,
            filesystem("/srv/read"),
        ),
    )];
    assert!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &read_only,
            )
            .is_ok()
    );

    // The same consumer, the same view, the same destination: asking to write
    // it is refused rather than widened.
    let mutating = [VolumeConsumerRequest::new(
        ResourceUid::parse(PROCESS_UID).expect("uid"),
        request(
            "Process/worker",
            "work",
            "reader",
            AttachmentAccess::ReadWrite,
            filesystem("/srv/read"),
        ),
    )];
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &mutating,
            )
            .unwrap_err(),
        BindingRefusal::new(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused)
    );
}

#[test]
fn a_second_consumer_cannot_take_a_writer_the_first_already_holds() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let support = full_support();
    let dependencies = fence(&[
        ("Guest/work-vm", ResourceUid::parse(GUEST_UID).expect("uid")),
        ("Process/worker", ResourceUid::parse(PROCESS_UID).expect("uid")),
    ]);
    // A Guest takes the writer; a Process then asks to write the same Volume
    // from a different consumer and a different destination.  The writer
    // belongs to the source, so the two are arbitrated against each other.
    let competing = [
        VolumeConsumerRequest::new(
            ResourceUid::parse(GUEST_UID).expect("uid"),
            request(
                "Guest/work-vm",
                "state",
                "controller",
                AttachmentAccess::ReadWrite,
                filesystem("/state"),
            ),
        ),
        VolumeConsumerRequest::new(
            ResourceUid::parse(PROCESS_UID).expect("uid"),
            request(
                "Process/worker",
                "work",
                "controller",
                AttachmentAccess::ReadWrite,
                filesystem("/srv/work"),
            ),
        ),
    ];
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &competing,
            )
            .unwrap_err(),
        BindingRefusal::new(AdmissionStage::Reserve, RefusalReason::ConflictingDeclaration)
    );

    // A read-only consumer was never competing for the writer.
    let readers = [
        VolumeConsumerRequest::new(
            ResourceUid::parse(GUEST_UID).expect("uid"),
            request(
                "Guest/work-vm",
                "state",
                "controller",
                AttachmentAccess::ReadWrite,
                filesystem("/state"),
            ),
        ),
        VolumeConsumerRequest::new(
            ResourceUid::parse(PROCESS_UID).expect("uid"),
            request(
                "Process/worker",
                "work",
                "reader",
                AttachmentAccess::ReadOnly,
                filesystem("/srv/docs"),
            ),
        ),
    ];
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &readers,
            )
            .expect("a reader never competes for the writer")
            .len(),
        2
    );
}

#[test]
fn shared_write_is_refused_by_a_profile_that_does_not_declare_it() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let support = full_support();
    let dependencies = fence(&[("Guest/work-vm", ResourceUid::parse(GUEST_UID).expect("uid"))]);
    let shared = [VolumeConsumerRequest::new(
        ResourceUid::parse(GUEST_UID).expect("uid"),
        request(
            "Guest/work-vm",
            "state",
            "controller",
            AttachmentAccess::SharedWrite,
            filesystem("/state"),
        ),
    )];
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &shared,
            )
            .unwrap_err(),
        BindingRefusal::new(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused)
    );
    assert_eq!(
        shared_write_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &shared,
            )
            .expect("a profile that declares shared write admits it")
            .len(),
        1
    );
}

#[test]
fn a_realization_that_cannot_enforce_the_presentation_is_refused() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let dependencies = fence(&[("Guest/work-vm", ResourceUid::parse(GUEST_UID).expect("uid"))]);
    let block = [VolumeConsumerRequest::new(
        ResourceUid::parse(GUEST_UID).expect("uid"),
        request(
            "Guest/work-vm",
            "state",
            "controller",
            AttachmentAccess::ReadOnly,
            VolumePresentation::block_device(0).expect("device slot"),
        ),
    )];
    // The selected realization serves filesystems and claims no device slot,
    // so a block presentation is refused rather than silently mounted.
    let filesystem_only =
        BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
            .expect("support set");
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&filesystem_only, &AUTHORIZED, &dependencies),
                &block,
            )
            .unwrap_err(),
        BindingRefusal::new(
            AdmissionStage::Prepare,
            RefusalReason::MandatoryFacetUnsupported
        )
    );
    let support = full_support();
    assert!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &block,
            )
            .is_ok()
    );
}

#[test]
fn an_unauthorized_request_is_refused_before_the_source_decides() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let support = full_support();
    let dependencies = fence(&[("Process/worker", ResourceUid::parse(PROCESS_UID).expect("uid"))]);
    let requests = [VolumeConsumerRequest::new(
        ResourceUid::parse(PROCESS_UID).expect("uid"),
        request(
            "Process/worker",
            "work",
            "controller",
            AttachmentAccess::ReadWrite,
            filesystem("/srv/work"),
        ),
    )];
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &VolumeAdmissionGrant::new(&support, &ABSENT_AUTHORIZATION, &dependencies),
                &requests,
            )
            .unwrap_err(),
        BindingRefusal::new(
            AdmissionStage::Authorize,
            RefusalReason::IdentityNotAuthorized
        )
    );
}

#[test]
fn an_admitted_view_presents_its_declared_subdirectory() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let support = full_support();
    let dependencies = fence(&[
        ("Guest/work-vm", ResourceUid::parse(GUEST_UID).expect("uid")),
        ("Process/worker", ResourceUid::parse(PROCESS_UID).expect("uid")),
    ]);
    let requests = [
        VolumeConsumerRequest::new(
            ResourceUid::parse(GUEST_UID).expect("uid"),
            request(
                "Guest/work-vm",
                "state",
                "reader",
                AttachmentAccess::ReadOnly,
                filesystem("/state"),
            ),
        ),
        VolumeConsumerRequest::new(
            ResourceUid::parse(PROCESS_UID).expect("uid"),
            request(
                "Process/worker",
                "work",
                "controller",
                AttachmentAccess::ReadOnly,
                filesystem("/srv/work"),
            ),
        ),
    ];
    let admitted = shipped_controller(&ports)
        .admit_bindings(
            &zone(),
            &reference("Volume/state"),
            &volume_uid(),
            &spec,
            &grant(&support, &AUTHORIZED, &dependencies),
            &requests,
        )
        .expect("admitted");
    // The reader view declares `data`, so the source side of that
    // relationship is `data` - never the Volume root standing in for it.
    assert_eq!(admitted[0].view_subdirectory(), "data");
    assert_eq!(admitted[1].view_subdirectory(), "");
}

#[test]
fn one_consumer_cannot_claim_one_destination_twice() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let support = full_support();
    let dependencies = fence(&[
        ("Guest/work-vm", ResourceUid::parse(GUEST_UID).expect("uid")),
        ("Process/worker", ResourceUid::parse(PROCESS_UID).expect("uid")),
    ]);
    let colliding = [
        VolumeConsumerRequest::new(
            ResourceUid::parse(PROCESS_UID).expect("uid"),
            request(
                "Process/worker",
                "work",
                "controller",
                AttachmentAccess::ReadOnly,
                filesystem("/srv/work"),
            ),
        ),
        VolumeConsumerRequest::new(
            ResourceUid::parse(PROCESS_UID).expect("uid"),
            request(
                "Process/worker",
                "mirror",
                "reader",
                AttachmentAccess::ReadOnly,
                filesystem("/srv/work"),
            ),
        ),
    ];
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &colliding,
            )
            .unwrap_err(),
        BindingRefusal::new(AdmissionStage::Admit, RefusalReason::ConflictingDeclaration)
    );

    // The same destination on a different consumer is not a collision: a
    // destination is consumer-local.
    let per_consumer = [
        VolumeConsumerRequest::new(
            ResourceUid::parse(GUEST_UID).expect("uid"),
            request(
                "Guest/work-vm",
                "state",
                "controller",
                AttachmentAccess::ReadOnly,
                filesystem("/srv/work"),
            ),
        ),
        VolumeConsumerRequest::new(
            ResourceUid::parse(PROCESS_UID).expect("uid"),
            request(
                "Process/worker",
                "work",
                "controller",
                AttachmentAccess::ReadOnly,
                filesystem("/srv/work"),
            ),
        ),
    ];
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &per_consumer,
            )
            .expect("destinations are consumer-local")
            .len(),
        2
    );
}

#[test]
fn a_parent_default_shapes_only_the_child_it_names() {
    let defaults = ChildRequestDefaults::new(
        reference("Process/worker"),
        DefaultedSource::new(
            BindingKind::Volume,
            reference("Volume/state"),
            Some(BoundedToken::parse("reader").expect("view token")),
        )
        .expect("defaulted source"),
    )
    .expect("child defaults");
    let unnamed = ChildBindingRequest::new(reference("Process/worker"), BindingKind::Volume)
        .expect("draft for a declared consumer");
    let normalized = normalize_consumer_request(
        &unnamed,
        None,
        Some(&defaults),
        slot("work"),
        filesystem("/srv/work"),
    )
    .expect("the default fills the unnamed child's request");
    assert_eq!(normalized.source_ref(), &reference("Volume/state"));
    assert_eq!(normalized.view().as_str(), "reader");
    // The default supplies a source and a view; the right falls back to the
    // kind's own default, which for a Volume is observation.
    assert_eq!(normalized.requested_rights(), RequestedRights::Observe);

    // A different child cannot borrow them.
    let other = ChildBindingRequest::new(reference("Process/other"), BindingKind::Volume)
        .expect("draft for another consumer");
    assert_eq!(
        normalize_consumer_request(
            &other,
            None,
            Some(&defaults),
            slot("work"),
            filesystem("/srv/work"),
        )
        .unwrap_err(),
        BindingRefusal::new(AdmissionStage::Authorize, RefusalReason::ConflictingDeclaration)
    );

    // A child that declared its own source keeps it: a default fills an
    // unset field, never an overriding one.
    let declaring = ChildBindingRequest::new(reference("Process/worker"), BindingKind::Volume)
        .expect("draft for a declared consumer")
        .declaring(
            reference("Volume/other"),
            Some(BoundedToken::parse("controller").expect("view token")),
            RequestedRights::Mutate,
        )
        .expect("declared request");
    let kept = normalize_consumer_request(
        &declaring,
        None,
        Some(&defaults),
        slot("work"),
        filesystem("/srv/work"),
    )
    .expect("normalization keeps the child's own declaration");
    assert_eq!(kept.source_ref(), &reference("Volume/other"));
    assert_eq!(kept.requested_rights(), RequestedRights::Mutate);
}

#[test]
fn a_child_support_ceiling_bounds_what_a_child_may_request() {
    let observe_only = ChildSupportCeiling::new(vec![
        BindingSupportEntry::new(BindingKind::Volume, vec![RequestedRights::Observe])
            .expect("support entry"),
    ])
    .expect("child support ceiling");
    let mutating = ChildBindingRequest::new(reference("Process/worker"), BindingKind::Volume)
        .expect("draft for a declared consumer")
        .declaring(
            reference("Volume/state"),
            Some(BoundedToken::parse("controller").expect("view token")),
            RequestedRights::Mutate,
        )
        .expect("declared request");
    // A ceiling that lists only Observe does not admit a mutating child
    // request, even though the child's own declaration is well formed.
    assert_eq!(
        normalize_consumer_request(
            &mutating,
            Some(&observe_only),
            None,
            slot("work"),
            filesystem("/srv/work"),
        )
        .unwrap_err(),
        BindingRefusal::new(AdmissionStage::Authorize, RefusalReason::TargetSupportMissing)
    );
    let write_ceiling = ChildSupportCeiling::new(vec![
        BindingSupportEntry::new(
            BindingKind::Volume,
            vec![RequestedRights::Observe, RequestedRights::Mutate],
        )
        .expect("support entry"),
    ])
    .expect("child support ceiling");
    assert!(
        normalize_consumer_request(
            &mutating,
            Some(&write_ceiling),
            None,
            slot("work"),
            filesystem("/srv/work"),
        )
        .is_ok()
    );
}

#[test]
fn a_draft_that_names_no_view_is_refused_rather_than_defaulted_to_the_root() {
    let draft = ChildBindingRequest::new(reference("Process/worker"), BindingKind::Volume)
        .expect("draft for a declared consumer")
        .declaring(
            reference("Volume/state"),
            None,
            RequestedRights::Observe,
        )
        .expect("declared request");
    assert_eq!(
        normalize_consumer_request(&draft, None, None, slot("work"), filesystem("/srv/work"))
            .unwrap_err(),
        BindingRefusal::new(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused)
    );
}

#[test]
fn releasing_the_last_consumer_retains_a_shared_source() {
    // The last consumer released and nothing else happened: the source is
    // retained, which is what lets the next consumer be admitted at all.
    assert_eq!(
        decide_source_release(SourceReleaseObservation::new(0, false, false)),
        SourceReleaseDecision::RetainSharedSource
    );
    // A consumer still holds it, so there is nothing to release.
    assert_eq!(
        decide_source_release(SourceReleaseObservation::new(1, false, false)),
        SourceReleaseDecision::ConsumersRemain { live_consumers: 1 }
    );
    // Deletion requested for a source this Provider does not own is still
    // not this Provider's decision.
    assert_eq!(
        decide_source_release(SourceReleaseObservation::new(0, true, true)),
        SourceReleaseDecision::RetainSharedSource
    );
    // Only an explicit deletion request for an owned source reaches cleanup.
    assert_eq!(
        decide_source_release(SourceReleaseObservation::new(0, true, false)),
        SourceReleaseDecision::DeleteSource
    );
}

#[test]
fn a_request_for_another_source_is_refused() {
    let ports = ScriptedPort::empty();
    let spec = two_view_volume();
    let support = full_support();
    let dependencies = fence(&[("Process/worker", ResourceUid::parse(PROCESS_UID).expect("uid"))]);
    let requests = [VolumeConsumerRequest::new(
        ResourceUid::parse(PROCESS_UID).expect("uid"),
        VolumeBindingRequest::new(
            reference("Volume/elsewhere"),
            reference("Process/worker"),
            slot("work"),
            BoundedToken::parse("reader").expect("view token"),
            AttachmentAccess::ReadOnly,
            filesystem("/srv/work"),
        )
        .expect("canonical consumer request"),
    )];
    assert_eq!(
        shipped_controller(&ports)
            .admit_bindings(
                &zone(),
                &reference("Volume/state"),
                &volume_uid(),
                &spec,
                &grant(&support, &AUTHORIZED, &dependencies),
                &requests,
            )
            .unwrap_err(),
        BindingRefusal::new(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused)
    );
}
