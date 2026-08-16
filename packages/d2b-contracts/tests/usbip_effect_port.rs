use d2b_contracts::{
    usbip_effect_port::{
        DeviceProbeResult, FirewallAction, FirewallGenerationFence, FirewallProjection,
        FirewallToken, LeaseToken, NetworkUid, PhysicalUsbBacking, TransientDetail, UsbBindingUid,
        UsbipEffectError,
    },
    v3::{ResourceBundleGenerationId, ResourceUid},
};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[test]
fn effect_port_identity_and_lease_debug_are_redacted() {
    let device = d2b_contracts::usbip_effect_port::DeviceUid::from_core(uid(
        "123e4567-e89b-42d3-a456-426614174000",
    ));
    let network = NetworkUid::from_core(uid("223e4567-e89b-42d3-a456-426614174001"));
    let binding = UsbBindingUid::from_core(uid("323e4567-e89b-42d3-a456-426614174002"));
    let generation =
        ResourceBundleGenerationId::parse(format!("sha256:{}", "a".repeat(64))).unwrap();
    let projection = FirewallProjection::new(
        network,
        binding,
        FirewallGenerationFence::from_core(generation),
        FirewallAction::Apply,
    );

    let rendered = format!(
        "{device:?}{projection:?}{:?}",
        LeaseToken::from_adapter([7; 16])
    );
    assert!(!rendered.contains("123e4567"));
    assert!(!rendered.contains("223e4567"));
    assert!(!rendered.contains("323e4567"));
    assert!(!rendered.contains('7'));
}

#[test]
fn firewall_action_and_effect_error_surface_are_closed() {
    assert_eq!(FirewallAction::Apply, FirewallAction::Apply);
    assert_ne!(FirewallAction::Apply, FirewallAction::Remove);
    assert_eq!(
        UsbipEffectError::Transient(TransientDetail::StaleGeneration).to_string(),
        "transient"
    );
    assert_eq!(
        format!(
            "{:?}",
            UsbipEffectError::Transient(TransientDetail::StaleGeneration)
        ),
        "UsbipEffectError::Transient(<redacted>)"
    );
    assert_eq!(DeviceProbeResult::Present, DeviceProbeResult::Present);
    let _ = PhysicalUsbBacking::from_core([9; 32]);
    let _ = FirewallToken::from_adapter([8; 16]);
}
