use d2b_core::device_usbip_adapter::{
    USBIP_HOST_MODULE_DOMAIN, USBIP_NETWORK_RELAY_DOMAIN, UsbipCoreAdapter, UsbipCoreAdapterError,
};

#[test]
fn physical_backing_identity_is_shared_and_domain_separated() {
    let token = b"fake-usb-physical-identity";
    assert_eq!(
        UsbipCoreAdapter::physical_usb_backing_key(token),
        UsbipCoreAdapter::physical_usb_backing_key(token)
    );
    assert_ne!(
        UsbipCoreAdapter::physical_usb_backing_key(token),
        UsbipCoreAdapter::network_relay_key(token)
    );
    assert_eq!(USBIP_HOST_MODULE_DOMAIN, "d2b:usbip-host-module/v1");
    assert_eq!(USBIP_NETWORK_RELAY_DOMAIN, "d2b:usbip-network-relay/v1");
}

#[test]
fn provider_class_bypass_is_rejected_before_adapter_use() {
    assert_eq!(
        UsbipCoreAdapter::validate_provider_class("device-usbip"),
        Ok(())
    );
    assert_eq!(
        UsbipCoreAdapter::validate_provider_class("device-security-key"),
        Ok(())
    );
    assert_eq!(
        UsbipCoreAdapter::validate_provider_class("network-local"),
        Err(UsbipCoreAdapterError::ProviderClassBypass)
    );
}

#[test]
fn zone_and_opt_in_refuse_before_bundle_lookup() {
    assert_eq!(
        UsbipCoreAdapter::validate_zone("work", "personal", true),
        Err(UsbipCoreAdapterError::WrongZone)
    );
    assert_eq!(
        UsbipCoreAdapter::validate_zone("work", "work", false),
        Err(UsbipCoreAdapterError::ZoneNotOptedIn)
    );
    assert_eq!(
        UsbipCoreAdapter::validate_zone("work", "work", true),
        Ok(())
    );
}
