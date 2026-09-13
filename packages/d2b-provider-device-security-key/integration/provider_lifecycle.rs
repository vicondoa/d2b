//! integration-target: container
//!
//! Public Provider-boundary integration coverage using a fake Core effect
//! port. Host device opening and transport orchestration remain outside this
//! crate.

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_provider_device_security_key::{
    FrontendProcessDeclaration, LeaseState, PhysicalAuthorityLease, PhysicalUsbBackingClaim,
    PhysicalUsbBackingToken, RelayLaunchTicket, SecurityKeyController, SecurityKeyEffectError,
    SecurityKeyEffectPort, SecurityKeyOpenIntent, SecurityKeyProcessRole, SecurityKeySessionId,
    security_key_process_name,
};

#[derive(Default)]
struct FakeCore {
    events: Vec<&'static str>,
}

impl SecurityKeyEffectPort for FakeCore {
    fn claim_physical_backing(
        &mut self,
        claim: &PhysicalUsbBackingClaim,
    ) -> Result<PhysicalAuthorityLease, SecurityKeyEffectError> {
        assert_eq!(claim.authority_scope, "Host");
        assert_eq!(claim.backing_class, "physical-usb-backing");
        self.events.push("claim");
        Ok(PhysicalAuthorityLease::from_core([1; 16]))
    }

    fn open_hidraw(
        &mut self,
        intent: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError> {
        assert_eq!(
            intent.session_id(),
            &SecurityKeySessionId::from_core([7; 16])
        );
        self.events.push("open");
        Ok(RelayLaunchTicket::from_core([2; 16]))
    }

    fn release_physical_backing(
        &mut self,
        _lease: PhysicalAuthorityLease,
    ) -> Result<(), SecurityKeyEffectError> {
        self.events.push("release");
        Ok(())
    }
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("fixture UID is canonical")
}

#[test]
fn provider_lifecycle_keeps_core_effects_and_guest_placement_bounded() {
    let device_uid = uid("123e4567-e89b-42d3-a456-426614174000");
    let session = SecurityKeySessionId::from_core([7; 16]);
    let guest = ResourceRef::parse("Guest/corp-vm").expect("fixture Guest is canonical");
    let frontend =
        FrontendProcessDeclaration::new(&device_uid, guest.clone()).expect("Guest is accepted");

    assert_eq!(frontend.execution_ref(), &guest);
    assert_eq!(frontend.role(), SecurityKeyProcessRole::GuestFrontend);
    assert_eq!(frontend.domain(), "user");
    assert_eq!(
        security_key_process_name(&device_uid, SecurityKeyProcessRole::HostRelay).unwrap(),
        "device-123e4567e89b-sk-relay"
    );

    let backing = PhysicalUsbBackingClaim::from_core(PhysicalUsbBackingToken::from_core([3; 32]));
    let mut controller = SecurityKeyController::new(device_uid.clone(), backing, 8)
        .expect("bounded session-ring capacity");
    let mut core = FakeCore::default();

    controller
        .acquire(session, device_uid, &mut core)
        .expect("Core admits the opaque backing");
    assert_eq!(controller.lease().state(), LeaseState::Active);
    assert_eq!(core.events, ["claim", "open"]);

    controller
        .complete(&mut core)
        .expect("terminal release succeeds");
    assert_eq!(controller.lease().state(), LeaseState::Completed);
    assert_eq!(core.events, ["claim", "open", "release"]);
}
