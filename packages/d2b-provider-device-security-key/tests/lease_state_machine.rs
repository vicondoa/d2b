use d2b_contracts_resource::v3::ResourceUid;
use d2b_provider_device_security_key::{
    LeaseState, MAX_SESSION_RING_SIZE, MIN_SESSION_RING_SIZE, PhysicalAuthorityLease,
    PhysicalUsbBackingClaim, PhysicalUsbBackingToken, RelayLaunchTicket,
    SECURITY_KEY_BINDING_RESOURCE_TYPE, SECURITY_KEY_SERVICE_RESOURCE_TYPE, SecurityKeyController,
    SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyLease, SecurityKeyOpenIntent,
    SecurityKeySessionId, security_key_runner_contract,
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
