//! The security-key lease, on the `Device` relationship.
//!
//! The pre-graph lease took a Core admission, claimed the Host physical backing
//! itself, and opened the hidraw node from that claim. The converted lease
//! realizes one relationship the `Device` source arbitrated, with the Host relay
//! as a bounded leg: it takes no device claim of its own, it refuses a claim or
//! a leg that is not the source's, and it stops the relay before the source
//! reservation is handed back.

use std::sync::OnceLock;

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingEvidence, BindingKey,
    BindingLifecycleState, BindingObservation, CompletionCondition, DesiredDigest, DesiredRevision,
    DeviceAuthorityKey, DeviceEffectOperation, DeviceFunction, FreshnessTuple, RefusalReason,
    ReleaseOutcome, RequestedRights, ResourceRef, ResourceUid, SourceAdmission, SourceReservation,
    StoreIncarnation, ZoneId, admit_binding_request, execution_policy::BoundedToken,
};
use d2b_provider_device_security_key::{
    AdmittedDeviceClaim, BoundDeviceLeg, LeaseState, MAX_SESSION_RING_SIZE, MIN_SESSION_RING_SIZE,
    PhysicalAuthorityLease, PhysicalUsbBackingClaim, PhysicalUsbBackingToken, RelayLaunchTicket,
    SECURITY_KEY_BINDING_RESOURCE_TYPE, SECURITY_KEY_HIDRAW_FUNCTION,
    SECURITY_KEY_RELAY_OPERATIONS, SECURITY_KEY_SERVICE_RESOURCE_TYPE, SecurityKeyClaimPort,
    SecurityKeyClaimRequest, SecurityKeyController,
    SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyLease, SecurityKeyLeaseError,
    SecurityKeyOpenIntent, SecurityKeySessionId, security_key_device_request,
    security_key_runner_contract,
};

struct FakePort {
    opens: usize,
    releases: usize,
    conflict: bool,
    release_error: Option<SecurityKeyEffectError>,
}

impl SecurityKeyEffectPort for FakePort {
    fn claim_physical_backing(
        &mut self,
        _: &PhysicalUsbBackingClaim,
    ) -> Result<PhysicalAuthorityLease, SecurityKeyEffectError> {
        if self.conflict {
            Err(SecurityKeyEffectError::PhysicalUsbBackingConflict)
        } else {
            Ok(PhysicalAuthorityLease::from_core([1; 16]))
        }
    }

    fn open_hidraw(
        &mut self,
        _: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError> {
        self.opens += 1;
        Ok(RelayLaunchTicket::from_core([2; 16]))
    }

    fn release_physical_backing(
        &mut self,
        _: PhysicalAuthorityLease,
    ) -> Result<(), SecurityKeyEffectError> {
        self.releases += 1;
        self.release_error.take().map_or(Ok(()), Err)
    }
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[test]
fn acquire_complete_and_cancel_follow_closed_lease_transitions() {
    let backing = PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core([7; 32]));
    let mut lease = SecurityKeyLease::new(uid("123e4567-e89b-42d3-a456-426614174000"), backing);
    let mut port = FakePort {
        opens: 0,
        releases: 0,
        conflict: false,
        release_error: None,
    };
    lease
        .acquire(
            SecurityKeySessionId::from_core([3; 16]),
            uid("223e4567-e89b-42d3-a456-426614174001"),
            &mut port,
        )
        .unwrap();
    assert_eq!(lease.state(), LeaseState::Active);
    lease.cancel(&mut port).unwrap();
    assert_eq!(lease.state(), LeaseState::Cancelled);
    assert_eq!(port.opens, 1);
    assert_eq!(port.releases, 1);
}

#[test]
fn failed_release_retains_authority_until_a_retry_succeeds() {
    let backing = PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core([8; 32]));
    let mut lease = SecurityKeyLease::new(uid("123e4567-e89b-42d3-a456-426614174000"), backing);
    let mut port = FakePort {
        opens: 0,
        releases: 0,
        conflict: false,
        release_error: Some(SecurityKeyEffectError::Transient),
    };
    lease
        .acquire(
            SecurityKeySessionId::from_core([6; 16]),
            uid("223e4567-e89b-42d3-a456-426614174001"),
            &mut port,
        )
        .unwrap();
    assert_eq!(
        lease.cancel(&mut port),
        Err(
            d2b_provider_device_security_key::SecurityKeyLeaseError::Effect(
                SecurityKeyEffectError::Transient
            )
        )
    );
    assert_eq!(lease.state(), LeaseState::Active);
    assert_eq!(port.releases, 1);

    lease.cancel(&mut port).unwrap();
    assert_eq!(lease.state(), LeaseState::Cancelled);
    assert_eq!(port.releases, 2);
}

#[test]
fn security_key_runner_contract_disables_legacy_scheduling() {
    let contract = security_key_runner_contract();
    assert_eq!(
        contract.service_resource_type(),
        SECURITY_KEY_SERVICE_RESOURCE_TYPE
    );
    assert_eq!(
        contract.binding_resource_type(),
        SECURITY_KEY_BINDING_RESOURCE_TYPE
    );
    assert!(contract.watched_configuration_is_dependency());
    assert!((30..=60).contains(&contract.repair_interval_secs()));
}

#[test]
fn session_ring_capacity_bounds_are_enforced() {
    let holder = uid("123e4567-e89b-42d3-a456-426614174000");
    let backing = PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core([7; 32]));
    assert!(
        SecurityKeyController::new(holder.clone(), backing.clone(), MIN_SESSION_RING_SIZE - 1)
            .is_err()
    );
    assert!(SecurityKeyController::new(holder, backing, MAX_SESSION_RING_SIZE + 1).is_err());
}

// ---------------------------------------------------------------------------
// The converted lease: an admitted `Device` claim and bounded helper legs
// ---------------------------------------------------------------------------

const HIDRAW_KEY: [u8; 32] = [7; 32];
const DEVICE: &str = "Device/security-key";
const RELAY: &str = "Process/device-security-key-relay";
const GUEST: &str = "Guest/ceremony-a";

fn zone() -> ZoneId {
    ZoneId::parse("dev").expect("bounded zone")
}

fn store(value: &str) -> StoreIncarnation {
    StoreIncarnation::parse(value).expect("bounded store incarnation")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("typed reference")
}

fn relay_uid() -> ResourceUid {
    uid("923e4567-e89b-42d3-a456-426614174005")
}

fn device_uid() -> ResourceUid {
    uid("223e4567-e89b-42d3-a456-426614174001")
}

fn holder_uid() -> ResourceUid {
    uid("323e4567-e89b-42d3-a456-426614174002")
}

/// Admit the family's own hidraw claim through the shared binding contract.
fn admitted_claim(
    in_zone: &ZoneId,
    store: &StoreIncarnation,
    authority: [u8; 32],
    state: BindingLifecycleState,
) -> AdmittedDeviceClaim {
    let device_ref = reference(DEVICE);
    let request = security_key_device_request(&device_ref)
        .expect("a security-key Service's device request is constructible");
    let key = request
        .key(in_zone.clone(), device_uid(), holder_uid())
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
        &[FreshnessTuple::new(
            in_zone.clone(),
            store.clone(),
            device_ref,
            device_uid(),
            DesiredRevision::INITIAL,
            DesiredDigest::of(b"security-key"),
        )],
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
struct FakeClaimPort {
    calls: Vec<&'static str>,
    open_failure: Option<SecurityKeyEffectError>,
    stop_failure: Option<SecurityKeyEffectError>,
    release_failure: Option<SecurityKeyEffectError>,
    seen_device: Option<ResourceUid>,
}

impl SecurityKeyClaimPort for FakeClaimPort {
    fn open_hidraw_leg(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceRef,
        intent: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError> {
        self.calls.push("open-hidraw-leg");
        self.seen_device = Some(intent.device_uid().clone());
        self.open_failure
            .take()
            .map_or(Ok(RelayLaunchTicket::from_core([4; 16])), Err)
    }

    fn stop_relay_leg(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceRef,
        _: RelayLaunchTicket,
    ) -> Result<(), SecurityKeyEffectError> {
        self.calls.push("stop-relay-leg");
        self.stop_failure.take().map_or(Ok(()), Err)
    }

    fn release_claim(&mut self, _: &AdmittedDeviceClaim) -> Result<(), SecurityKeyEffectError> {
        self.calls.push("release-claim");
        self.release_failure.take().map_or(Ok(()), Err)
    }
}

fn bound_lease(store_value: &str) -> (SecurityKeyLease, AdmittedDeviceClaim, StoreIncarnation) {
    let admitted_store = store(store_value);
    let claim = admitted_claim(&zone(), &admitted_store, HIDRAW_KEY, BindingLifecycleState::Active);
    let mut lease = SecurityKeyLease::new(
        device_uid(),
        PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core(HIDRAW_KEY)),
    );
    lease
        .admit_relay_claim(
            &zone(),
            &reference(DEVICE),
            &admitted_store,
            &reference(RELAY),
            &claim,
        )
        .expect("the Service realizes the claim the source admitted");
    (lease, claim, admitted_store)
}

/// The facts the security-key Service row declares about its claim.
/// The facts the security-key Service row declares about its claim.
///
/// The identities are process-wide constants for this file, so the request is
/// built once and borrowed: nothing here names a host identity, and the holder
/// is the `Guest` the Binding rides rather than anything the relay chose.
fn request(
    helper: &str,
) -> SecurityKeyClaimRequest<'static> {
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

fn acquire_bound(
    lease: &mut SecurityKeyLease,
    leg: &RelayLeg,
    port: &mut FakeClaimPort,
) -> Result<(), SecurityKeyLeaseError> {
    lease.acquire_bound(
        SecurityKeySessionId::from_core([5; 16]),
        &request(RELAY),
        leg,
        port,
    )
}

/// Relay shutdown precedes the source release, and a failed stop holds the
/// reservation.
///
/// The relay that can still reach the key is stopped first and the `Device`
/// reservation is handed back second, on every terminal transition. When the
/// stop does not confirm, nothing is released: a live relay holding a released
/// reservation is the same use-after-release as a mount that outlives its
/// volume, and the lease stays Active so the session is still owned.
#[test]
fn relay_shutdown_precedes_the_source_release_on_every_terminal_transition() {
    for (terminal, expected) in [
        ("complete", LeaseState::Completed),
        ("cancel", LeaseState::Cancelled),
        ("expire", LeaseState::Expired),
    ] {
        let (mut lease, claim, admitted_store) = bound_lease("store-one");
        let leg = RelayLeg::for_claim(&claim, &admitted_store);
        let mut port = FakeClaimPort::default();
        acquire_bound(&mut lease, &leg, &mut port)
            .expect("an admitted claim with a bounded leg opens the key");
        assert_eq!(port.calls, ["open-hidraw-leg"]);
        assert_eq!(port.seen_device.as_ref(), Some(&device_uid()));
        assert_eq!(lease.state(), LeaseState::Active);

        match terminal {
            "complete" => lease.complete_bound(&mut port).unwrap(),
            "cancel" => lease.cancel_bound(&mut port).unwrap(),
            _ => lease.expire_bound(&mut port).unwrap(),
        }
        assert_eq!(
            port.calls,
            ["open-hidraw-leg", "stop-relay-leg", "release-claim"],
            "{terminal} must stop the relay before the source release"
        );
        assert_eq!(lease.state(), expected);
        assert!(lease.source_released());
        assert!(lease.claim().is_none());
        assert!(lease.session().is_none());
    }

    // A relay that would not stop keeps the reservation held.
    let (mut lease, claim, admitted_store) = bound_lease("store-one");
    let leg = RelayLeg::for_claim(&claim, &admitted_store);
    let mut port = FakeClaimPort {
        stop_failure: Some(SecurityKeyEffectError::Transient),
        ..Default::default()
    };
    acquire_bound(&mut lease, &leg, &mut port).unwrap();
    port.calls.clear();
    assert!(matches!(
        lease.complete_bound(&mut port),
        Err(SecurityKeyLeaseError::Effect(SecurityKeyEffectError::Transient))
    ));
    assert_eq!(port.calls, ["stop-relay-leg"]);
    assert!(
        !port.calls.contains(&"release-claim"),
        "the reservation stays held while the relay is still running"
    );
    assert_eq!(lease.state(), LeaseState::Active);
    assert!(!lease.source_released());
    assert!(lease.claim().is_some());
}

/// A device that reappears under a new authority does not take over the lease.
///
/// The lease is Active on one claim when the device goes away and the `Device`
/// source admits a fresh relationship under a new store incarnation with a
/// different physical authority. That is a different admission, so the retained
/// claim is not replaced under a live session and the previous owner's relay leg
/// cannot ride the reappeared device.
#[test]
fn a_reappearing_device_does_not_adopt_the_previous_owners_lease() {
    let (mut lease, claim, admitted_store) = bound_lease("store-one");
    let leg = RelayLeg::for_claim(&claim, &admitted_store);
    let mut port = FakeClaimPort::default();
    acquire_bound(&mut lease, &leg, &mut port)
        .expect("the first relationship is realized");
    assert_eq!(lease.state(), LeaseState::Active);

    let second_store = store("store-two");
    let second = admitted_claim(&zone(), &second_store, [9; 32], BindingLifecycleState::Active);
    assert_eq!(
        second.key(),
        claim.key(),
        "the relationship slot is stable across the reappearance"
    );
    assert_ne!(second.epoch(), claim.epoch());
    assert_ne!(second.authority_key(), claim.authority_key());

    // A live session is not carried across the change.
    assert!(matches!(
        lease.admit_relay_claim(
            &zone(),
            &reference(DEVICE),
            &second_store,
            &reference(RELAY),
            &second,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(_, _))
    ));
    assert_eq!(
        lease.claim().map(AdmittedDeviceClaim::epoch),
        Some(claim.epoch()),
        "the retained claim is still the one the source admitted"
    );
    assert_eq!(lease.state(), LeaseState::Active);

    // A second session cannot start on the retained claim while one is live,
    // and it does not reach the hidraw effect either.
    let mut port_after = FakeClaimPort::default();
    assert!(matches!(
        acquire_bound(&mut lease, &leg, &mut port_after),
        Err(SecurityKeyLeaseError::SessionConflict)
    ));
    assert!(port_after.calls.is_empty());

    // The session drains, and only a fresh admission starts the next one.
    lease.complete_bound(&mut port).expect("the retained session drains");
    assert_eq!(port.calls, ["open-hidraw-leg", "stop-relay-leg", "release-claim"]);
    lease
        .admit_relay_claim(
            &zone(),
            &reference(DEVICE),
            &second_store,
            &reference(RELAY),
            &second,
        )
        .expect("a terminal lease accepts the reappeared device's own admission");
    assert_ne!(lease.claim().map(AdmittedDeviceClaim::epoch), Some(claim.epoch()));

    // The previous owner's leg cannot open the reappeared device.
    let mut stale = FakeClaimPort::default();
    assert_eq!(
        acquire_bound(&mut lease, &leg, &mut stale),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Reserve,
            RefusalReason::StaleAuthority
        )),
        "the previous owner's leg is fenced against the store it was bound in"
    );
    assert!(
        stale.calls.is_empty(),
        "a stale leg must not reach the hidraw effect"
    );

    let second_leg = RelayLeg::for_claim(&second, &second_store);
    let mut next = FakeClaimPort::default();
    acquire_bound(&mut lease, &second_leg, &mut next)
        .expect("the reappeared device is realized under its own admission");
    assert_eq!(next.calls, ["open-hidraw-leg"]);
    assert_eq!(lease.state(), LeaseState::Active);
}

/// A failed bounded open consumes nothing the source owns.
///
/// The claim is the source's reservation, so an open that fails leaves it
/// exactly where it was: the lease is immediately retryable and no second
/// consumer can be admitted against the same authority in between.
#[test]
fn a_failed_bounded_open_leaves_the_source_claim_in_place() {
    let (mut lease, claim, admitted_store) = bound_lease("store-one");
    let leg = RelayLeg::for_claim(&claim, &admitted_store);
    let mut port = FakeClaimPort {
        open_failure: Some(SecurityKeyEffectError::BrokerInaccessible),
        ..Default::default()
    };
    assert!(matches!(
        acquire_bound(&mut lease, &leg, &mut port),
        Err(SecurityKeyLeaseError::Effect(
            SecurityKeyEffectError::BrokerInaccessible
        ))
    ));
    assert_eq!(port.calls, ["open-hidraw-leg"]);
    assert_eq!(
        lease.state(),
        LeaseState::Idle,
        "a failed open is not a held session"
    );
    assert!(lease.claim().is_some(), "the source still holds its claim");
    assert!(!lease.source_released());
    assert!(lease.session().is_none());

    let mut retry = FakeClaimPort::default();
    acquire_bound(&mut lease, &leg, &mut retry)
        .expect("the retry rides the same admitted claim");
    assert_eq!(retry.calls, ["open-hidraw-leg"]);
    assert_eq!(lease.state(), LeaseState::Active);
}

/// A bounded session needs the source's relationship, its own Zone, and a leg
/// that realizes exactly that reservation.
#[test]
fn a_bounded_session_requires_the_sources_own_relationship() {
    let (mut lease, claim, admitted_store) = bound_lease("store-one");
    let leg = RelayLeg::for_claim(&claim, &admitted_store);
    let mut port = FakeClaimPort::default();

    // A claim admitted for another Zone is not this Service's relationship.
    let cross_zone = admitted_claim(
        &ZoneId::parse("other").expect("bounded zone"),
        &admitted_store,
        HIDRAW_KEY,
        BindingLifecycleState::Active,
    );
    let mut cross_zone_lease = SecurityKeyLease::new(
        device_uid(),
        PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core(HIDRAW_KEY)),
    );
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
    let cross_zone_leg = RelayLeg::for_claim(&cross_zone, &admitted_store);
    assert!(matches!(
        cross_zone_lease.acquire_bound(
            SecurityKeySessionId::from_core([8; 16]),
            &request(RELAY),
            &cross_zone_leg,
            &mut port,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(_, _))
    ));
    assert!(port.calls.is_empty());

    // A leg that claims the device itself.
    let mut competing = RelayLeg::for_claim(&claim, &admitted_store);
    competing.holds_claim = true;
    assert!(matches!(
        acquire_bound(&mut lease, &competing, &mut port),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Reserve,
            RefusalReason::ConflictingDeclaration
        ))
    ));
    assert!(port.calls.is_empty());

    // A leg fenced against a store the claim was not admitted under.
    let mut stale = RelayLeg::for_claim(&claim, &store("store-two"));
    stale.epoch = store("store-two");
    assert!(matches!(
        acquire_bound(&mut lease, &stale, &mut port),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Reserve,
            RefusalReason::StaleAuthority
        ))
    ));
    assert!(port.calls.is_empty());

    // A leg bound to a different helper is not this Service's relay.
    let mut other = RelayLeg::for_claim(&claim, &admitted_store);
    other.helper_ref = reference("Process/d2b-sk-frontend");
    assert!(matches!(
        acquire_bound(&mut lease, &other, &mut port),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Authorize,
            RefusalReason::SourcePolicyRefused
        ))
    ));
    assert!(port.calls.is_empty());

    // A relationship that is revoking does not admit a new session.
    let revoking = admitted_claim(
        &zone(),
        &admitted_store,
        HIDRAW_KEY,
        BindingLifecycleState::Revoking,
    );
    let revoking_leg = RelayLeg::for_claim(&revoking, &admitted_store);
    let mut revoking_lease = SecurityKeyLease::new(
        device_uid(),
        PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core(HIDRAW_KEY)),
    );
    assert!(matches!(
        revoking_lease.admit_relay_claim(
            &zone(),
            &reference(DEVICE),
            &admitted_store,
            &reference(RELAY),
            &revoking,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority
        ))
    ));
    assert!(matches!(
        revoking_lease.acquire_bound(
            SecurityKeySessionId::from_core([6; 16]),
            &request(RELAY),
            &revoking_leg,
            &mut port,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(_, _))
    ));
    assert!(port.calls.is_empty());

    // Only the source's own relationship with a bounded leg opens the key.
    acquire_bound(&mut lease, &leg, &mut port)
        .expect("the admitted relationship realizes the relay");
    assert_eq!(port.calls, ["open-hidraw-leg"]);
}

/// A lease with no admitted claim cannot open the key at all.
#[test]
fn a_lease_without_an_admitted_claim_opens_nothing() {
    let mut lease = SecurityKeyLease::new(
        device_uid(),
        PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core(HIDRAW_KEY)),
    );
    let admitted_store = store("store-one");
    let claim = admitted_claim(&zone(), &admitted_store, HIDRAW_KEY, BindingLifecycleState::Active);
    let leg = RelayLeg::for_claim(&claim, &admitted_store);
    let mut port = FakeClaimPort::default();
    assert_eq!(
        lease.acquire_bound(
            SecurityKeySessionId::from_core([7; 16]),
            &request(RELAY),
            &leg,
            &mut port,
        ),
        Err(SecurityKeyLeaseError::ClaimRefused(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused
        ))
    );
    assert!(port.calls.is_empty());
    assert_eq!(lease.state(), LeaseState::Idle);
}
