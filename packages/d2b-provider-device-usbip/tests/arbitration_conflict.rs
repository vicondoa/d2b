use d2b_contracts::v3::{DeviceArbitration, ResourceUid};
use d2b_provider_device_usbip::{PhysicalUsbBackingToken, UsbipArbitrator, UsbipClaimError};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[test]
fn second_exclusive_claim_is_rejected_before_effects() {
    let backing = PhysicalUsbBackingToken::from_core([7; 32]);
    let mut arbiter =
        UsbipArbitrator::new(DeviceArbitration::Exclusive, 1, backing.clone()).unwrap();
    arbiter
        .claim(uid("123e4567-e89b-42d3-a456-426614174000"), backing.clone())
        .unwrap();
    assert_eq!(
        arbiter.claim(uid("223e4567-e89b-42d3-a456-426614174001"), backing),
        Err(UsbipClaimError::ClaimConflict)
    );
}
