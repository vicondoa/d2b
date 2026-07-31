use d2b_contracts::{
    broker_wire::{ApplyNftablesProjectionRequest, NftablesProjectionAction},
    usbip::{SYSFS_BUS_ID_MAX, validate_bus_id},
};
use d2b_provider_device_usbip::{
    AttachmentActivation, AttachmentCommand, FirewallProjectionAction, PROVIDER_REF,
    USB_BINDING_RESOURCE_TYPE, USB_SERVICE_RESOURCE_TYPE, UsbipEffectError, UsbipWorkerClass,
    UsbipWorkerDeclaration,
};

#[test]
fn provider_identity_and_provider_neutral_usb_types_are_fixed() {
    assert_eq!(PROVIDER_REF, "Provider/device-usbip");
    assert_eq!(USB_SERVICE_RESOURCE_TYPE, "usb.d2bus.org.UsbService");
    assert_eq!(USB_BINDING_RESOURCE_TYPE, "usb.d2bus.org.UsbBinding");
}

#[test]
fn projection_action_is_closed_and_matches_the_shared_broker_contract() {
    use core::any::TypeId;

    assert_eq!(
        FirewallProjectionAction::parse("Apply"),
        Ok(FirewallProjectionAction::Apply)
    );
    assert_eq!(
        FirewallProjectionAction::parse("Remove"),
        Ok(FirewallProjectionAction::Remove)
    );
    assert_eq!(
        FirewallProjectionAction::parse("Replace"),
        Err(UsbipEffectError::UnknownProjectionAction)
    );
    assert_ne!(
        NftablesProjectionAction::Apply,
        NftablesProjectionAction::Remove
    );
    assert_ne!(
        TypeId::of::<ApplyNftablesProjectionRequest>(),
        TypeId::of::<NftablesProjectionAction>()
    );
}

#[test]
fn adapted_bus_id_validation_rejects_unsafe_and_noncanonical_values() {
    let max = format!("1-{}", "1.".repeat(14) + "1");
    assert_eq!(max.len(), SYSFS_BUS_ID_MAX);
    assert!(validate_bus_id(&max).is_ok());
    for rejected in ["", "01", "1-02", "1-2.", "1-2/a", "1-2;run"] {
        assert!(validate_bus_id(rejected).is_err());
    }
}

#[test]
fn only_long_lived_workers_are_process_declarations() {
    let backend = UsbipWorkerDeclaration::for_class(UsbipWorkerClass::HostBackend);
    let relay = UsbipWorkerDeclaration::for_class(UsbipWorkerClass::NetworkRelay);
    let proxy = UsbipWorkerDeclaration::for_class(UsbipWorkerClass::BindingProxy);
    assert_eq!(backend.template(), "usbip-daemon");
    assert_eq!(relay.template(), "usbip-relay");
    assert_eq!(proxy.template(), "usbip-guest-proxy");
    assert_eq!(backend.placement(), "host");
    assert_eq!(proxy.placement(), "guest");

    assert_eq!(
        AttachmentCommand::Attach(AttachmentActivation::Declared),
        AttachmentCommand::Attach(AttachmentActivation::Declared)
    );
    assert_ne!(
        AttachmentCommand::Attach(AttachmentActivation::Explicit),
        AttachmentCommand::Detach
    );
}
