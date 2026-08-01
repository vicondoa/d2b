use d2b_contracts::v3::{ResourceGeneration, ResourceUid};
use d2b_provider_device_usbip::{
    FirewallConfirmation, FirewallDigest, FirewallObservation, FirewallProjectionAction,
    FirewallProjectionIntent, FirewallToken, NetworkDependency, RelayAuthorityLease,
    ScopedResourceUid, UsbipController, UsbipControllerError, UsbipEffectError, UsbipEffectPort,
    UsbipServicePhase,
};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[derive(Default)]
struct FakePort {
    actions: Vec<FirewallProjectionAction>,
    fences: Vec<(u64, u64)>,
    release_results: Vec<Result<FirewallConfirmation, UsbipEffectError>>,
    relay_acquires: usize,
    relay_releases: usize,
    effects: usize,
}

impl UsbipEffectPort for FakePort {
    fn acquire_relay(&mut self, _: &ResourceUid) -> Result<RelayAuthorityLease, UsbipEffectError> {
        self.effects += 1;
        self.relay_acquires += 1;
        Ok(RelayAuthorityLease::from_adapter([1; 16]))
    }

    fn mutate_firewall(
        &mut self,
        intent: &FirewallProjectionIntent,
        _: Option<&FirewallToken>,
    ) -> Result<FirewallConfirmation, UsbipEffectError> {
        self.effects += 1;
        self.actions.push(intent.action());
        self.fences.push((
            intent.expected().network_generation().get(),
            intent.expected().service_generation().get(),
        ));
        assert_eq!(
            intent.device_uid(),
            &uid("423e4567-e89b-42d3-a456-426614174003")
        );
        assert_eq!(
            intent.network_uid(),
            &uid("323e4567-e89b-42d3-a456-426614174002")
        );
        match intent.action() {
            FirewallProjectionAction::Apply => Ok(FirewallConfirmation::applied(
                FirewallToken::from_adapter([2; 16]),
                FirewallDigest::from_adapter([3; 32]),
            )),
            FirewallProjectionAction::Remove => {
                if self.release_results.is_empty() {
                    Ok(FirewallConfirmation::removed())
                } else {
                    self.release_results.remove(0)
                }
            }
        }
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
        self.relay_releases += 1;
        Ok(())
    }
}

fn controller_and_network(zone: ResourceUid) -> (UsbipController, NetworkDependency) {
    let service = ScopedResourceUid::new(zone.clone(), uid("223e4567-e89b-42d3-a456-426614174001"));
    let network = NetworkDependency::new(
        ScopedResourceUid::new(zone, uid("323e4567-e89b-42d3-a456-426614174002")),
        ResourceGeneration::new(7).unwrap(),
        true,
    );
    (
        UsbipController::new(
            service,
            ResourceGeneration::new(4).unwrap(),
            uid("423e4567-e89b-42d3-a456-426614174003"),
        ),
        network,
    )
}

#[test]
fn apply_observe_and_confirmed_remove_use_the_closed_projection_actions() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let (mut controller, network) = controller_and_network(zone);
    let mut port = FakePort::default();
    controller.reconcile(network, &mut port).unwrap();
    assert_eq!(controller.phase(), UsbipServicePhase::Ready);
    assert!(controller.relay_authority_retained());
    assert!(controller.firewall_status_retained());
    controller.observe(&mut port).unwrap();
    controller.finalize(&mut port).unwrap();
    assert_eq!(
        port.actions,
        [
            FirewallProjectionAction::Apply,
            FirewallProjectionAction::Remove
        ]
    );
    assert_eq!(port.relay_releases, 1);
    assert_eq!(port.relay_acquires, 1);
    assert_eq!(port.fences, [(7, 4), (7, 4)]);
    assert!(!controller.relay_authority_retained());
    assert!(!controller.firewall_status_retained());
}

#[test]
fn transient_remove_retains_token_status_and_relay_until_retry_confirms() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let (mut controller, network) = controller_and_network(zone);
    let mut port = FakePort::default();
    controller.reconcile(network, &mut port).unwrap();
    port.release_results = vec![
        Err(UsbipEffectError::Transient),
        Ok(FirewallConfirmation::validated_absent()),
    ];
    assert_eq!(
        controller.finalize(&mut port),
        Err(UsbipControllerError::Effect(UsbipEffectError::Transient))
    );
    assert_eq!(controller.phase(), UsbipServicePhase::Releasing);
    assert!(controller.relay_authority_retained());
    assert!(controller.firewall_status_retained());
    assert_eq!(port.relay_releases, 0);

    controller.finalize(&mut port).unwrap();
    assert!(!controller.relay_authority_retained());
    assert!(!controller.firewall_status_retained());
    assert_eq!(port.relay_releases, 1);
}

#[test]
fn foreign_marker_blocks_release_and_preserves_all_authority() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let (mut controller, network) = controller_and_network(zone);
    let mut port = FakePort::default();
    controller.reconcile(network, &mut port).unwrap();
    port.release_results = vec![Err(UsbipEffectError::FirewallForeignConflict)];
    assert_eq!(
        controller.finalize(&mut port),
        Err(UsbipControllerError::Effect(
            UsbipEffectError::FirewallForeignConflict
        ))
    );
    assert_eq!(controller.phase(), UsbipServicePhase::Blocked);
    assert!(controller.relay_authority_retained());
    assert!(controller.firewall_status_retained());
    assert_eq!(port.relay_releases, 0);
}
