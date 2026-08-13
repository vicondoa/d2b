use d2b_contracts::v3::{ResourceRef, ResourceUid};
use d2b_provider_device_security_key::{
    PhysicalAuthorityLease, PhysicalUsbBackingToken, RelayLaunchTicket, SecurityKeyAdmission,
    SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyLease, SecurityKeyOpenIntent,
    SecurityKeySessionId,
};

struct RecordingPort;

impl SecurityKeyEffectPort for RecordingPort {
    fn claim_physical_backing(
        &mut self,
        _: &d2b_provider_device_security_key::PhysicalUsbBackingClaim,
    ) -> Result<PhysicalAuthorityLease, SecurityKeyEffectError> {
        Ok(PhysicalAuthorityLease::from_core([1; 16]))
    }

    fn open_hidraw(
        &mut self,
        _: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError> {
        panic!("wrong-device authorization must refuse before hidraw open");
    }

    fn release_physical_backing(
        &mut self,
        _: PhysicalAuthorityLease,
    ) -> Result<(), SecurityKeyEffectError> {
        Ok(())
    }
}

#[test]
fn exact_device_and_subject_binding_is_checked_before_hidraw() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let holder = ResourceRef::parse("Guest/work-vm").unwrap();
    let admission = SecurityKeyAdmission::from_core(
        ResourceRef::parse("Zone/work").unwrap(),
        device.clone(),
        holder.clone(),
        PhysicalUsbBackingToken::from_core([7; 32]),
    );
    let mut lease = SecurityKeyLease::new_authorized(device.clone(), admission).unwrap();
    let wrong_device = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
    assert_eq!(
        lease.acquire_authorized(
            SecurityKeySessionId::from_core([3; 16]),
            wrong_device,
            &holder,
            &mut RecordingPort,
        ),
        Err(d2b_provider_device_security_key::SecurityKeyLeaseError::AuthorizationDenied)
    );
}
