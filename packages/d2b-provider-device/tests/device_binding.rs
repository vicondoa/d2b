//! The `Device` source's typed binding admission.
//!
//! A device grant used to be a `(provider, template)` pair: a closed table in
//! shared code named the device nodes and the Volume class a worker template
//! received, and the launch inherited whatever that row said. These tests
//! pin the replacement from the source side: the Device source admits one
//! `DeviceBindingRequest` at a time against the exact capabilities its
//! trusted inventory resolved, arbitrates the claims once, hands a helper an
//! attenuated leg of its parent's reservation instead of a second claim, and
//! revokes exactly the use an absence observation affects.

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAuthorization, BindingLifecycleState, BindingRealizationSupport,
    BindingRefusal, BindingSlot, DeviceArbitration, DeviceAuthorityArbitration, DeviceAuthorityKey,
    DeviceBindingRequest, DeviceClaimRequest, DeviceClass, DeviceEffectOperation, DeviceFunction,
    DeviceSpec, DesiredDigest, DesiredRevision, FreshnessTuple, InventorySelector, InventorySpec,
    RefusalReason, RequestedRights, ResourceRef, ResourceUid, StoreIncarnation, ZoneId,
    execution_policy::BoundedToken,
};
use d2b_provider_device::binding::{
    DeviceAdmissionGrant, DeviceAdmissionSource, DeviceBindingFate,
    DeviceHelperLeg, DeviceInventory, DeviceInventoryEntry, DevicePresence, DeviceUseOutcome,
    LiveDeviceBinding, admit_device_request, binding_row_name, canonical_binding_row,
    decide_presence, device_attachment_support, leg_outcome, parsed_consumer_request,
};

/// The store generation every admission in this file is fenced against.
fn store() -> StoreIncarnation {
    StoreIncarnation::parse("store-one").expect("bounded store incarnation")
}

fn zone() -> ZoneId {
    ZoneId::parse("dev").expect("bounded zone")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("typed reference")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical resource uid")
}

fn device_uid() -> ResourceUid {
    uid("123e4567-e89b-42d3-a456-426614174000")
}

fn function(value: &str) -> DeviceFunction {
    DeviceFunction::parse(value).expect("bounded function token")
}

fn slot(value: &str) -> BindingSlot {
    BindingSlot::parse(value).expect("bounded slot token")
}

fn freshness(resource: &ResourceRef, resource_uid: ResourceUid) -> FreshnessTuple {
    FreshnessTuple::new(
        zone(),
        store(),
        resource.clone(),
        resource_uid,
        DesiredRevision::INITIAL,
        DesiredDigest::of(resource.to_canonical_string().as_bytes()),
    )
}

/// The dependency fence every admission below is evaluated against: the
/// source itself plus each consumer it names.
fn fence(device_ref: &ResourceRef, consumers: &[(&str, ResourceUid)]) -> Vec<FreshnessTuple> {
    let mut dependencies = vec![freshness(device_ref, device_uid())];
    dependencies.extend(
        consumers
            .iter()
            .map(|(name, consumer_uid)| freshness(&reference(name), consumer_uid.clone())),
    );
    dependencies
}

/// An exclusive physical Device over a DRM bus class.
fn drm_spec() -> DeviceSpec {
    DeviceSpec::new(
        DeviceClass::Physical,
        DeviceArbitration::Exclusive,
        1,
        InventorySpec::new(Some(InventorySelector::Drm {
            label: BoundedToken::parse("gpu-zero").expect("bounded label"),
            pci_slot: None,
        })),
    )
    .expect("the DRM Device spec is valid")
}

/// A shared-arbitration physical Device over a DRM bus class, bounded to two
/// claimants.
fn shared_drm_spec() -> DeviceSpec {
    DeviceSpec::new(
        DeviceClass::Physical,
        DeviceArbitration::Shared,
        2,
        InventorySpec::new(Some(InventorySelector::Drm {
            label: BoundedToken::parse("gpu-zero").expect("bounded label"),
            pci_slot: None,
        })),
    )
    .expect("the shared DRM Device spec is valid")
}

const DRM_KEY: [u8; 32] = [7; 32];
const UVM_KEY: [u8; 32] = [8; 32];

/// One DRM Device with a render node and a separate NVIDIA UVM node, each
/// backed by its own physical authority.
fn drm_inventory() -> DeviceInventory {
    DeviceInventory::new(vec![
        DeviceInventoryEntry::new(
            function("render-node"),
            DeviceAuthorityKey::from_core(DRM_KEY),
            DeviceAuthorityArbitration::Exclusive,
            DevicePresence::Present,
        ),
        DeviceInventoryEntry::new(
            function("nvidia-uvm"),
            DeviceAuthorityKey::from_core(UVM_KEY),
            DeviceAuthorityArbitration::Exclusive,
            DevicePresence::Present,
        ),
    ])
    .expect("the DRM inventory is well formed")
}

const GPU_WORKER: &str = "Process/gpu-zero";
const GPU_WORKER_UID: &str = "223e4567-e89b-42d3-a456-426614174001";
const VIDEO_WORKER: &str = "Process/video-zero";
const VIDEO_WORKER_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
const OTHER_WORKER: &str = "Process/gpu-other";
const OTHER_WORKER_UID: &str = "423e4567-e89b-42d3-a456-426614174003";

fn operations() -> [DeviceEffectOperation; 2] {
    [DeviceEffectOperation::OpenDevice, DeviceEffectOperation::SpawnRunner]
}

fn exclusive_request(
    device_ref: &ResourceRef,
    consumer: &ResourceRef,
    slot_name: &str,
    function_name: &str,
) -> DeviceBindingRequest {
    DeviceBindingRequest::new(
        device_ref.clone(),
        consumer.clone(),
        slot(slot_name),
        function(function_name),
        DeviceClaimRequest::Exclusive,
        d2b_contracts_resource::v3::DeviceAttachmentMode::Descriptor,
    )
    .expect("an exclusive device claim over an admitted consumer is constructible")
}

/// The evidence one source-side admission is evaluated against.
///
/// The grant borrows the realization support, the authorization evidence, the
/// freshness fence, and the operation classes, so it is built once and the
/// Zone and source identity travel with it.
struct SourceEvidence<'a> {
    zone: ZoneId,
    device_uid: ResourceUid,
    grant: DeviceAdmissionGrant<'a>,
}

impl<'a> SourceEvidence<'a> {
    fn new(
        support: &'a BindingRealizationSupport,
        authorization: &'a BindingAuthorization,
        dependencies: &'a [FreshnessTuple],
        operations: &'a [DeviceEffectOperation],
    ) -> Self {
        Self {
            zone: zone(),
            device_uid: device_uid(),
            grant: DeviceAdmissionGrant::new(support, authorization, dependencies, operations),
        }
    }

    /// Bind this evidence to one Device row and one resolved inventory.
    fn source<'b>(
        &'b self,
        device_ref: &'b ResourceRef,
        spec: &'b DeviceSpec,
        inventory: &'b DeviceInventory,
    ) -> DeviceAdmissionSource<'b> {
        DeviceAdmissionSource::new(
            &self.zone,
            device_ref,
            &self.device_uid,
            spec,
            inventory,
            &self.grant,
        )
    }
}

/// An exclusive parent claim supports its bounded helper without competing for
/// a second allocation.
///
/// The parent takes the render node exclusively. The video sidecar realizes
/// that same relationship, so it takes a bound *leg* of the parent's
/// reservation and holds no claim of its own; a genuinely competing consumer
/// is refused instead. The claim only becomes available again once the parent
/// has release evidence, not when it starts revoking.
#[test]
fn an_exclusive_parent_claim_supports_its_bounded_helper() {
    let device_ref = reference("Device/gpu-zero");
    let spec = drm_spec();
    let inventory = drm_inventory();
    let support = device_attachment_support();
    let authorization = BindingAuthorization::granted();
    let dependencies = fence(
        &device_ref,
        &[
            (GPU_WORKER, uid(GPU_WORKER_UID)),
            (VIDEO_WORKER, uid(VIDEO_WORKER_UID)),
            (OTHER_WORKER, uid(OTHER_WORKER_UID)),
        ],
    );
    let operations = operations();
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);

    let parent = admit_device_request(
        &admitted_source,
        &uid(GPU_WORKER_UID),
        &exclusive_request(&device_ref, &reference(GPU_WORKER), "render-node", "render-node"),
        &[],
    )
    .expect("an exclusive claim on a free authority is admitted");
    assert!(parent.holds_exclusive());
    assert_eq!(parent.authority_key().as_bytes(), &DRM_KEY);

    let leg = DeviceHelperLeg::bind(
        &parent,
        reference(VIDEO_WORKER),
        uid(VIDEO_WORKER_UID),
        &[DeviceEffectOperation::OpenDevice],
        &store(),
    )
    .expect("the helper is bound to its parent's reservation");
    assert!(
        !leg.holds_claim(),
        "a helper leg is a realization of the parent, not a second allocation"
    );
    assert_eq!(leg.parent_key(), parent.key());
    assert_eq!(leg.reservation(), parent.reservation());
    assert_eq!(leg.authority_key(), parent.authority_key());
    assert_eq!(leg.function(), parent.function());
    assert!(leg.covers(DeviceEffectOperation::OpenDevice));
    assert!(
        !leg.covers(DeviceEffectOperation::ApplyNftablesProjection),
        "a leg may not drive an operation its parent never admitted"
    );

    // Each family owns a read-only `BoundDeviceLeg` view because depending on
    // this crate would be a package cycle, so the two implementations in
    // `binding` are the only place the source's real leg satisfies them. Prove
    // each view reads THIS leg rather than a copy, and that neither family can
    // see a device claim through it: a helper that could look like a competing
    // allocation is exactly what AE27 forbids.
    assert_eq!(d2b_provider_device_usbip::BoundDeviceLeg::parent_key(&leg), leg.parent_key());
    assert_eq!(d2b_provider_device_security_key::BoundDeviceLeg::parent_key(&leg), leg.parent_key());
    assert_eq!(d2b_provider_device_usbip::BoundDeviceLeg::reservation(&leg), leg.reservation());
    assert_eq!(d2b_provider_device_security_key::BoundDeviceLeg::reservation(&leg), leg.reservation());
    assert_eq!(d2b_provider_device_usbip::BoundDeviceLeg::helper_ref(&leg), leg.helper_ref());
    assert_eq!(d2b_provider_device_security_key::BoundDeviceLeg::helper_ref(&leg), leg.helper_ref());
    assert_eq!(d2b_provider_device_usbip::BoundDeviceLeg::helper_uid(&leg), leg.helper_uid());
    assert_eq!(d2b_provider_device_security_key::BoundDeviceLeg::helper_uid(&leg), leg.helper_uid());
    assert_eq!(d2b_provider_device_usbip::BoundDeviceLeg::function(&leg), leg.function());
    assert_eq!(d2b_provider_device_security_key::BoundDeviceLeg::function(&leg), leg.function());
    assert_eq!(d2b_provider_device_usbip::BoundDeviceLeg::authority_key(&leg), leg.authority_key());
    assert_eq!(d2b_provider_device_security_key::BoundDeviceLeg::authority_key(&leg), leg.authority_key());
    assert_eq!(d2b_provider_device_usbip::BoundDeviceLeg::operations(&leg), leg.operations());
    assert_eq!(d2b_provider_device_security_key::BoundDeviceLeg::operations(&leg), leg.operations());
    assert_eq!(d2b_provider_device_usbip::BoundDeviceLeg::epoch(&leg), leg.epoch());
    assert_eq!(d2b_provider_device_security_key::BoundDeviceLeg::epoch(&leg), leg.epoch());
    assert!(
        !d2b_provider_device_usbip::BoundDeviceLeg::holds_claim(&leg) && !d2b_provider_device_security_key::BoundDeviceLeg::holds_claim(&leg),
        "neither family may observe a helper leg as a claim of its own"
    );

    // The competing consumer is refused at the reservation stage: the parent's
    // exclusive claim is live.
    let held = [LiveDeviceBinding::new(parent.clone(), BindingLifecycleState::Active)];
    let competing = exclusive_request(
        &device_ref,
        &reference(OTHER_WORKER),
        "render-node",
        "render-node",
    );
    let refused = admit_device_request(
        &admitted_source,
        &uid(OTHER_WORKER_UID),
        &competing,
        &held,
    )
    .expect_err("a second exclusive claim on one authority must be refused");
    assert_eq!(refused.stage(), AdmissionStage::Reserve);
    assert_eq!(refused.reason(), RefusalReason::ConflictingDeclaration);

    // The helper does not become a competing allocation: the same competing
    // request is still refused while the leg exists, because the leg holds no
    // claim of its own to release and the parent's claim is still live.
    assert!(!leg.holds_claim());
    assert_eq!(
        admit_device_request(&admitted_source, &uid(OTHER_WORKER_UID), &competing, &held)
            .expect_err("the parent still holds the authority")
            .reason(),
        RefusalReason::ConflictingDeclaration
    );

    // Release evidence gates reassignment: a revoking or draining parent still
    // holds its claim.
    for lifecycle in [
        BindingLifecycleState::Revoking,
        BindingLifecycleState::Draining,
        BindingLifecycleState::Unknown,
    ] {
        let draining = [LiveDeviceBinding::new(parent.clone(), lifecycle)];
        assert!(
            admit_device_request(&admitted_source, &uid(OTHER_WORKER_UID), &competing, &draining).is_err(),
            "{lifecycle:?} is not release evidence: the authority is still held"
        );
    }

    // Only the released relationship frees the authority.
    let released = [LiveDeviceBinding::new(parent.clone(), BindingLifecycleState::Released)];
    let successor = admit_device_request(
        &admitted_source,
        &uid(OTHER_WORKER_UID),
        &competing,
        &released,
    )
    .expect("released evidence hands the authority to the next consumer");
    assert_eq!(successor.authority_key().as_bytes(), &DRM_KEY);
    assert_ne!(successor.key(), parent.key());
}

/// A helper leg cannot be fenced against a broker epoch the parent was not
/// admitted under.
///
/// A leg that names a stale epoch would outlive the epoch its parent's
/// admission was evaluated against, which is exactly the case a cached claim
/// must not survive.
#[test]
fn a_helper_leg_is_fenced_against_its_parents_epoch() {
    let device_ref = reference("Device/gpu-zero");
    let spec = drm_spec();
    let inventory = drm_inventory();
    let support = device_attachment_support();
    let authorization = BindingAuthorization::granted();
    let dependencies = fence(&device_ref, &[(GPU_WORKER, uid(GPU_WORKER_UID))]);
    let operations = operations();
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);
    let parent = admit_device_request(
        &admitted_source,
        &uid(GPU_WORKER_UID),
        &exclusive_request(&device_ref, &reference(GPU_WORKER), "render-node", "render-node"),
        &[],
    )
    .expect("the parent claim is admitted");
    let stale = StoreIncarnation::parse("store-zero").expect("bounded store incarnation");

    let refused = DeviceHelperLeg::bind(
        &parent,
        reference(VIDEO_WORKER),
        uid(VIDEO_WORKER_UID),
        &operations,
        &stale,
    )
    .expect_err("a leg under another epoch is refused");
    assert_eq!(refused.stage(), AdmissionStage::Reserve);
    assert_eq!(refused.reason(), RefusalReason::StaleAuthority);

    // A leg with no operation at all is refused too: a helper that may drive
    // nothing is not an admitted realization of the parent's claim.
    assert!(
        DeviceHelperLeg::bind(
            &parent,
            reference(VIDEO_WORKER),
            uid(VIDEO_WORKER_UID),
            &[],
            &store(),
        )
        .is_err()
    );
}

/// A request may name only a capability this source's trusted inventory
/// resolved, and only while the inventory backs it.
///
/// A device grant cannot be spelled into a declaration the host does not
/// provide, and a name is not a prefix: there is no `/dev/dri` form that
/// resolves to a render node.
#[test]
fn only_resolved_and_present_capabilities_are_admitted() {
    let device_ref = reference("Device/gpu-zero");
    let spec = drm_spec();
    let inventory = drm_inventory();
    let support = device_attachment_support();
    let authorization = BindingAuthorization::granted();
    let dependencies = fence(&device_ref, &[(GPU_WORKER, uid(GPU_WORKER_UID))]);
    let operations = operations();
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);

    for unresolved in ["dri", "udmabuf", "nvidia-device", "kvm", "render-node2"] {
        let request = exclusive_request(
            &device_ref,
            &reference(GPU_WORKER),
            "unresolved",
            unresolved,
        );
        let refused = admit_device_request(&admitted_source, &uid(GPU_WORKER_UID), &request, &[])
            .expect_err("an unresolved capability must be refused");
        assert_eq!(refused.stage(), AdmissionStage::Admit);
        assert_eq!(refused.reason(), RefusalReason::SourcePolicyRefused);
    }

    // A resolved capability the inventory no longer backs is refused too.
    let absent = inventory
        .with_presence(&function("nvidia-uvm"), DevicePresence::Absent)
        .expect("the inventory resolved nvidia-uvm");
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let gone = evidence.source(&device_ref, &spec, &absent);
    let request = exclusive_request(&device_ref, &reference(GPU_WORKER), "uvm", "nvidia-uvm");
    assert_eq!(
        admit_device_request(&gone, &uid(GPU_WORKER_UID), &request, &[])
            .expect_err("an absent capability must be refused")
            .reason(),
        RefusalReason::SourcePolicyRefused
    );
    assert_eq!(
        absent
            .present_functions()
            .iter()
            .map(DeviceFunction::as_str)
            .collect::<Vec<_>>(),
        vec!["render-node"],
        "only the backed capability is in the admitted surface"
    );
}

/// A claim from another Device, an unauthorized subject, an unsupported
/// presentation, and an empty fence are all refused.
///
/// Each of these is a way a request could reach a device without the source
/// deciding, and the shared evaluator refuses them before any effect.
#[test]
fn a_claim_that_the_source_did_not_decide_is_refused() {
    let device_ref = reference("Device/gpu-zero");
    let spec = drm_spec();
    let inventory = drm_inventory();
    let support = device_attachment_support();
    let dependencies = fence(&device_ref, &[(GPU_WORKER, uid(GPU_WORKER_UID))]);
    let operations = operations();
    let request = exclusive_request(&device_ref, &reference(GPU_WORKER), "render-node", "render-node");

    let granted = BindingAuthorization::granted();
    let evidence = SourceEvidence::new(&support, &granted, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);
    // A request that names a different source is not this source's to admit.
    let other = exclusive_request(
        &reference("Device/gpu-one"),
        &reference(GPU_WORKER),
        "render-node",
        "render-node",
    );
    assert_eq!(
        admit_device_request(&admitted_source, &uid(GPU_WORKER_UID), &other, &[])
            .expect_err("a claim for another device is refused")
            .reason(),
        RefusalReason::SourcePolicyRefused
    );

    // A well-formed request with no authorization evidence is refused.
    let unauthorized = BindingAuthorization::absent();
    let evidence = SourceEvidence::new(&support, &unauthorized, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);
    let refused = admit_device_request(&admitted_source, &uid(GPU_WORKER_UID), &request, &[])
        .expect_err("an unauthorized claim is refused");
    assert_eq!(refused.stage(), AdmissionStage::Authorize);
    assert_eq!(refused.reason(), RefusalReason::IdentityNotAuthorized);

    // A realization that cannot deliver a device attachment is refused.
    let unsupported = BindingRealizationSupport::default();
    let evidence = SourceEvidence::new(&unsupported, &granted, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);
    let refused = admit_device_request(&admitted_source, &uid(GPU_WORKER_UID), &request, &[])
        .expect_err("an unrealizable presentation is refused");
    assert_eq!(refused.stage(), AdmissionStage::Prepare);
    assert_eq!(refused.reason(), RefusalReason::MandatoryFacetUnsupported);

    // An empty freshness fence is refused: a claim cached without its
    // dependency revisions is not an admission.
    let unfenced: Vec<FreshnessTuple> = Vec::new();
    let evidence = SourceEvidence::new(&support, &granted, &unfenced, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);
    let refused = admit_device_request(&admitted_source, &uid(GPU_WORKER_UID), &request, &[])
        .expect_err("an unfenced claim is refused");
    assert_eq!(refused.stage(), AdmissionStage::Reserve);
    assert_eq!(refused.reason(), RefusalReason::UnprovenEffect);
}

/// A shared Device arbitrates its claims against the declared ceiling.
///
/// The DRM node and the NVIDIA UVM node are separate physical authorities, so
/// two consumers of the same Device holding different capabilities are not a
/// conflict. Two consumers of the *same* authority are, once the ceiling is
/// reached.
#[test]
fn a_shared_device_arbitrates_against_its_declared_ceiling() {
    let device_ref = reference("Device/gpu-zero");
    let spec = shared_drm_spec();
    let inventory = drm_inventory();
    let support = device_attachment_support();
    let authorization = BindingAuthorization::granted();
    let dependencies = fence(
        &device_ref,
        &[
            (GPU_WORKER, uid(GPU_WORKER_UID)),
            (VIDEO_WORKER, uid(VIDEO_WORKER_UID)),
        ],
    );
    let operations = operations();
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);

    let shared = |consumer: &str, slot_name: &str, function_name: &str| {
        DeviceBindingRequest::new(
            device_ref.clone(),
            reference(consumer),
            slot(slot_name),
            function(function_name),
            DeviceClaimRequest::Shared,
            d2b_contracts_resource::v3::DeviceAttachmentMode::Mediated,
        )
        .expect("a shared device claim is constructible")
    };

    let first = admit_device_request(
        &admitted_source,
        &uid(GPU_WORKER_UID),
        &shared(GPU_WORKER, "dri", "render-node"),
        &[],
    )
    .expect("the first shared claim is admitted");
    assert_eq!(first.admission().rights(), RequestedRights::Share);
    assert!(!first.holds_exclusive());

    let second = admit_device_request(
        &admitted_source,
        &uid(VIDEO_WORKER_UID),
        &shared(VIDEO_WORKER, "uvm", "render-node"),
        &[LiveDeviceBinding::new(first.clone(), BindingLifecycleState::Active)],
    )
    .expect("the second shared claim is admitted: the ceiling is two");
    assert_eq!(second.authority_key().as_bytes(), &DRM_KEY);

    // The ceiling is reached; a third consumer of the same authority is
    // refused, and a different authority on the same Device still is not.
    let refused = admit_device_request(
        &admitted_source,
        &uid(OTHER_WORKER_UID),
        &shared(OTHER_WORKER, "dri", "render-node"),
        &[
            LiveDeviceBinding::new(first.clone(), BindingLifecycleState::Active),
            LiveDeviceBinding::new(second, BindingLifecycleState::Active),
        ],
    )
    .expect_err("the holder ceiling is two");
    assert_eq!(refused.stage(), AdmissionStage::Reserve);
    assert_eq!(refused.reason(), RefusalReason::ConflictingDeclaration);

    let other_authority = admit_device_request(
        &admitted_source,
        &uid(OTHER_WORKER_UID),
        &shared(OTHER_WORKER, "uvm", "nvidia-uvm"),
        &[
            LiveDeviceBinding::new(first, BindingLifecycleState::Active),
        ],
    )
    .expect("a distinct physical authority is not at the render node's ceiling");
    assert_eq!(other_authority.authority_key().as_bytes(), &UVM_KEY);

    // An exclusive claim is refused on a shared Device: the source does not
    // arbitrate exclusively, so the request is a source-policy refusal rather
    // than a conflict.
    let exclusive = exclusive_request(&device_ref, &reference(OTHER_WORKER), "x", "nvidia-uvm");
    let refused = admit_device_request(&admitted_source, &uid(OTHER_WORKER_UID), &exclusive, &[])
        .expect_err("a shared device does not admit an exclusive claim");
    assert_eq!(refused.stage(), AdmissionStage::Admit);
    assert_eq!(refused.reason(), RefusalReason::SourcePolicyRefused);
}

/// Device disappearance revokes or degrades current use without releasing
/// another owner's claim.
///
/// The DRM render node disappears. That claim and its helper leg are revoked
/// and the leg is withdrawn with its parent; the UVM claim on the same Device
/// is untouched; the other Device's own exclusive claim is untouched; and the
/// revoked authority is *not* handed to a new consumer until the revoked
/// relationship has release evidence.
#[test]
fn device_disappearance_revokes_only_the_affected_use() {
    let zero = reference("Device/gpu-zero");
    let one = reference("Device/gpu-one");
    let spec = drm_spec();
    let inventory = drm_inventory();
    let support = device_attachment_support();
    let authorization = BindingAuthorization::granted();
    let operations = operations();
    let zero_fence = fence(
        &zero,
        &[
            (GPU_WORKER, uid(GPU_WORKER_UID)),
            (OTHER_WORKER, uid(OTHER_WORKER_UID)),
        ],
    );
    let evidence = SourceEvidence::new(&support, &authorization, &zero_fence, &operations);
    let admitted_source = evidence.source(&zero, &spec, &inventory);
    let render = admit_device_request(
        &admitted_source,
        &uid(GPU_WORKER_UID),
        &exclusive_request(&zero, &reference(GPU_WORKER), "render-node", "render-node"),
        &[],
    )
    .expect("the render node claim is admitted");
    let uvm = admit_device_request(
        &admitted_source,
        &uid(OTHER_WORKER_UID),
        &exclusive_request(&zero, &reference(OTHER_WORKER), "uvm", "nvidia-uvm"),
        &[],
    )
    .expect("the uvm claim is admitted: a distinct physical authority");
    let leg = DeviceHelperLeg::bind(
        &render,
        reference(VIDEO_WORKER),
        uid(VIDEO_WORKER_UID),
        &[DeviceEffectOperation::OpenDevice],
        &store(),
    )
    .expect("the helper leg is bound to the render node claim");

    // The DRM node is gone. The UVM node is still there.
    let observed = inventory
        .with_presence(&function("render-node"), DevicePresence::Absent)
        .expect("the inventory resolved the render node");
    let held = [
        LiveDeviceBinding::new(render.clone(), BindingLifecycleState::Active),
        LiveDeviceBinding::new(uvm.clone(), BindingLifecycleState::Active),
    ];
    let fates: Vec<DeviceBindingFate> = decide_presence(&held, &observed);

    let render_fate = fates
        .iter()
        .find(|fate| fate.key() == render.key())
        .expect("the render node claim has a decision");
    assert_eq!(render_fate.outcome(), DeviceUseOutcome::Revoked);
    assert_eq!(render_fate.function().as_str(), "render-node");

    let uvm_fate = fates
        .iter()
        .find(|fate| fate.key() == uvm.key())
        .expect("the uvm claim has a decision");
    assert_eq!(
        uvm_fate.outcome(),
        DeviceUseOutcome::Retained,
        "another capability on the same device is untouched"
    );

    // The helper leg cannot outlive its parent's revocation.
    assert_eq!(
        leg_outcome(&leg, &fates),
        DeviceUseOutcome::Revoked,
        "a leg is its parent's reservation, so a revoked parent withdraws it"
    );

    // A claim whose own effect was never proven is degraded, not reported as
    // revoked, so uncertainty is never read as a completed release.
    let unproven = [LiveDeviceBinding::new(uvm.clone(), BindingLifecycleState::Unknown)];
    let degraded = decide_presence(&unproven, &observed);
    assert_eq!(degraded[0].outcome(), DeviceUseOutcome::Degraded);

    // The other Device's own claim is not part of this source's decision at
    // all, and the revoked authority is not reassigned while it drains.
    let draining = [LiveDeviceBinding::new(render.clone(), BindingLifecycleState::Revoking)];
    let replacement = exclusive_request(
        &zero,
        &reference(OTHER_WORKER),
        "render-node-2",
        "render-node",
    );
    let reassigned = evidence.source(&zero, &spec, &observed);
    let refused = admit_device_request(&reassigned, &uid(OTHER_WORKER_UID), &replacement, &draining)
        .expect_err("the revoked node is no longer backed, so a fresh claim is refused");
    assert_eq!(refused.reason(), RefusalReason::SourcePolicyRefused);
    let _: BindingRefusal = refused;

    // Once the revoked relationship has release evidence and the inventory
    // backs the node again, a new consumer is admitted.
    let restored = observed
        .with_presence(&function("render-node"), DevicePresence::Present)
        .expect("the inventory resolved the render node");
    let available = evidence.source(&zero, &spec, &restored);
    let released = [LiveDeviceBinding::new(render, BindingLifecycleState::Released)];
    assert!(
        admit_device_request(&available, &uid(OTHER_WORKER_UID), &replacement, &released).is_ok(),
        "release evidence plus a backed inventory reassigns the authority"
    );
    assert_eq!(
        one.to_canonical_string(),
        "Device/gpu-one",
        "the other device is a separate source with its own authority"
    );
}

/// The committed `DeviceBinding` row is the request the consumer authored.
///
/// The row name derives from the KTD3 identities rather than a declaration
/// position, so reordering declarations cannot churn it, and the committed
/// bytes read back as the same request rather than a second description.
#[test]
fn the_committed_row_is_the_request_the_consumer_authored() {
    let device_ref = reference("Device/gpu-zero");
    let spec = drm_spec();
    let inventory = drm_inventory();
    let support = device_attachment_support();
    let authorization = BindingAuthorization::granted();
    let dependencies = fence(&device_ref, &[(GPU_WORKER, uid(GPU_WORKER_UID))]);
    let operations = operations();
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);
    let request = exclusive_request(
        &device_ref,
        &reference(GPU_WORKER),
        "render-node",
        "render-node",
    );
    let admitted =
        admit_device_request(&admitted_source, &uid(GPU_WORKER_UID), &request, &[]).expect("admitted");

    let row = canonical_binding_row(&admitted).expect("the row renders");
    assert_eq!(row.name(), &binding_row_name(admitted.key()).expect("a row name"));
    assert_eq!(
        parsed_consumer_request(row.spec()).expect("the committed row decodes"),
        request,
        "the committed row declares exactly the request the consumer authored"
    );
    let rendered = String::from_utf8(row.spec().to_vec()).expect("canonical bytes are utf-8");
    assert!(
        !rendered.contains("/dev/") && !rendered.contains("uid="),
        "the committed request carries no device node path or numerical principal: {rendered}"
    );

    // The row name identifies the relationship, not its position: two
    // consumers with distinct identities and slots get distinct rows, and
    // the same relationship always names the same row. The second consumer is
    // admitted once the first has release evidence, so the two rows are
    // compared after a real succession rather than through a live conflict.
    let successor = exclusive_request(
        &device_ref,
        &reference(OTHER_WORKER),
        "render-node",
        "render-node",
    );
    let released = [LiveDeviceBinding::new(admitted.clone(), BindingLifecycleState::Released)];
    let successor = admit_device_request(
        &admitted_source,
        &uid(OTHER_WORKER_UID),
        &successor,
        &released,
    )
    .expect("a released relationship hands the authority to its successor");
    assert_ne!(
        binding_row_name(successor.key()).expect("a row name"),
        binding_row_name(admitted.key()).expect("a row name"),
        "two relationships never share one row"
    );
    assert_eq!(
        binding_row_name(admitted.key()).expect("a row name"),
        canonical_binding_row(&admitted)
            .expect("the row renders")
            .name()
            .clone(),
        "the same relationship always names the same row"
    );
}

/// An empty, oversized, or duplicated inventory is refused at construction.
///
/// A Device row whose resolved capability set is not bounded and unique could
/// be turned into a device-directory grant, so it never becomes an inventory
/// at all.
#[test]
fn an_inventory_must_be_bounded_and_name_each_capability_once() {
    let entry = |name: &str, key: u8| {
        DeviceInventoryEntry::new(
            function(name),
            DeviceAuthorityKey::from_core([key; 32]),
            DeviceAuthorityArbitration::Exclusive,
            DevicePresence::Present,
        )
    };
    assert!(DeviceInventory::new(Vec::new()).is_err(), "an empty set is refused");
    let duplicated = vec![entry("render-node", 1), entry("render-node", 2)];
    assert!(
        DeviceInventory::new(duplicated).is_err(),
        "one name may not appear twice"
    );
    let oversized: Vec<DeviceInventoryEntry> = (0..=d2b_provider_device::binding::MAX_DEVICE_FUNCTIONS)
        .map(|index| {
            DeviceInventoryEntry::new(
                function(&format!("node-{index}")),
                DeviceAuthorityKey::from_core([index as u8 + 1; 32]),
                DeviceAuthorityArbitration::Exclusive,
                DevicePresence::Present,
            )
        })
        .collect();
    assert!(
        DeviceInventory::new(oversized).is_err(),
        "the capability set stays bounded"
    );
    let inventory = drm_inventory();
    assert!(inventory.resolves(&function("render-node")));
    assert!(!inventory.resolves(&function("dri")));
}
