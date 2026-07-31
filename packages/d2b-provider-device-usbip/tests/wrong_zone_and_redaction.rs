use d2b_contracts::v3::{ResourceGeneration, ResourceUid};
use d2b_provider_device_usbip::{
    FirewallConfirmation, FirewallDigest, FirewallObservation, FirewallProjectionIntent,
    FirewallToken, NetworkDependency, RelayAuthorityLease, ScopedResourceUid, UsbipController,
    UsbipControllerError, UsbipEffectError, UsbipEffectPort, UsbipMetricLabels, UsbipOperation,
    UsbipOutcome,
};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[derive(Default)]
struct CountingPort {
    effects: usize,
}

impl UsbipEffectPort for CountingPort {
    fn acquire_relay(&mut self, _: &ResourceUid) -> Result<RelayAuthorityLease, UsbipEffectError> {
        self.effects += 1;
        Ok(RelayAuthorityLease::from_adapter([1; 16]))
    }

    fn mutate_firewall(
        &mut self,
        _: &FirewallProjectionIntent,
        _: Option<&FirewallToken>,
    ) -> Result<FirewallConfirmation, UsbipEffectError> {
        self.effects += 1;
        Ok(FirewallConfirmation::applied(
            FirewallToken::from_adapter([2; 16]),
            FirewallDigest::from_adapter([3; 32]),
        ))
    }

    fn observe_firewall(
        &mut self,
        _: &FirewallProjectionIntent,
        _: &FirewallToken,
    ) -> Result<FirewallObservation, UsbipEffectError> {
        self.effects += 1;
        Ok(FirewallObservation::new(
            true,
            FirewallDigest::from_adapter([3; 32]),
        ))
    }

    fn release_relay(&mut self, _: RelayAuthorityLease) -> Result<(), UsbipEffectError> {
        self.effects += 1;
        Ok(())
    }
}

#[test]
fn wrong_zone_network_is_rejected_before_relay_or_firewall_effect() {
    let service_zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let network_zone = uid("223e4567-e89b-42d3-a456-426614174001");
    let mut controller = UsbipController::new(
        ScopedResourceUid::new(service_zone, uid("323e4567-e89b-42d3-a456-426614174002")),
        ResourceGeneration::new(1).unwrap(),
        uid("423e4567-e89b-42d3-a456-426614174003"),
    );
    let network = NetworkDependency::new(
        ScopedResourceUid::new(network_zone, uid("523e4567-e89b-42d3-a456-426614174004")),
        ResourceGeneration::new(1).unwrap(),
        true,
    );
    let mut port = CountingPort::default();
    assert_eq!(
        controller.reconcile(network, &mut port),
        Err(UsbipControllerError::Effect(UsbipEffectError::WrongZone))
    );
    assert_eq!(port.effects, 0);
}

#[test]
fn debug_errors_and_metric_label_values_are_identity_free() {
    let zone_canary = "123e4567-e89b-42d3-a456-426614174000";
    let resource_canary = "223e4567-e89b-42d3-a456-426614174001";
    let scoped = ScopedResourceUid::new(uid(zone_canary), uid(resource_canary));
    let labels = UsbipMetricLabels::new(
        UsbipOperation::RemoveFirewall,
        UsbipOutcome::Blocked,
        Some(UsbipEffectError::FirewallForeignConflict),
    );
    let rendered = format!(
        "{scoped:?} {:?} {} {labels:?}",
        UsbipEffectError::FirewallForeignConflict,
        UsbipEffectError::FirewallForeignConflict,
    );
    for canary in [zone_canary, resource_canary] {
        assert!(!rendered.contains(canary));
    }
    assert_eq!(labels.provider, "device-usbip");
    assert_eq!(labels.component, "service-controller");
    assert_eq!(labels.operation, "remove-firewall");
    assert_eq!(labels.outcome, "blocked");
    assert_eq!(labels.error, "firewall-foreign-conflict");
}
