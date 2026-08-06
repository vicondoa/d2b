use d2b_provider_device_usbip::FirewallOwnershipMarker;

#[test]
fn marker_uses_only_the_provider_namespace_and_opaque_projection() {
    let marker = FirewallOwnershipMarker::from_core([3; 32]);
    assert_eq!(FirewallOwnershipMarker::namespace(), "d2b managed: usbip:");
    assert_eq!(marker.digest(), &[3; 32]);
    let rendered = format!("{marker:?}");
    assert!(!rendered.contains("3"));
}
