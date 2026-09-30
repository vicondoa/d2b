//! The USB Service's physical backing is a `Device` relationship, not a claim
//! this Provider takes.
//!
//! A USB Service used to arbitrate its own backing: the exclusive/shared
//! ceiling, the conflict, and the release lived in a table inside this crate,
//! over a Core-derived token no consumer could be checked against, and the host
//! bind followed that table. These tests pin the replacement from this side.
//! The Service *requests* a canonical `DeviceBindingRequest`, the `Device` source
//! arbitrates it, and the family realizes only what was admitted, as a bounded
//! helper leg. A second consumer's claim, a claim from another Zone, a claim
//! admitted under a previous store, a leg that claims the device itself, and a
//! device that reappears under a new authority all stop before a relay, a
//! listener, or a firewall rule exists.

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingEvidence, BindingKey,
    BindingKind, BindingLifecycleState, BindingObservation, BindingRefusal, CompletionCondition,
    DesiredDigest, DesiredRevision, DeviceAuthorityKey, DeviceBindingRequest, DeviceClaimRequest,
    DeviceEffectOperation, DeviceFunction, FreshnessTuple, RefusalReason, ReleaseOutcome,
    RequestedRights, ResourceGeneration, ResourceRef, ResourceUid, SourceAdmission, SourceReservation,
    StoreIncarnation, ZoneId, admit_binding_request, execution_policy::BoundedToken,
};
use d2b_provider_device_usbip::{
    AdmittedDeviceClaim, BoundDeviceLeg, ClaimProjectionFence, FirewallConfirmation, FirewallDigest,
    FirewallObservation, FirewallProjectionAction, FirewallToken, NetworkDependency,
    RelayAuthorityLease, ScopedResourceUid, USBIP_RELAY_OPERATIONS,
    UsbipClaimPort, UsbipController, UsbipControllerError, UsbipEffectError, UsbipServiceClaim,
    UsbipServicePhase, usbip_service_device_request,
};

const BUS_KEY: [u8; 32] = [7; 32];
const FOREIGN_KEY: [u8; 32] = [9; 32];

const DEVICE: &str = "Device/usb-bus";
const CONTROLLER: &str = "Process/device-usbip-service-controller";
const RELAY: &str = "Process/usbip-relay";

fn zone(value: &str) -> ZoneId {
    ZoneId::parse(value).expect("bounded zone")
}

fn store(value: &str) -> StoreIncarnation {
    StoreIncarnation::parse(value).expect("bounded store incarnation")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("typed reference")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical resource uid")
}

fn function() -> DeviceFunction {
    DeviceFunction::parse("usb-bus").expect("bounded function token")
}

fn relay_uid() -> ResourceUid {
    uid("523e4567-e89b-42d3-a456-426614174004")
}

fn device_uid() -> ResourceUid {
    uid("123e4567-e89b-42d3-a456-426614174000")
}

fn controller_uid() -> ResourceUid {
    uid("223e4567-e89b-42d3-a456-426614174001")
}

fn freshness(
    zone: &ZoneId,
    resource: &ResourceRef,
    resource_uid: ResourceUid,
    store: &StoreIncarnation,
) -> FreshnessTuple {
    FreshnessTuple::new(
        zone.clone(),
        store.clone(),
        resource.clone(),
        resource_uid,
        DesiredRevision::INITIAL,
        DesiredDigest::of(resource.to_canonical_string().as_bytes()),
    )
}

/// Admit the family's own request through the shared binding contract.
///
/// This is the same shared evaluator the `Device` source's own admission runs,
/// so an `AdmittedDeviceClaim` exists here only because an authorization, a
/// source decision, a realization support, and a freshness fence all agreed.
fn admit(
    zone: &ZoneId,
    store: &StoreIncarnation,
    device_ref: &ResourceRef,
    source_uid: ResourceUid,
    consumer_uid: ResourceUid,
    authority: DeviceAuthorityKey,
    state: BindingLifecycleState,
) -> Result<AdmittedDeviceClaim, BindingRefusal> {
    let request: DeviceBindingRequest =
        usbip_service_device_request(device_ref, DeviceClaimRequest::Exclusive)
            .map_err(|_| {
                BindingRefusal::new(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused)
            })?;
    let key = request
        .key(zone.clone(), source_uid.clone(), consumer_uid)
        .map_err(|_| BindingRefusal::new(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused))?;
    let source = SourceAdmission::new(
        key.clone(),
        vec![RequestedRights::Exclusive],
        BindingArbitration::Exclusive,
    )
    .map_err(|_| BindingRefusal::new(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused))?;
    let admission = admit_binding_request(
        &key,
        request.requested_rights(),
        request.required_facets(),
        &BindingAuthorization::granted(),
        &source,
        &AdmittedDeviceClaim::support()
            .map_err(|_| BindingRefusal::new(AdmissionStage::Prepare, RefusalReason::MandatoryFacetUnsupported))?,
        &[freshness(zone, device_ref, source_uid.clone(), store)],
    )?;
    let reservation = SourceReservation::new(
        zone.clone(),
        source_uid,
        BoundedToken::parse("usb-res").expect("bounded reservation id"),
    );
    let evidence = BindingEvidence::admitted(admission, reservation).observed(
        BindingObservation::new(
            state,
            CompletionCondition::Complete,
            CompletionCondition::Pending,
            ReleaseOutcome::Outstanding,
        ),
    );
    AdmittedDeviceClaim::new(evidence, function(), authority, USBIP_RELAY_OPERATIONS.to_vec())
}

fn claim(store_value: &str, authority: [u8; 32]) -> AdmittedDeviceClaim {
    admit(
        &zone("dev"),
        &store(store_value),
        &reference(DEVICE),
        device_uid(),
        controller_uid(),
        DeviceAuthorityKey::from_core(authority),
        BindingLifecycleState::Active,
    )
    .expect("an exclusive USB backing claim is admitted")
}

/// One leg the `Device` source bound to the relay.
///
/// The fields are exactly what the source's own leg carries; the doubles below
/// vary one at a time so each refusal names the reason it exists.
struct TestLeg {
    parent_key: BindingKey,
    reservation: SourceReservation,
    helper_ref: ResourceRef,
    helper_uid: ResourceUid,
    function: DeviceFunction,
    authority: DeviceAuthorityKey,
    operations: Vec<DeviceEffectOperation>,
    epoch: StoreIncarnation,
    holds_claim: bool,
}

impl TestLeg {
    fn for_claim(
        claim: &AdmittedDeviceClaim,
        store: &StoreIncarnation,
        helper: &ResourceRef,
        helper_uid: ResourceUid,
    ) -> Self {
        Self {
            parent_key: claim.key().clone(),
            reservation: claim.reservation().clone(),
            helper_ref: helper.clone(),
            helper_uid,
            function: claim.function().clone(),
            authority: claim.authority_key().clone(),
            operations: USBIP_RELAY_OPERATIONS.to_vec(),
            epoch: store.clone(),
            holds_claim: false,
        }
    }
}

impl BoundDeviceLeg for TestLeg {
    fn parent_key(&self) -> &BindingKey {
        &self.parent_key
    }

    fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    fn helper_ref(&self) -> &ResourceRef {
        &self.helper_ref
    }

    fn helper_uid(&self) -> &ResourceUid {
        &self.helper_uid
    }

    fn function(&self) -> &DeviceFunction {
        &self.function
    }

    fn authority_key(&self) -> &DeviceAuthorityKey {
        &self.authority
    }

    fn operations(&self) -> &[DeviceEffectOperation] {
        &self.operations
    }

    fn epoch(&self) -> &StoreIncarnation {
        &self.epoch
    }

    fn holds_claim(&self) -> bool {
        self.holds_claim
    }
}

/// Records every effect the controller attempted, so a refusal can be shown to
/// have happened before any of them.
#[derive(Default)]
struct FakeClaimPort {
    calls: Vec<&'static str>,
    applied: bool,
}

impl UsbipClaimPort for FakeClaimPort {
    fn start_relay_leg(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceRef,
        _: &ResourceUid,
        _: &ClaimProjectionFence,
    ) -> Result<RelayAuthorityLease, UsbipEffectError> {
        self.calls.push("start-relay-leg");
        Ok(RelayAuthorityLease::from_adapter([1; 16]))
    }

    fn mutate_claim_firewall(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceUid,
        action: FirewallProjectionAction,
        _: &ClaimProjectionFence,
        _: Option<&FirewallToken>,
    ) -> Result<FirewallConfirmation, UsbipEffectError> {
        match action {
            FirewallProjectionAction::Apply => {
                self.calls.push("apply-firewall");
                self.applied = true;
                Ok(FirewallConfirmation::applied(
                    FirewallToken::from_adapter([2; 16]),
                    FirewallDigest::from_adapter([3; 32]),
                ))
            }
            FirewallProjectionAction::Remove => {
                self.calls.push("remove-firewall");
                self.applied = false;
                Ok(FirewallConfirmation::removed())
            }
        }
    }

    fn observe_claim_firewall(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceUid,
        _: &ClaimProjectionFence,
        _: &FirewallToken,
    ) -> Result<FirewallObservation, UsbipEffectError> {
        self.calls.push("observe-firewall");
        Ok(FirewallObservation::new(
            true,
            FirewallDigest::from_adapter([3; 32]),
        ))
    }

    fn stop_relay_leg(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceRef,
    ) -> Result<(), UsbipEffectError> {
        self.calls.push("stop-relay-leg");
        Ok(())
    }

    fn release_claim(&mut self, _: &AdmittedDeviceClaim) -> Result<(), UsbipEffectError> {
        self.calls.push("release-claim");
        Ok(())
    }
}

fn controller() -> UsbipController {
    UsbipController::new(
        ScopedResourceUid::new(
            uid("123e4567-e89b-42d3-a456-4266141740ff"),
            uid("323e4567-e89b-42d3-a456-426614174002"),
        ),
        ResourceGeneration::new(1).expect("bounded generation"),
        device_uid(),
    )
}

fn network() -> NetworkDependency {
    NetworkDependency::new(
        ScopedResourceUid::new(
            uid("123e4567-e89b-42d3-a456-4266141740ff"),
            uid("423e4567-e89b-42d3-a456-426614174003"),
        ),
        ResourceGeneration::new(4).expect("bounded generation"),
        true,
    )
    .with_assignment_epoch(3)
    .expect("a bounded assignment epoch")
}

fn reconcile(
    service: &mut UsbipController,
    store: &StoreIncarnation,
    claim: &AdmittedDeviceClaim,
    leg: &TestLeg,
    port: &mut FakeClaimPort,
) -> Result<(), UsbipControllerError> {
    service.reconcile_claim(
        &UsbipServiceClaim::new(
            &zone("dev"),
            store,
            &reference(DEVICE),
            claim,
            &reference(RELAY),
            &relay_uid(),
        ),
        leg,
        network(),
        port,
    )
}

/// A semantic USB Service requests its `Device` relationship and holds no
/// claim of its own.
///
/// The request names the Service's own controller `Process` and one stable slot,
/// so the relationship the `Device` source arbitrates is keyed by the family's
/// declaration rather than by a backing token, a bus id, or a table row. Without
/// an authorization the shared contract admits nothing at all, so no request can
/// stand in for an admission - and the only path that realizes the backing is a
/// bounded leg of the admitted relationship. A leg that claims the device itself
/// is refused as a competing allocation.
#[test]
fn a_semantic_service_requests_the_device_and_only_a_bounded_leg_realizes_it() {
    let request = usbip_service_device_request(&reference(DEVICE), DeviceClaimRequest::Exclusive)
        .expect("a USB Service's device request is constructible");
    assert_eq!(request.consumer_ref(), &reference(CONTROLLER));
    assert_eq!(request.slot().as_str(), "usb-backing");
    assert_eq!(request.function().as_str(), "usb-bus");
    assert_eq!(request.claim(), DeviceClaimRequest::Exclusive);
    assert_eq!(request.kind(), BindingKind::Device);
    assert_eq!(
        request.required_facets(),
        &[AdmittedDeviceClaim::required_facet()]
    );

    // Without the source's authorization the shared contract mints nothing, so
    // there is no evidence the family could realize.
    let key = request
        .key(zone("dev"), device_uid(), controller_uid())
        .expect("the request keys a relationship");
    let source = SourceAdmission::new(
        key.clone(),
        vec![RequestedRights::Exclusive],
        BindingArbitration::Exclusive,
    )
    .expect("the source decision is constructible");
    let refused = admit_binding_request(
        &key,
        request.requested_rights(),
        request.required_facets(),
        &BindingAuthorization::absent(),
        &source,
        &AdmittedDeviceClaim::support().expect("the attachment facet is supported"),
        &[freshness(
            &zone("dev"),
            &reference(DEVICE),
            device_uid(),
            &store("store-one"),
        )],
    )
    .expect_err("an unauthorized request is not an admission");
    assert_eq!(refused.reason(), RefusalReason::IdentityNotAuthorized);

    // The admitted relationship plus a bounded leg is what realizes the relay.
    let admitted_store = store("store-one");
    let admitted = claim("store-one", BUS_KEY);
    let leg = TestLeg::for_claim(&admitted, &admitted_store, &reference(RELAY), relay_uid());
    let mut port = FakeClaimPort::default();
    let mut service = controller();
    reconcile(&mut service, &admitted_store, &admitted, &leg, &mut port)
        .expect("an admitted claim with a bounded leg is realized");
    assert_eq!(port.calls, ["start-relay-leg", "apply-firewall"]);
    assert_eq!(service.phase(), UsbipServicePhase::Ready);

    // A helper that claims the device is not a bounded realization of it.
    let mut competing = TestLeg::for_claim(&admitted, &admitted_store, &reference(RELAY), relay_uid());
    competing.holds_claim = true;
    let refusal = admitted
        .verify_helper_leg(
            &reference(RELAY),
            &relay_uid(),
            &USBIP_RELAY_OPERATIONS,
            &competing,
        )
        .expect_err("a leg holding its own claim is a competing allocation");
    assert_eq!(refusal.stage(), AdmissionStage::Reserve);
    assert_eq!(refusal.reason(), RefusalReason::ConflictingDeclaration);

    // A leg that drops an operation class the relay needs is refused too.
    let mut narrowed = TestLeg::for_claim(&admitted, &admitted_store, &reference(RELAY), relay_uid());
    narrowed.operations = vec![DeviceEffectOperation::SpawnRunner];
    assert_eq!(
        admitted
            .verify_helper_leg(
                &reference(RELAY),
                &relay_uid(),
                &USBIP_RELAY_OPERATIONS,
                &narrowed,
            )
            .expect_err("a leg without the projection operation cannot drive the relay")
            .reason(),
        RefusalReason::MandatoryFacetUnsupported
    );
}

/// A second consumer's claim is not this Service's relationship.
///
/// Two USB Services naming the same backing device produce two distinct
/// relationships: the `Device` source arbitrates them, and this Service refuses
/// anything whose key, slot, source, or consumer is not its own. The refusal
/// happens before the relay exists, so the losing Service leaves no listener and
/// no firewall rule behind.
#[test]
fn a_foreign_consumers_claim_is_refused_before_any_effect() {
    let admitted_store = store("store-one");
    let own = claim("store-one", BUS_KEY);
    let foreign = admit(
        &zone("dev"),
        &admitted_store,
        &reference("Device/usb-bus-other"),
        uid("623e4567-e89b-42d3-a456-426614174005"),
        uid("723e4567-e89b-42d3-a456-426614174006"),
        DeviceAuthorityKey::from_core(FOREIGN_KEY),
        BindingLifecycleState::Active,
    )
    .expect("the other Service's own claim is admitted by the source");
    assert_ne!(foreign.key(), own.key());

    let mut port = FakeClaimPort::default();
    let leg = TestLeg::for_claim(&foreign, &admitted_store, &reference(RELAY), relay_uid());
    assert_eq!(
        reconcile(&mut controller(), &admitted_store, &foreign, &leg, &mut port),
        Err(UsbipControllerError::Effect(UsbipEffectError::ClaimRefused(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        )))
    );
    assert_eq!(
        port.calls,
        Vec::<&str>::new(),
        "a refused claim must not have reached the effect port"
    );

    // A leg bound to the other Service's reservation is refused even when the
    // claim handed over is this Service's own.
    let borrowed = TestLeg::for_claim(&foreign, &admitted_store, &reference(RELAY), relay_uid());
    let refusal = own
        .verify_helper_leg(
            &reference(RELAY),
            &relay_uid(),
            &USBIP_RELAY_OPERATIONS,
            &borrowed,
        )
        .expect_err("a leg riding another reservation is stale");
    assert_eq!(refusal.stage(), AdmissionStage::Reserve);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
}

/// A cross-Zone or stale claim refuses before any host bind or firewall change.
///
/// The Service compares the claim against its own row's Zone, the store
/// incarnation it is fenced against, and the leg's own fence - all before the
/// port is called even once. A claim from another Zone, a draining
/// relationship, a leg fenced against another store, a leg reaching another
/// physical authority, and a leg bound to a different helper all leave the
/// effect log empty.
#[test]
fn a_cross_zone_or_stale_claim_refuses_with_an_empty_effect_log() {
    let admitted_store = store("store-one");
    let admitted = claim("store-one", BUS_KEY);

    let cross_zone = admit(
        &zone("other"),
        &admitted_store,
        &reference(DEVICE),
        device_uid(),
        controller_uid(),
        DeviceAuthorityKey::from_core(BUS_KEY),
        BindingLifecycleState::Active,
    )
    .expect("a cross-Zone relationship is still a well-formed relationship");
    let cross_zone_leg = TestLeg::for_claim(&cross_zone, &admitted_store, &reference(RELAY), relay_uid());
    let mut cross_zone_port = FakeClaimPort::default();
    assert_eq!(
        reconcile(
            &mut controller(),
            &admitted_store,
            &cross_zone,
            &cross_zone_leg,
            &mut cross_zone_port
        ),
        Err(UsbipControllerError::Effect(UsbipEffectError::ClaimRefused(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        )))
    );
    assert!(cross_zone_port.calls.is_empty());

    let draining = admit(
        &zone("dev"),
        &admitted_store,
        &reference(DEVICE),
        device_uid(),
        controller_uid(),
        DeviceAuthorityKey::from_core(BUS_KEY),
        BindingLifecycleState::Draining,
    )
    .expect("a draining relationship is still a relationship");
    let draining_leg = TestLeg::for_claim(&draining, &admitted_store, &reference(RELAY), relay_uid());
    let mut draining_port = FakeClaimPort::default();
    assert_eq!(
        reconcile(
            &mut controller(),
            &admitted_store,
            &draining,
            &draining_leg,
            &mut draining_port
        ),
        Err(UsbipControllerError::Effect(UsbipEffectError::ClaimRefused(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
        )))
    );
    assert!(draining_port.calls.is_empty());

    for (mutated, stage, reason) in [
        (
            {
                let mut stale = TestLeg::for_claim(&admitted, &admitted_store, &reference(RELAY), relay_uid());
                stale.epoch = store("store-two");
                stale
            },
            AdmissionStage::Reserve,
            RefusalReason::StaleAuthority,
        ),
        (
            {
                let mut foreign = TestLeg::for_claim(&admitted, &admitted_store, &reference(RELAY), relay_uid());
                foreign.authority = DeviceAuthorityKey::from_core(FOREIGN_KEY);
                foreign
            },
            AdmissionStage::Authorize,
            RefusalReason::RequiredCapabilityOutsideCeiling,
        ),
        (
            {
                let mut holder = TestLeg::for_claim(&admitted, &admitted_store, &reference(RELAY), relay_uid());
                holder.holds_claim = true;
                holder
            },
            AdmissionStage::Reserve,
            RefusalReason::ConflictingDeclaration,
        ),
    ] {
        let mut port = FakeClaimPort::default();
        assert_eq!(
            reconcile(&mut controller(), &admitted_store, &admitted, &mutated, &mut port),
            Err(UsbipControllerError::Effect(UsbipEffectError::ClaimRefused(
                stage, reason
            )))
        );
        assert!(
            port.calls.is_empty(),
            "a refused leg must not have reached the effect port"
        );
    }

    // A leg bound to a different helper is not this Service's relay leg.
    let other_helper =
        TestLeg::for_claim(&admitted, &admitted_store, &reference("Process/usbip-daemon"), relay_uid());
    let mut helper_port = FakeClaimPort::default();
    assert_eq!(
        reconcile(
            &mut controller(),
            &admitted_store,
            &admitted,
            &other_helper,
            &mut helper_port
        ),
        Err(UsbipControllerError::Effect(UsbipEffectError::ClaimRefused(
            AdmissionStage::Authorize,
            RefusalReason::SourcePolicyRefused,
        )))
    );
    assert!(helper_port.calls.is_empty());

    // A projection fence cannot be minted for a store the claim was not
    // admitted under, so a stale store never reaches the effect port.
    assert_eq!(
        ClaimProjectionFence::new(
            &admitted,
            ResourceGeneration::new(4).unwrap(),
            ResourceGeneration::new(1).unwrap(),
            &store("store-two"),
        )
        .expect_err("a projection cannot be fenced against a store the claim was not admitted under")
        .reason(),
        RefusalReason::StaleAuthority
    );
    let fence = ClaimProjectionFence::new(
        &admitted,
        ResourceGeneration::new(4).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        &admitted_store,
    )
    .expect("the claim's own store fences its projection");
    assert_eq!(fence.store(), &admitted_store);
    assert_eq!(fence.network_generation(), ResourceGeneration::new(4).unwrap());
    assert_eq!(fence.service_generation(), ResourceGeneration::new(1).unwrap());
}

/// A device that reappears under a new authority does not adopt the previous
/// owner's retained state.
///
/// The Service is Ready on one claim: the relay leg is up and the projection is
/// installed. The device then goes away and comes back, and the `Device` source
/// admits a fresh relationship under a new store incarnation with a different
/// physical authority. That relationship is a different claim, and this Service
/// refuses it instead of re-pointing its retained relay and projection at the
/// reappeared device: the old reservation is not the new one, and adopting it
/// would be a foreign owner keeping the backing.
#[test]
fn a_reappearing_device_does_not_adopt_the_previous_owners_claim() {
    let first_store = store("store-one");
    let first = claim("store-one", BUS_KEY);
    let first_leg = TestLeg::for_claim(&first, &first_store, &reference(RELAY), relay_uid());
    let mut port = FakeClaimPort::default();
    let mut service = controller();
    reconcile(&mut service, &first_store, &first, &first_leg, &mut port)
        .expect("the first relationship is realized");
    assert_eq!(port.calls, ["start-relay-leg", "apply-firewall"]);
    assert_eq!(service.phase(), UsbipServicePhase::Ready);

    let second_store = store("store-two");
    let second = claim("store-two", FOREIGN_KEY);
    assert_eq!(
        second.key(),
        first.key(),
        "the relationship slot is stable across the reappearance; the authority is not"
    );
    assert_eq!(
        second.reservation(),
        first.reservation(),
        "the reservation is derived from the relationship, not from the device"
    );
    assert_ne!(
        second.epoch(),
        first.epoch(),
        "a reappearance after a store replacement is a new admission"
    );
    assert_ne!(
        second.authority_key(),
        first.authority_key(),
        "the inventory re-resolved the backing to a different physical authority"
    );
    let second_leg = TestLeg::for_claim(&second, &second_store, &reference(RELAY), relay_uid());
    let before = port.calls.clone();
    assert_eq!(
        reconcile(&mut service, &second_store, &second, &second_leg, &mut port),
        Err(UsbipControllerError::Effect(UsbipEffectError::ClaimRefused(
            AdmissionStage::Reserve,
            RefusalReason::StaleAuthority,
        ))),
        "a claim from a replaced store must not be realized over the retained one"
    );
    assert_eq!(
        port.calls, before,
        "the reappearance must not re-apply a projection or restart a relay"
    );
    assert_eq!(
        service.claim().map(AdmittedDeviceClaim::key),
        Some(first.key()),
        "the retained claim is still the one the source admitted"
    );
    assert!(port.applied, "the original projection is still the live one");

    // The previous owner's leg cannot ride the reappeared device either.
    let refusal = second
        .verify_helper_leg(
            &reference(RELAY),
            &relay_uid(),
            &USBIP_RELAY_OPERATIONS,
            &first_leg,
        )
        .expect_err("the previous owner's leg is fenced against the old store");
    assert_eq!(refusal.stage(), AdmissionStage::Reserve);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);

    // Teardown of the retained claim is still possible and still ordered, and
    // only then is the Service free for a fresh relationship.
    service
        .finalize_claim(&mut port)
        .expect("the retained claim drains");
    assert_eq!(
        port.calls,
        [
            "start-relay-leg",
            "apply-firewall",
            "remove-firewall",
            "stop-relay-leg",
            "release-claim",
        ]
    );
    assert!(service.source_released());
    assert!(service.claim().is_none());
    assert!(!port.applied);
}

/// The pre-graph claim table still refuses a second exclusive claimant, and it
/// is the only thing in this crate that arbitrates one.
#[test]
fn the_legacy_claim_table_still_refuses_a_second_exclusive_claimant() {
    use d2b_contracts_resource::v3::device::DeviceArbitration;
    use d2b_provider_device_usbip::{PhysicalUsbBackingToken, UsbipArbitrator, UsbipClaimError};

    let backing = PhysicalUsbBackingToken::from_core(BUS_KEY);
    let mut arbiter =
        UsbipArbitrator::new(DeviceArbitration::Exclusive, 1, backing.clone()).unwrap();
    arbiter.claim(device_uid(), backing.clone()).unwrap();
    assert_eq!(
        arbiter.claim(controller_uid(), backing),
        Err(UsbipClaimError::ClaimConflict)
    );
    assert_eq!(
        UsbipClaimError::ClaimConflict.code(),
        "device-claim-conflict"
    );
}
