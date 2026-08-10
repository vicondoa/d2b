use d2b_contracts::v3::{ResourceRef, ResourceUid};
use d2b_provider_device_security_key::{
    FrontendProcessDeclaration, SecurityKeyProcessRole, security_key_process_name,
};

#[test]
fn frontend_process_is_guest_placed_and_uid_derived() {
    let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let guest = ResourceRef::parse("Guest/corp-vm").unwrap();
    let declaration = FrontendProcessDeclaration::new(&uid, guest).unwrap();
    assert_eq!(declaration.name(), "device-123e4567e89b-sk-frontend");
    assert_eq!(declaration.role(), SecurityKeyProcessRole::GuestFrontend);
    assert_eq!(declaration.domain(), "user");
    assert_eq!(
        security_key_process_name(&uid, SecurityKeyProcessRole::HostRelay).unwrap(),
        "device-123e4567e89b-sk-relay"
    );
}
