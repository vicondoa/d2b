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
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingContractError, BindingKind,
    BindingLifecycleState, BindingRealizationFacet, BindingRealizationSupport, BindingRefusal,
    BindingRowError, BindingSlot, DeviceArbitration, DeviceAuthorityArbitration, DeviceAuthorityKey,
    DeviceBindingRequest, DeviceBindingSpec, DeviceClaimRequest, DeviceClass, DeviceEffectOperation,
    DeviceFunction, DeviceSpec, DesiredDigest, DesiredRevision, FreshnessTuple, InventorySelector,
    ControllerGeneration, InventorySpec, RefusalReason, RequestedRights, ResourceRef, ResourceUid,
    StoreIncarnation, ZoneId, admit_binding_row_refs, execution_policy::BoundedToken,
};
use d2b_provider_device::binding::{
    DeviceAdmissionGrant, DeviceAdmissionSource, DeviceBindingDerivationError, DeviceBindingFate,
    DeviceHelperLeg, DeviceInventory, DeviceInventoryEntry, DevicePresence, DeviceUseOutcome,
    LiveDeviceBinding, admit_device_request, binding_row_name, canonical_binding_rows,
    decide_presence, device_attachment_support, device_binding_spec_decoder, leg_outcome,
};
use d2b_provider_device::test_support::{RecordingInventory, RecordingRuntime, recording_facets};
use d2b_provider_device::{
    DeviceBindingDriverArgs, DeviceBindingDriverStatus, DeviceComponent, UnattachedReason,
    declared_device_functions, device_binding_descriptor,
};
use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome,
};
use d2b_resource_runtime::error::{FailureClass, FailureKinds};
use d2b_resource_runtime::identity::{
    ResourceKey, ResourceProvenance, StoredDesiredResource,
};
use d2b_resource_runtime::manager::deterministic_uid;
use std::sync::Arc;

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
/// The consumer row name `GPU_WORKER` resolves to.
const GPU_WORKER_NAME: &str = "gpu-zero";
/// The zone every row in this file lives in.
const ZONE: &str = "dev";
/// The GPU Provider the parent `Device` row's own `providerRef` names.
const GPU_PROVIDER: &str = d2b_provider_device_gpu::PROVIDER_REF;

/// The decision a committed row carries when the source admitted it normally:
/// the claim's own right plus the family's declared attachment facet.
fn committed_decision() -> serde_json::Value {
    serde_json::json!({
        "admittedRights": ["exclusive"],
        "arbitration": "exclusive",
        "realizedFacets": ["device-attachment"],
    })
}
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

/// One committed `Device` row implies one `DeviceBinding` row per claim it
/// admits.
///
/// Each row is the family's own `DeviceBindingSpec`: the identities the
/// consumer authored plus the source's accepted decision. The decision admits
/// the right the row claims, realizes the attachment facet, and re-derives the
/// same KTD3 key the source admitted, so a reader and the graph cannot
/// disagree about what was admitted. The row name comes from those identities
/// rather than from a declaration position, so two claims never share one row.
#[test]
fn each_admitted_claim_mints_one_row_carrying_the_source_decision() {
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
        ],
    );
    let operations = operations();
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);

    // Two named capabilities on two physical authorities: neither claim
    // competes with the other, so both belong to this one Device row.
    let render = admit_device_request(
        &admitted_source,
        &uid(GPU_WORKER_UID),
        &exclusive_request(&device_ref, &reference(GPU_WORKER), "render-node", "render-node"),
        &[],
    )
    .expect("an exclusive claim on a free authority is admitted");
    let uvm = admit_device_request(
        &admitted_source,
        &uid(VIDEO_WORKER_UID),
        &exclusive_request(&device_ref, &reference(VIDEO_WORKER), "uvm", "nvidia-uvm"),
        &[],
    )
    .expect("the second named capability is a second physical authority");
    let admitted = [render, uvm];

    let rows = canonical_binding_rows(DeviceComponent::Gpu, &spec, &inventory, &admitted)
        .expect("the committed Device row mints its admitted relationships");
    assert_eq!(
        rows.len(),
        admitted.len(),
        "one row per (device, consumer, slot) claim"
    );
    assert_ne!(rows[0].name(), rows[1].name(), "two claims never share one row");

    for (row, binding) in rows.iter().zip(&admitted) {
        assert_eq!(row.name(), &binding_row_name(binding.key()).expect("a row name"));
        let decoded: DeviceBindingSpec = serde_json::from_slice(row.spec())
            .expect("the committed bytes decode through the family's own wire decoder");
        admit_binding_row_refs(
            BindingKind::Device,
            decoded.device_ref(),
            decoded.execution_ref(),
        )
        .expect("the row's own references are admitted for this binding kind");
        assert_eq!(decoded.device_ref(), &device_ref);
        assert_eq!(decoded.execution_ref(), binding.request().consumer_ref());
        assert_eq!(decoded.function(), binding.function());
        assert_eq!(decoded.claim(), &binding.request().claim());
        assert_eq!(decoded.slot().as_str(), binding.key().slot().as_str());
        assert_eq!(
            decoded
                .key(
                    zone(),
                    device_uid(),
                    binding.key().consumer_uid().clone(),
                )
                .expect("the committed row derives its own key"),
            *binding.key(),
            "the committed row reaches exactly the relationship the source admitted"
        );

        let decision = decoded.source();
        assert!(
            decision
                .admitted_rights()
                .contains(&decoded.claim().requested_rights()),
            "the source decision admits the right this row claims"
        );
        assert_eq!(decision.arbitration(), BindingArbitration::Exclusive);
        assert_eq!(decision.realized_facets(), support.facets());
        assert!(
            decision
                .realized_facets()
                .contains(&BindingRealizationFacet::DeviceAttachment),
            "a device binding is delivered as an attachment or a descriptor"
        );

        let rendered = String::from_utf8(row.spec().to_vec()).expect("canonical bytes are utf-8");
        assert!(
            !rendered.contains("/dev/") && !rendered.contains("uid="),
            "the committed row carries no device node path or numerical principal: {rendered}"
        );
    }
}

/// The capability vocabulary is the source row's, not the inventory's.
///
/// The trusted inventory says which named capabilities the host backs right
/// now; the committed row says which ones this Provider can deliver. A claim
/// outside the second is refused at the row rather than committed as a
/// declaration no family realizes, and the row's own `providerRef` is what
/// selects the vocabulary.
#[test]
fn a_claim_outside_the_source_rows_vocabulary_is_refused() {
    let device_ref = reference("Device/gpu-zero");
    let spec = drm_spec();
    // The observed DRM inventory also resolves a hidraw node: the trusted
    // inventory is an observation, not this family's capability vocabulary.
    let inventory = DeviceInventory::new(vec![
        DeviceInventoryEntry::new(
            function("render-node"),
            DeviceAuthorityKey::from_core(DRM_KEY),
            DeviceAuthorityArbitration::Exclusive,
            DevicePresence::Present,
        ),
        DeviceInventoryEntry::new(
            function("hidraw"),
            DeviceAuthorityKey::from_core(UVM_KEY),
            DeviceAuthorityArbitration::Exclusive,
            DevicePresence::Present,
        ),
    ])
    .expect("the observed inventory is well formed");
    let declared = declared_device_functions(DeviceComponent::Gpu, &spec);
    assert!(
        declared.contains(&function("render-node")) && !declared.contains(&function("hidraw")),
        "the GPU family delivers DRM capabilities, not a security-key node"
    );

    let support = device_attachment_support();
    let authorization = BindingAuthorization::granted();
    let dependencies = fence(&device_ref, &[(GPU_WORKER, uid(GPU_WORKER_UID))]);
    let operations = operations();
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);
    let undeclared = admit_device_request(
        &admitted_source,
        &uid(GPU_WORKER_UID),
        &exclusive_request(&device_ref, &reference(GPU_WORKER), "hidraw-node", "hidraw"),
        &[],
    )
    .expect("the inventory backed the node, so the claim itself is admitted");

    assert_eq!(
        canonical_binding_rows(
            DeviceComponent::Gpu,
            &spec,
            &inventory,
            std::slice::from_ref(&undeclared),
        )
            .expect_err("a capability this row does not declare mints no row"),
        DeviceBindingDerivationError::FunctionNotDeclared
    );
    assert!(
        declared_device_functions(DeviceComponent::SecurityKey, &spec).is_empty(),
        "the security-key family does not bind a DRM bus class"
    );
    assert_eq!(
        canonical_binding_rows(DeviceComponent::SecurityKey, &spec, &inventory, &[undeclared])
            .expect_err("a family that binds no capability of this row declares nothing"),
        DeviceBindingDerivationError::FunctionNotDeclared
    );
}

/// A Device row that backs no claim mints no row.
///
/// Zero admitted relationships is not a row with a default attachment, and a
/// relationship whose named capability the trusted inventory no longer backs
/// is not committed either: the source keeps holding that claim until release
/// evidence arrives, but there is no attachment to declare for a capability
/// the host cannot deliver, so that one retires while its peers stay.
#[test]
fn a_source_row_that_backs_no_claim_mints_no_row() {
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
        ],
    );
    let operations = operations();
    let evidence = SourceEvidence::new(&support, &authorization, &dependencies, &operations);
    let admitted_source = evidence.source(&device_ref, &spec, &inventory);
    let render = admit_device_request(
        &admitted_source,
        &uid(GPU_WORKER_UID),
        &exclusive_request(&device_ref, &reference(GPU_WORKER), "render-node", "render-node"),
        &[],
    )
    .expect("an exclusive claim on a free authority is admitted");
    let uvm = admit_device_request(
        &admitted_source,
        &uid(VIDEO_WORKER_UID),
        &exclusive_request(&device_ref, &reference(VIDEO_WORKER), "uvm", "nvidia-uvm"),
        &[],
    )
    .expect("the second named capability is a second physical authority");
    let admitted = [render, uvm];

    assert!(
        canonical_binding_rows(DeviceComponent::Gpu, &spec, &inventory, &[])
            .expect("an empty relationship set is an answer, not a refusal")
            .is_empty(),
        "a Device row that admits no claim mints no row"
    );

    let uvm_gone = inventory
        .with_presence(&function("nvidia-uvm"), DevicePresence::Absent)
        .expect("the inventory resolved the UVM node");
    let rows = canonical_binding_rows(DeviceComponent::Gpu, &spec, &uvm_gone, &admitted)
        .expect("a revoked capability retires its row rather than refusing the source");
    assert_eq!(
        rows.len(),
        1,
        "only the capability the host still backs keeps a row"
    );
    let decoded: DeviceBindingSpec =
        serde_json::from_slice(rows[0].spec()).expect("the retained row decodes");
    assert_eq!(decoded.function(), &function("render-node"));

    let all_gone = uvm_gone
        .with_presence(&function("render-node"), DevicePresence::Absent)
        .expect("the inventory resolved the render node");
    assert!(
        canonical_binding_rows(DeviceComponent::Gpu, &spec, &all_gone, &admitted)
            .expect("every relationship is revoked, not refused")
            .is_empty(),
        "a Device row whose capabilities are all gone mints no row at all"
    );
}

/// A consumer this binding kind does not admit never reaches a row.
///
/// The Device kind admits every binding consumer, so the only consumer refusal
/// is a reference that is not a consumer at all. It is refused where the
/// relationship is authored and again by the rule the committed row itself is
/// checked against, so no derived row can name one.
#[test]
fn a_consumer_this_kind_does_not_admit_never_reaches_a_row() {
    let device_ref = reference("Device/gpu-zero");
    let not_a_consumer = reference("Volume/data");
    assert_eq!(
        DeviceBindingRequest::new(
            device_ref.clone(),
            not_a_consumer.clone(),
            slot("render-node"),
            function("render-node"),
            DeviceClaimRequest::Exclusive,
            d2b_contracts_resource::v3::DeviceAttachmentMode::Descriptor,
        )
        .expect_err("a Volume is not a binding consumer"),
        BindingContractError::WrongResourceType
    );
    assert_eq!(
        admit_binding_row_refs(BindingKind::Device, &device_ref, &not_a_consumer)
            .expect_err("the row's own consumer reference is refused too"),
        BindingRowError::WrongConsumerType
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

// ---------------------------------------------------------------------------
// The `DeviceBinding` serving driver (U16)
// ---------------------------------------------------------------------------
//
// The driver is driven through the composition root's own construction: the
// factory builds its effects from the declared `DeviceEffectFacets`, so a test
// supplying the recording runtime and the recording inventory is exercising
// the production wiring rather than a substituted port.
//
// The properties under test are the ones the serving half has to earn:
//
// 1. A committed row reaches `validate` and `reconcile` through every fence:
//    the wire decode, the committed decision, the parent row behind its owner
//    fence, that row's own declared capability vocabulary, and the consumer
//    row.
// 2. The committed `BindingSourceDecision` is enforced, not trusted.
// 3. A capability the trusted inventory no longer backs is reported as
//    revoked through the family's own `capability_backed` observation, never
//    as served.
// 4. The attachment this driver cannot route is named, never claimed.

/// One committed `DeviceBinding` row, carrying the decision the source's own
/// `canonical_binding_rows` commits.
fn committed_binding_row(
    name: &str,
    source: serde_json::Value,
    function: &str,
    claim: DeviceClaimRequest,
) -> StoredDesiredResource {
    StoredDesiredResource {
        key: ResourceKey::new(ZONE, "DeviceBinding", name),
        uid: deterministic_uid(&ResourceKey::new(ZONE, "DeviceBinding", name)),
        generation: 1,
        owner_uid: Some(deterministic_uid(&ResourceKey::new(ZONE, "Device", "gpu-zero"))),
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec: serde_json::json!({
            "deviceRef": "Device/gpu-zero",
            "executionRef": GPU_WORKER,
            "function": function,
            "claim": serde_json::to_value(claim).expect("claim wire"),
            "slot": "gpu-render",
            "source": source,
        })
        .to_string()
        .into_bytes(),
        metadata: Vec::new(),
        created_at: 0,
    }
}

/// The driver context plus the manager endpoint and requeue it records.
struct Fixture {
    ctx: ResourceContext,
    manager: RecordingManagerEndpoint,
}

fn binding_fixture(row: StoredDesiredResource, manager: RecordingManagerEndpoint) -> Fixture {
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let requeue = RecordingRequeue::default();
    let ctx = ResourceContext::new(
        row,
        device_binding_spec_decoder(),
        Arc::new(manager.clone()),
        Arc::new(requeue.clone()),
        effects_tx,
        notify_tx,
    );
    let _ = requeue;
    Fixture { ctx, manager }
}

/// One seeded manager row.
fn seeded(key: ResourceKey, spec: Vec<u8>) -> StoredDesiredResource {
    StoredDesiredResource {
        uid: deterministic_uid(&key),
        generation: 2,
        owner_uid: None,
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec,
        metadata: Vec::new(),
        created_at: 0,
        key,
    }
}

/// The parent `Device` row: the stored envelope carries the Provider its
/// delivery is scoped to, which is what selects the realizing component.
///
/// The fixture is written in the contract's WIRE spelling rather than
/// round-tripped through its `Serialize` impl: `DeviceSpec` serializes the DRM
/// selector's `pci_slot` field as written while its own `Deserialize` admits
/// only `pciSlot`, so a round-tripped value would not decode. Spelling the row
/// the way the store persists it is what makes this a fixture for the driver's
/// real decode rather than a second opinion about it.
fn parent_device_bytes(provider_ref: &str) -> Vec<u8> {
    serde_json::json!({
        "providerRef": provider_ref,
        "deviceClass": "physical",
        "arbitration": "exclusive",
        "maxConcurrentClaims": 1,
        // The label the recording inventory's own spec resolves under, so the
        // capabilities it backs are the ones this row's named function is
        // compared against.
        "inventory": { "selector": { "busClass": "drm", "label": "recorded", "pciSlot": null } }
    })
    .to_string()
    .into_bytes()
}

/// A manager holding the parent `Device` row and the consumer row, so every
/// fence has something real to resolve.
fn serving_manager() -> RecordingManagerEndpoint {
    RecordingManagerEndpoint::new()
        .with_row(seeded(
            ResourceKey::new(ZONE, "Device", "gpu-zero"),
            parent_device_bytes(GPU_PROVIDER),
        ))
        .with_row(seeded(
            ResourceKey::new(ZONE, "Process", GPU_WORKER_NAME),
            b"{}".to_vec(),
        ))
}

/// The driver over the production facet construction.
async fn binding_driver() -> Box<dyn DynResourceDriver> {
    device_binding_descriptor(DeviceBindingDriverArgs {
        zone: ZoneId::parse(ZONE).expect("zone"),
        controller_generation: ControllerGeneration::new(1).expect("controller generation"),
        facets: recording_facets(Arc::new(RecordingRuntime::default())),
    })
    .factory
    .create(&ResourceKey::new(ZONE, "DeviceBinding", "row"))
    .await
}

/// A committed row reaches validate and reconcile through every fence, and the
/// pass reports the attachment it cannot route by name rather than claiming
/// one.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_committed_row_reaches_validate_and_reconcile() {
    let manager = serving_manager();
    let mut f = binding_fixture(
        committed_binding_row(
            "dev-binding-row",
            committed_decision(),
            "render-node",
            DeviceClaimRequest::Exclusive,
        ),
        manager.clone(),
    );
    let mut d = binding_driver().await;

    d.validate(&mut f.ctx).await.expect("validate");

    assert_eq!(d.reconcile(&mut f.ctx).await.expect("reconcile"), ReconcileOutcome::Satisfied);
    // The admitted state names the component the parent's own `providerRef`
    // selected and the opaque authority the trusted inventory resolved for the
    // row's named capability. The authority is asserted as "the one the
    // inventory resolved for THIS row", not as a literal: the digest is the
    // inventory's to mint, and pinning it here would pin a test double's
    // arithmetic rather than the driver's wiring.
    let inventory = RecordingInventory
        .resolved_for(GPU_PROVIDER)
        .expect("the recording inventory resolves the parent's declared selector");
    let resolved = inventory
        .entries()
        .iter()
        .find(|entry| entry.function().as_str() == "render-node")
        .expect("the recording inventory resolves the named capability");
    assert_eq!(
        f.ctx.status::<DeviceBindingDriverStatus>(),
        Some(&DeviceBindingDriverStatus::Admitted {
            component: DeviceComponent::Gpu,
            authority: resolved.authority_key().clone(),
        }),
        "the capability is declared and the recording inventory backs it",
    );
    // Both dependency edges were registered: the Device row and the consumer
    // row (R12/R17).
    let order = f.manager.call_order();
    assert!(order.iter().any(|entry| entry.contains("watch:Device/gpu-zero")), "{order:?}");
    assert!(order.iter().any(|entry| entry.contains("watch:Process/")), "{order:?}");
}

/// The committed `BindingSourceDecision` is enforced, not trusted: a row whose
/// admitted rights do not cover its own claim, that drops the attachment
/// facet, or that commits a facet outside the family's declared support is
/// refused terminally.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_committed_decision_that_does_not_admit_the_row_is_refused() {
    let manager = serving_manager();
    let refused: Vec<(&str, serde_json::Value)> = vec![
        (
            "observe-only",
            serde_json::json!({
                "admittedRights": ["observe"],
                "arbitration": "shared",
                "realizedFacets": ["device-attachment"],
            }),
        ),
        (
            "no-attachment-facet",
            serde_json::json!({
                "admittedRights": ["exclusive"],
                "arbitration": "exclusive",
                "realizedFacets": ["filesystem-presentation"],
            }),
        ),
        (
            "unsupported-facet",
            serde_json::json!({
                "admittedRights": ["exclusive"],
                "arbitration": "exclusive",
                "realizedFacets": ["device-attachment", "namespace-interface"],
            }),
        ),
    ];

    for (label, decision) in refused {
        let mut f = binding_fixture(
            committed_binding_row(
                "dev-binding-row",
                decision,
                "render-node",
                DeviceClaimRequest::Exclusive,
            ),
            manager.clone(),
        );
        let mut d = binding_driver().await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("a decision that does not admit the row is refused");
        assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID, "{label}");
        assert_eq!(failure.class(), FailureClass::Terminal, "{label} cannot converge by retrying");
    }
}

/// A parent row whose owner uid differs from this binding's owner is refused
/// terminally: the manager would silently re-parent.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_parent_whose_owner_differs_is_refused() {
    let manager = RecordingManagerEndpoint::new().with_row(StoredDesiredResource {
        key: ResourceKey::new(ZONE, "Device", "gpu-zero"),
        uid: deterministic_uid(&ResourceKey::new(ZONE, "Process", GPU_WORKER_NAME)),
        generation: 2,
        owner_uid: None,
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec: parent_device_bytes(GPU_PROVIDER),
        metadata: Vec::new(),
        created_at: 0,
    });
    let mut f = binding_fixture(
        committed_binding_row(
            "dev-binding-row",
            committed_decision(),
            "render-node",
            DeviceClaimRequest::Exclusive,
        ),
        manager,
    );
    let mut d = binding_driver().await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("a re-parenting row is refused");
    assert_eq!(failure.kind(), FailureKinds::BINDING_OWNER_MISMATCH);
    assert_eq!(failure.class(), FailureClass::Terminal);
}

/// A parent row that is not observable yet defers retryably rather than failing
/// the binding terminal (issue #511).
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_unobservable_parent_defers_retryably() {
    let mut f = binding_fixture(
        committed_binding_row(
            "dev-binding-row",
            committed_decision(),
            "render-node",
            DeviceClaimRequest::Exclusive,
        ),
        RecordingManagerEndpoint::new(),
    );
    let mut d = binding_driver().await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("an absent parent is not observable");
    assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
    assert_eq!(failure.class(), FailureClass::Retryable);
}

/// A parent row whose declared vocabulary does not back the capability the row
/// names is refused: the row says which ones this Provider can deliver, and a
/// claim outside it was never admitted here.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_capability_outside_the_parents_vocabulary_is_refused() {
    let manager = serving_manager();
    let mut f = binding_fixture(
        committed_binding_row(
            "dev-binding-row",
            committed_decision(),
            "not-a-declared-capability",
            DeviceClaimRequest::Exclusive,
        ),
        manager,
    );
    let mut d = binding_driver().await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("an undeclared capability is refused");
    assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
    assert_eq!(failure.class(), FailureClass::Terminal);
}

/// A parent row naming no Provider this family serves selects no component, so
/// no realization is reachable and the row is refused rather than dispatched
/// on a guess.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_parent_naming_a_foreign_provider_is_refused() {
    let manager = RecordingManagerEndpoint::new()
        .with_row(seeded(
            ResourceKey::new(ZONE, "Device", "gpu-zero"),
            parent_device_bytes("Provider/not-a-device-provider"),
        ))
        .with_row(seeded(
            ResourceKey::new(ZONE, "Process", GPU_WORKER_NAME),
            b"{}".to_vec(),
        ));
    let mut f = binding_fixture(
        committed_binding_row(
            "dev-binding-row",
            committed_decision(),
            "render-node",
            DeviceClaimRequest::Exclusive,
        ),
        manager,
    );
    let mut d = binding_driver().await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("no component resolves");
    assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
    assert_eq!(failure.class(), FailureClass::Terminal);
}

/// A restart adopts nothing: no host effect persists a claim this actor could
/// find, and adopting an attachment it cannot prove is exactly the failure R41
/// exists to prevent.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn recover_adopts_nothing_it_cannot_prove() {
    let mut f = binding_fixture(
        committed_binding_row(
            "dev-binding-row",
            committed_decision(),
            "render-node",
            DeviceClaimRequest::Exclusive,
        ),
        serving_manager(),
    );
    let mut d = binding_driver().await;

    assert_eq!(d.recover(&mut f.ctx).await.expect("recover"), RecoveryOutcome::Missing);
}

/// A pre-drain fences the relationship and the next pass reads the fence back
/// out of the in-memory status rather than re-admitting it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn pre_drain_fences_and_the_next_pass_reads_the_fence_back() {
    let mut f = binding_fixture(
        committed_binding_row(
            "dev-binding-row",
            committed_decision(),
            "render-node",
            DeviceClaimRequest::Exclusive,
        ),
        serving_manager(),
    );
    let mut d = binding_driver().await;

    d.pre_drain(&mut f.ctx).await.expect("pre_drain");
    assert_eq!(
        f.ctx.status::<DeviceBindingDriverStatus>(),
        Some(&DeviceBindingDriverStatus::Draining { component: DeviceComponent::Gpu }),
    );

    d.reconcile(&mut f.ctx).await.expect("reconcile after the fence");
    assert_eq!(
        f.ctx.status::<DeviceBindingDriverStatus>(),
        Some(&DeviceBindingDriverStatus::Draining { component: DeviceComponent::Gpu }),
        "the fence survives the next pass (R11: the status slot is the fence)",
    );
}

/// Teardown converges idempotently and converges on every retry: a
/// relationship this driver never attached has nothing to release, and the
/// status it leaves behind says so rather than claiming a withdrawal.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn teardown_is_idempotent_and_claims_no_withdrawal() {
    let mut f = binding_fixture(
        committed_binding_row(
            "dev-binding-row",
            committed_decision(),
            "render-node",
            DeviceClaimRequest::Exclusive,
        ),
        serving_manager(),
    );
    let mut d = binding_driver().await;

    d.delete(&mut f.ctx).await.expect("delete");
    assert_eq!(
        f.ctx.status::<DeviceBindingDriverStatus>(),
        Some(&DeviceBindingDriverStatus::Unattached {
            reason: UnattachedReason::AttachDispatchUnroutable,
        }),
    );
    d.delete(&mut f.ctx).await.expect("a retried delete converges");
}

