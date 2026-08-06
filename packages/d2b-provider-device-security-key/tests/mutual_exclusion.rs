use d2b_contracts::v3::ResourceUid;
use d2b_provider_device_security_key::{
    PhysicalAuthorityLease, PhysicalUsbBackingClaim, PhysicalUsbBackingToken, RelayLaunchTicket,
    SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyLease, SecurityKeyOpenIntent,
    SecurityKeySessionId,
};

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

#[test]
fn physical_backing_conflict_is_reported_before_any_hidraw_effect() {
    let token = PhysicalUsbBackingToken::from_core([9; 32]);
    let claim = PhysicalUsbBackingClaim::from_core(token.clone());
    assert_eq!(claim.token().as_bytes(), &[9; 32]);
    let mut lease = SecurityKeyLease::new(
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        claim,
    );
    assert!(
        lease
            .acquire(
                SecurityKeySessionId::from_core([4; 16]),
                ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
                &mut ConflictPort,
            )
            .is_err()
    );
}
