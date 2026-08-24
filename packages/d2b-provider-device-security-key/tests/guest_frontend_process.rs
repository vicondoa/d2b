use d2b_contracts_provider::v3::semantic_services::child_resources::{
    BindingChildKind, BindingChildPlacement,
};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_provider_device_security_key::{
    FrontendProcessDeclaration, SecurityKeyController, SecurityKeyProcessRole,
    security_key_process_name,
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

#[test]
fn binding_children_require_authored_service_and_are_deleted_endpoint_first() {
    let children = SecurityKeyController::child_resources(
        &ResourceRef::parse("security-key.d2bus.org.SecurityKeyBinding/yubikey").unwrap(),
        &ResourceRef::parse("security-key.d2bus.org.SecurityKeyService/yubikey").unwrap(),
        &ResourceRef::parse("Guest/corp-vm").unwrap(),
    )
    .unwrap();
    assert_eq!(children.at(BindingChildPlacement::Host).count(), 0);
    assert_eq!(children.at(BindingChildPlacement::Guest).count(), 2);
    assert_eq!(
        children
            .teardown_order()
            .iter()
            .map(|child| child.kind())
            .collect::<Vec<_>>(),
            vec![
                BindingChildKind::Endpoint,
                BindingChildKind::Process,
            ]
    );
    assert!(
        SecurityKeyController::child_resources(
            &ResourceRef::parse("security-key.d2bus.org.SecurityKeyBinding/yubikey").unwrap(),
            &ResourceRef::parse("security-key.d2bus.org.SecurityKeyService/yubikey").unwrap(),
            &ResourceRef::parse("User/alice").unwrap(),
        )
        .is_err()
    );
}
