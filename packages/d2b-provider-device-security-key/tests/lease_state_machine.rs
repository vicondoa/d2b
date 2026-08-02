use d2b_contracts::v3::ResourceUid;
use d2b_provider_device_security_key::{
    GuestCid, LeaseState, PhysicalAuthorityLease, PhysicalUsbBackingClaim, PhysicalUsbBackingToken,
    RelayLaunchTicket, SecurityKeyCidTranslator, SecurityKeyEffectError, SecurityKeyEffectPort,
    SecurityKeyLease, SecurityKeyOpenIntent, SecurityKeySessionId,
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
fn cid_translation_round_trips_without_exposing_session_material() {
    let guest = GuestCid::new(0x0102_0304).unwrap();
    let translator = SecurityKeyCidTranslator::from_core(0x1020_3040).unwrap();
    let relay = translator.to_relay(guest);
    assert_eq!(translator.to_guest(relay).unwrap(), guest);
}
