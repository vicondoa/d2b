//! The security-key Service does not arbitrate or claim the key itself.
//!
//! The pre-graph path let the family take the single Host physical-device
//! authority (`claim_physical_backing`) and reported a conflict when someone
//! else already held it, and the relay's accept loop decided who could hold the
//! key from a configured VM-id list. Both are second authorities beside the
//! graph. These tests pin the replacement: a security-key Service *requests* one
//! exclusive `DeviceBinding` for its hidraw backing, the `Device` source
//! arbitrates it, the Host relay realizes it as a bounded leg, and a ceremony
//! needs the Guest's own admitted `EndpointBinding` on top of it.

use std::sync::OnceLock;

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingEvidence, BindingKey,
    BindingKind, BindingLifecycleState, BindingObservation, BindingRealizationFacet,
    BindingRealizationSupport, CompletionCondition, DesiredDigest, DesiredRevision,
    DeviceAttachmentMode, DeviceAuthorityKey, DeviceClaimRequest, DeviceEffectOperation,
    DeviceFunction, EndpointAttachmentKind, EndpointBindingRequest, FreshnessTuple, RefusalReason,
    ReleaseOutcome, RequestedRights, ResourceRef, ResourceUid, SourceAdmission, SourceReservation,
    StoreIncarnation, ZoneId, admit_binding_request, execution_policy::BoundedToken,
};
use d2b_provider_device_security_key::{
    AdmittedCeremony, AdmittedDeviceClaim, BoundDeviceLeg, PhysicalAuthorityLease,
    PhysicalUsbBackingClaim, PhysicalUsbBackingToken, RelayLaunchTicket, SECURITY_KEY_HIDRAW_FUNCTION,
    SECURITY_KEY_RELAY_OPERATIONS, SecurityKeyClaimPort, SecurityKeyClaimRequest,
    SecurityKeyEffectError, SecurityKeyEffectPort,
    SecurityKeyLease, SecurityKeyLeaseError, SecurityKeyOpenIntent, SecurityKeySessionId,
    admit_ceremony, release_ceremony, security_key_device_request, security_key_guest_endpoint_request,
};

const HIDRAW_KEY: [u8; 32] = [7; 32];
const FOREIGN_KEY: [u8; 32] = [9; 32];
const DEVICE: &str = "Device/security-key";
const CONTROLLER: &str = "Process/device-security-key-service-controller";
const RELAY: &str = "Process/device-security-key-relay";
const FRONTEND: &str = "Process/d2b-sk-frontend";
const ENDPOINT: &str = "Endpoint/security-key-relay";
const GUEST: &str = "Guest/ceremony-a";

struct ConflictPort;

impl SecurityKeyEffectPort for ConflictPort {
    fn claim_physical_backing(
        &mut self,
        _: &PhysicalUsbBackingClaim,
    ) -> Result<PhysicalAuthorityLease, SecurityKeyEffectError> {
        Err(SecurityKeyEffectError::PhysicalUsbBackingConflict)
    }

    fn open_hidraw(
        &mut self,
        _: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError> {
        panic!("hidraw must not open after a physical backing conflict");
    }

    fn release_physical_backing(
        &mut self,
        _: PhysicalAuthorityLease,
    ) -> Result<(), SecurityKeyEffectError> {
        Ok(())
    }
}

/// The facts the security-key Service row declares about its claim.
///
/// The identities are process-wide constants for this file, so the request is
/// built once and borrowed: nothing here names a host identity, and the holder
/// is the `Guest` the Binding rides rather than anything the relay chose.
fn claim_request(helper: &str) -> SecurityKeyClaimRequest<'static> {
    static ZONE: OnceLock<ZoneId> = OnceLock::new();
    static DEVICE_REF: OnceLock<ResourceRef> = OnceLock::new();
    static GUEST_REF: OnceLock<ResourceRef> = OnceLock::new();
    static RELAY_REF: OnceLock<ResourceRef> = OnceLock::new();
    static RELAY_IDENTITY: OnceLock<ResourceUid> = OnceLock::new();
    SecurityKeyClaimRequest::new(
        ZONE.get_or_init(zone),
        DEVICE_REF.get_or_init(|| reference(DEVICE)),
        GUEST_REF.get_or_init(|| reference(GUEST)),
        RELAY_REF.get_or_init(|| reference(helper)),
        RELAY_IDENTITY.get_or_init(relay_uid),
    )
}

fn zone() -> ZoneId {
    ZoneId::parse("dev").expect("bounded zone")
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

fn device_uid() -> ResourceUid {
    uid("223e4567-e89b-42d3-a456-426614174001")
}

fn consumer_uid() -> ResourceUid {
    uid("323e4567-e89b-42d3-a456-426614174002")
}

fn relay_uid() -> ResourceUid {
    uid("423e4567-e89b-42d3-a456-426614174003")
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

fn admitted_claim(
    in_zone: &ZoneId,
    store: &StoreIncarnation,
    authority: [u8; 32],
    state: BindingLifecycleState,
) -> AdmittedDeviceClaim {
    let device_ref = reference(DEVICE);
    let request =
        security_key_device_request(&device_ref).expect("the family's device request is typed");
    let key = request
        .key(in_zone.clone(), device_uid(), consumer_uid())
        .expect("the request keys a relationship");
    let source = SourceAdmission::new(
        key.clone(),
        vec![RequestedRights::Exclusive],
        BindingArbitration::Exclusive,
    )
    .expect("the source decision is constructible");
    let admission = admit_binding_request(
        &key,
        request.requested_rights(),
        request.required_facets(),
        &BindingAuthorization::granted(),
        &source,
        &AdmittedDeviceClaim::support().expect("the attachment facet is supported"),
        &[freshness(in_zone, &device_ref, device_uid(), store)],
    )
    .expect("an exclusive hidraw claim is admitted");
    let reservation = SourceReservation::new(
        in_zone.clone(),
        device_uid(),
        BoundedToken::parse("sk-res").expect("bounded reservation id"),
    );
    AdmittedDeviceClaim::new(
        BindingEvidence::admitted(admission, reservation).observed(BindingObservation::new(
            state,
            CompletionCondition::Complete,
            CompletionCondition::Pending,
            ReleaseOutcome::Outstanding,
        )),
        DeviceFunction::parse(SECURITY_KEY_HIDRAW_FUNCTION).expect("bounded function token"),
        DeviceAuthorityKey::from_core(authority),
        SECURITY_KEY_RELAY_OPERATIONS.to_vec(),
    )
    .expect("the admitted relationship carries the relay's operation classes")
}

/// One bounded leg the `Device` source bound to the Host relay.
struct RelayLeg {
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

impl RelayLeg {
    fn for_claim(claim: &AdmittedDeviceClaim, epoch: &StoreIncarnation) -> Self {
        Self {
            parent_key: claim.key().clone(),
            reservation: claim.reservation().clone(),
            helper_ref: reference(RELAY),
            helper_uid: relay_uid(),
            function: claim.function().clone(),
            authority: claim.authority_key().clone(),
            operations: SECURITY_KEY_RELAY_OPERATIONS.to_vec(),
            epoch: epoch.clone(),
            holds_claim: false,
        }
    }
}

impl BoundDeviceLeg for RelayLeg {
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

/// Records every claim-path effect, so a refusal can be shown to have happened
/// before any of them.
#[derive(Default)]
struct RecordingClaimPort {
    calls: Vec<&'static str>,
}

impl SecurityKeyClaimPort for RecordingClaimPort {
    fn open_hidraw_leg(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceRef,
        _: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError> {
        self.calls.push("open-hidraw-leg");
        Ok(RelayLaunchTicket::from_core([4; 16]))
    }

    fn stop_relay_leg(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceRef,
        _: RelayLaunchTicket,
    ) -> Result<(), SecurityKeyEffectError> {
        self.calls.push("stop-relay-leg");
        Ok(())
    }

    fn release_claim(&mut self, _: &AdmittedDeviceClaim) -> Result<(), SecurityKeyEffectError> {
        self.calls.push("release-claim");
        Ok(())
    }
}

fn idle_lease() -> SecurityKeyLease {
    SecurityKeyLease::new(
        device_uid(),
        PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core(HIDRAW_KEY)),
    )
}

fn bound_lease(
    claim: &AdmittedDeviceClaim,
    admitted_store: &StoreIncarnation,
) -> SecurityKeyLease {
    let mut lease = idle_lease();
    lease
        .admit_relay_claim(
            &zone(),
            &reference(DEVICE),
            admitted_store,
            &reference(RELAY),
            claim,
        )
        .expect("the Service realizes the claim the source admitted");
    lease
}

/// The pre-graph physical claim is still refused before any hidraw effect, and it
/// is the only path in this crate that takes a device claim at all.
#[test]
fn physical_backing_conflict_is_reported_before_any_hidraw_effect() {
    let token = PhysicalUsbBackingToken::from_core(HIDRAW_KEY);
    let claim = PhysicalUsbBackingClaim::from_core(token.clone());
    assert_eq!(claim.token().as_bytes(), &HIDRAW_KEY);
    let mut lease = SecurityKeyLease::new(
        uid("123e4567-e89b-42d3-a456-426614174000"),
        claim,
    );
    assert!(
        lease
            .acquire(
                SecurityKeySessionId::from_core([4; 16]),
                uid("223e4567-e89b-42d3-a456-426614174001"),
                &mut ConflictPort,
            )
            .is_err()
    );
}

/// A semantic security-key Binding never claims the device, and the Host relay
/// is a bounded leg of the Service's relationship.
///
/// The Service's own request is the canonical `DeviceBindingRequest` for one
/// hidraw capability, one consumer, and one stable slot; without an
/// authorization the shared contract admits nothing at all. The only path that
/// opens the key takes that admitted relationship plus a leg that rides its
/// reservation, and a leg that claims the device itself is refused as a
/// competing allocation rather than becoming a second claimant.
#[test]
fn a_semantic_binding_never_claims_the_device() {
    let request = security_key_device_request(&reference(DEVICE))
        .expect("a security-key Service's device request is constructible");
    assert_eq!(request.source_ref(), &reference(DEVICE));
    assert_eq!(request.consumer_ref(), &reference(CONTROLLER));
    assert_eq!(request.slot().as_str(), "hidraw-device");
    assert_eq!(request.function().as_str(), SECURITY_KEY_HIDRAW_FUNCTION);
    assert_eq!(request.claim(), DeviceClaimRequest::Exclusive);
    assert_eq!(request.attachment(), DeviceAttachmentMode::Descriptor);
    assert_eq!(request.kind(), BindingKind::Device);

    // A Binding asks for an Endpoint, not for the key: the ceremony rides the
    // Service's claim rather than claiming the device again.
    let endpoint_request =
        security_key_guest_endpoint_request(&reference(ENDPOINT), &reference(FRONTEND))
            .expect("the Binding requests its relay endpoint");
    assert_eq!(endpoint_request.source_ref(), &reference(ENDPOINT));
    assert_eq!(endpoint_request.consumer_ref(), &reference(FRONTEND));
    assert_eq!(endpoint_request.attachment(), EndpointAttachmentKind::Connect);
    assert_eq!(endpoint_request.kind(), BindingKind::Endpoint);
    assert_eq!(endpoint_request.slot().as_str(), "relay-endpoint");

    // Without the source's authorization there is no evidence to realize.
    let key = request
        .key(zone(), device_uid(), consumer_uid())
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
        &[freshness(&zone(), &reference(DEVICE), device_uid(), &store("store-one"))],
    )
    .expect_err("an unauthorized request is not an admission");
    assert_eq!(refused.reason(), RefusalReason::IdentityNotAuthorized);

    // Two Services asking for the same backing device are two relationships;
    // the source arbitrates them and neither family can hold both.
    let admitted_store = store("store-one");
    let first = admitted_claim(
        &zone(),
        &admitted_store,
        HIDRAW_KEY,
        BindingLifecycleState::Active,
    );
    let other = admitted_claim(
        &zone(),
        &store("store-two"),
        FOREIGN_KEY,
        BindingLifecycleState::Active,
    );
    assert_eq!(
        first.key(),
        other.key(),
        "one Service relationship, admitted twice under different stores"
    );
    assert_ne!(first.epoch(), other.epoch());
    assert_ne!(first.authority_key(), other.authority_key());

    // A helper that claims the device is not a bounded realization of it.
    let mut competing = RelayLeg::for_claim(&first, &admitted_store);
    competing.holds_claim = true;
    let mut lease = bound_lease(&first, &admitted_store);
    let mut port = RecordingClaimPort::default();
    assert_eq!(
        lease.acquire_bound(
            SecurityKeySessionId::from_core([5; 16]),
            &claim_request(RELAY),
            &competing,
            &mut port,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Reserve,
            RefusalReason::ConflictingDeclaration
        ))
    );
    assert!(
        port.calls.is_empty(),
        "a competing claim must not reach the hidraw effect"
    );

    // The bounded realization opens the key, and only that.
    let leg = RelayLeg::for_claim(&first, &admitted_store);
    lease
        .acquire_bound(
            SecurityKeySessionId::from_core([5; 16]),
            &claim_request(RELAY),
            &leg,
            &mut port,
        )
        .expect("the admitted relationship realizes the relay");
    assert_eq!(port.calls, ["open-hidraw-leg"]);
}

/// A cross-Zone or stale claim refuses before the key is opened.
///
/// The lease compares the claim against the Zone its own row lives in and
/// against the store incarnation that admission was fenced under, before the
/// effect port is called. A claim from another Zone, a claim admitted under a
/// previous store, a leg fenced against another store, and a leg reaching a
/// different physical authority all leave the effect log empty.
#[test]
fn a_cross_zone_or_stale_claim_refuses_with_an_empty_effect_log() {
    let admitted_store = store("store-one");
    let claim = admitted_claim(
        &zone(),
        &admitted_store,
        HIDRAW_KEY,
        BindingLifecycleState::Active,
    );
    let mut port = RecordingClaimPort::default();

    // A relationship the source admitted for another Zone.
    let cross_zone = admitted_claim(
        &ZoneId::parse("other").expect("bounded zone"),
        &admitted_store,
        HIDRAW_KEY,
        BindingLifecycleState::Active,
    );
    let mut cross_zone_lease = idle_lease();
    assert_eq!(
        cross_zone_lease.admit_relay_claim(
            &zone(),
            &reference(DEVICE),
            &admitted_store,
            &reference(RELAY),
            &cross_zone,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused
        ))
    );
    assert!(port.calls.is_empty());

    // The claim admitted under a previous store incarnation.
    let mut stale_lease = idle_lease();
    assert_eq!(
        stale_lease.admit_relay_claim(
            &zone(),
            &reference(DEVICE),
            &store("store-two"),
            &reference(RELAY),
            &claim,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Reserve,
            RefusalReason::StaleAuthority
        ))
    );
    assert!(port.calls.is_empty());

    for (mutated, stage, reason) in [
        (
            {
                let mut stale = RelayLeg::for_claim(&claim, &admitted_store);
                stale.epoch = store("store-two");
                stale
            },
            AdmissionStage::Reserve,
            RefusalReason::StaleAuthority,
        ),
        (
            {
                let mut foreign = RelayLeg::for_claim(&claim, &admitted_store);
                foreign.authority = DeviceAuthorityKey::from_core(FOREIGN_KEY);
                foreign
            },
            AdmissionStage::Authorize,
            RefusalReason::RequiredCapabilityOutsideCeiling,
        ),
        (
            {
                let mut narrowed = RelayLeg::for_claim(&claim, &admitted_store);
                narrowed.operations = vec![DeviceEffectOperation::SecurityKeyApplyUdevRules];
                narrowed
            },
            AdmissionStage::Authorize,
            RefusalReason::MandatoryFacetUnsupported,
        ),
    ] {
        let mut isolated = bound_lease(&claim, &admitted_store);
        assert_eq!(
            isolated.acquire_bound(
                SecurityKeySessionId::from_core([6; 16]),
                &claim_request(RELAY),
                &mutated,
                &mut port,
            ),
            Err(SecurityKeyLeaseError::ClaimRefused(stage, reason))
        );
        assert!(
            port.calls.is_empty(),
            "a refused leg must not reach the hidraw effect"
        );
    }

    // A leg bound to a different helper is not this Service's relay.
    let mut other_helper = RelayLeg::for_claim(&claim, &admitted_store);
    other_helper.helper_ref = reference("Process/device-security-key-binding-controller");
    let mut isolated = bound_lease(&claim, &admitted_store);
    assert_eq!(
        isolated.acquire_bound(
            SecurityKeySessionId::from_core([6; 16]),
            &claim_request("Process/device-security-key-binding-controller"),
            &other_helper,
            &mut port,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Authorize,
            RefusalReason::SourcePolicyRefused
        ))
    );
    assert!(port.calls.is_empty());
}

/// Admit one Guest's `EndpointBinding` to the relay.
fn admitted_endpoint(
    guest: &str,
    store: &StoreIncarnation,
    state: BindingLifecycleState,
) -> BindingEvidence {
    let request: EndpointBindingRequest =
        security_key_guest_endpoint_request(&reference(ENDPOINT), &reference(FRONTEND))
            .expect("the Binding requests its relay endpoint");
    let guest_uid = uid(match guest {
        GUEST => "323e4567-e89b-42d3-a456-426614174002",
        "Guest/ceremony-b" => "523e4567-e89b-42d3-a456-426614174004",
        _ => "623e4567-e89b-42d3-a456-426614174005",
    });
    assert_eq!(request.consumer_ref(), &reference(FRONTEND));
    let key = request
        .key(zone(), uid("723e4567-e89b-42d3-a456-426614174006"), guest_uid.clone())
        .expect("the endpoint request keys a relationship");
    let source = SourceAdmission::new(
        key.clone(),
        vec![RequestedRights::Consume],
        BindingArbitration::Shared,
    )
    .expect("the source decision is constructible");
    let support = BindingRealizationSupport::new(vec![BindingRealizationFacet::EndpointDescriptor])
        .expect("the descriptor facet is a valid support set");
    let admission = admit_binding_request(
        &key,
        request.requested_rights(),
        request.required_facets(),
        &BindingAuthorization::granted(),
        &source,
        &support,
        &[freshness(&zone(), &reference(ENDPOINT), guest_uid, store)],
    )
    .expect("the endpoint relationship is admitted");
    let reservation = SourceReservation::new(
        zone(),
        uid("723e4567-e89b-42d3-a456-426614174006"),
        BoundedToken::parse("sk-ep-res").expect("bounded reservation id"),
    );
    BindingEvidence::admitted(admission, reservation).observed(BindingObservation::new(
        state,
        CompletionCondition::Complete,
        CompletionCondition::Complete,
        ReleaseOutcome::Outstanding,
    ))
}

/// A ceremony needs the Guest's own admitted Endpoint relationship on top of the
/// Service's current claim.
///
/// The relay's lease is keyed on the admitted Guest identity, not on a
/// configured name, and a ceremony is refused whenever either relationship is no
/// longer the admitted one. A revoking endpoint, an endpoint admitted under a
/// different store, and a device claim that moved to another store all refuse;
/// only the current pair takes the lease, and only that Guest can release it.
#[test]
fn a_ceremony_needs_the_guests_own_admitted_endpoint_relationship() {
    let admitted_store = store("store-one");
    let claim = admitted_claim(
        &zone(),
        &admitted_store,
        HIDRAW_KEY,
        BindingLifecycleState::Active,
    );
    let endpoint = admitted_endpoint(GUEST, &admitted_store, BindingLifecycleState::Active);
    let mut relay = d2b_provider_device_security_key::SecurityKeyState::new("hidraw-selector");

    let ceremony = AdmittedCeremony::new(&claim, &endpoint, &reference(GUEST))
        .expect("a current claim and endpoint admit a ceremony");
    assert_eq!(ceremony.device_key(), claim.key());
    assert_eq!(ceremony.endpoint_key(), endpoint.key());
    assert_eq!(ceremony.guest(), &reference(GUEST));
    assert_eq!(ceremony.epoch(), &admitted_store);

    // A configured name is not admission: the relay's legacy access list does
    // not make a ceremony admissible.
    relay.enable_vm("ceremony-a");
    let revoking = admitted_endpoint(GUEST, &admitted_store, BindingLifecycleState::Revoking);
    assert!(AdmittedCeremony::new(&claim, &revoking, &reference(GUEST)).is_err());
    assert_eq!(
        admit_ceremony(&mut relay, &ceremony, &claim, &revoking),
        None,
        "a revoked endpoint must not take the lease"
    );
    assert_eq!(relay.lease.holder().map(str::to_owned), None);

    // A device claim from another store is not this ceremony's relationship.
    let moved = admitted_claim(
        &zone(),
        &store("store-two"),
        FOREIGN_KEY,
        BindingLifecycleState::Active,
    );
    assert_eq!(admit_ceremony(&mut relay, &ceremony, &moved, &endpoint), None);
    assert_eq!(relay.lease.holder().map(str::to_owned), None);

    // The admitted pair takes the lease, and only that Guest releases it.
    let lease = admit_ceremony(&mut relay, &ceremony, &claim, &endpoint)
        .expect("the current relationships admit the ceremony");
    assert_eq!(
        relay.lease.holder().map(str::to_owned),
        Some(reference(GUEST).to_canonical_string())
    );
    let other = AdmittedCeremony::new(
        &claim,
        &admitted_endpoint("Guest/ceremony-b", &admitted_store, BindingLifecycleState::Active),
        &reference("Guest/ceremony-b"),
    )
    .expect("the second Guest's own endpoint is admitted too");
    assert_eq!(
        admit_ceremony(&mut relay, &other, &claim, &endpoint),
        None,
        "a second ceremony cannot take a held key"
    );
    release_ceremony(&mut relay, &other, lease);
    assert_eq!(
        relay.lease.holder().map(str::to_owned),
        Some(reference(GUEST).to_canonical_string()),
        "a foreign Guest's release must not free the held lease"
    );
    release_ceremony(&mut relay, &ceremony, lease);
    assert_eq!(relay.lease.holder().map(str::to_owned), None);
}

/// A ceremony holder is a `Guest` relationship, not a name the relay chose.
///
/// The endpoint relationship is delivered to the frontend helper the Binding
/// declared, so the holder a ceremony is keyed on has to be the Binding's own
/// `Guest` identity: a process name, a host identity, or a configured VM id is
/// refused before the lease is touched. Two relationships admitted under
/// different stores are not one ceremony either.
#[test]
fn a_ceremony_holder_must_be_an_admitted_guest_relationship() {
    let admitted_store = store("store-one");
    let claim = admitted_claim(
        &zone(),
        &admitted_store,
        HIDRAW_KEY,
        BindingLifecycleState::Active,
    );
    let endpoint = admitted_endpoint(GUEST, &admitted_store, BindingLifecycleState::Active);
    let relay = d2b_provider_device_security_key::SecurityKeyState::new("hidraw-selector");

    for holder in [RELAY, FRONTEND, CONTROLLER] {
        let refused = AdmittedCeremony::new(&claim, &endpoint, &reference(holder))
            .expect_err("a ceremony holder is a Guest, not a helper or a host");
        assert_eq!(refused.stage(), AdmissionStage::Admit);
        assert_eq!(refused.reason(), RefusalReason::SourcePolicyRefused);
    }
    assert_eq!(relay.lease.holder().map(str::to_owned), None);

    // The delivery helper is read off the admitted relationship, not supplied.
    let ceremony =
        AdmittedCeremony::new(&claim, &endpoint, &reference(GUEST)).expect("the Guest is admitted");
    assert_eq!(ceremony.helper(), &reference(FRONTEND));

    // A relationship admitted under another store is stale, not usable.
    assert!(
        AdmittedCeremony::new(
            &claim,
            &admitted_endpoint(GUEST, &store("store-two"), BindingLifecycleState::Active),
            &reference(GUEST),
        )
        .is_err(),
        "two relationships admitted under different stores are not one ceremony"
    );
    assert_eq!(relay.lease.holder().map(str::to_owned), None);
}

/// The bounded vocabulary the family's relationship depends on stays closed: one
/// attachment facet, and the operations the relay actually drives.
#[test]
fn the_family_relationship_declares_a_closed_realization_surface() {
    assert_eq!(
        AdmittedDeviceClaim::support()
            .expect("the attachment facet is supported")
            .facets(),
        &[BindingRealizationFacet::DeviceAttachment]
    );
    assert_eq!(AdmittedDeviceClaim::required_facet(), BindingRealizationFacet::DeviceAttachment);
    assert_eq!(
        SECURITY_KEY_RELAY_OPERATIONS,
        [
            DeviceEffectOperation::SecurityKeyOpenDevice,
            DeviceEffectOperation::SecurityKeyApplyUdevRules,
        ]
    );
}
