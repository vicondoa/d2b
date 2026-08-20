use d2b_contracts_zone_session::v3::ResourceUid;
use d2b_provider_device_usbip::{
    AttachProcessIdentity, AttachmentObservation, BindingIdentity, BindingLifecycle,
    BindingLifecycleError, BindingPort, BindingProxyLease, BindingSlotLease, ServiceLifecycle,
    ServiceLifecycleError, ServicePhase, ServicePort, UsbipSupervisor,
};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

struct FakePort {
    calls: Vec<&'static str>,
    fail_physical: bool,
    fail_relay: bool,
    observation: AttachmentObservation,
}

impl Default for FakePort {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            fail_physical: false,
            fail_relay: false,
            observation: AttachmentObservation::Matching {
                slot: BindingSlotLease::from_adapter([4; 16]),
                proxy: BindingProxyLease::from_adapter([5; 16]),
            },
        }
    }
}

impl ServicePort for FakePort {
    fn reserve_physical(
        &mut self,
        _: &ResourceUid,
    ) -> Result<d2b_provider_device_usbip::PhysicalAuthorityLease, ServiceLifecycleError> {
        self.calls.push("reserve-physical");
        if self.fail_physical {
            Err(ServiceLifecycleError::PhysicalAuthorityConflict)
        } else {
            Ok(d2b_provider_device_usbip::PhysicalAuthorityLease::from_adapter([1; 16]))
        }
    }

    fn reserve_relay(
        &mut self,
        _: &ResourceUid,
    ) -> Result<d2b_provider_device_usbip::ServiceRelayLease, ServiceLifecycleError> {
        self.calls.push("reserve-relay");
        if self.fail_relay {
            Err(ServiceLifecycleError::RelayAuthorityConflict)
        } else {
            Ok(d2b_provider_device_usbip::ServiceRelayLease::from_adapter(
                [2; 16],
            ))
        }
    }

    fn bind_owned(
        &mut self,
        _: &d2b_provider_device_usbip::PhysicalAuthorityLease,
    ) -> Result<d2b_provider_device_usbip::OwnedBusBinding, ServiceLifecycleError> {
        self.calls.push("bind");
        Ok(d2b_provider_device_usbip::OwnedBusBinding::from_adapter(
            [3; 16],
        ))
    }

    fn unbind_owned(
        &mut self,
        _: &d2b_provider_device_usbip::OwnedBusBinding,
    ) -> Result<(), ServiceLifecycleError> {
        self.calls.push("unbind");
        Ok(())
    }

    fn release_relay(
        &mut self,
        _: d2b_provider_device_usbip::ServiceRelayLease,
    ) -> Result<(), ServiceLifecycleError> {
        self.calls.push("release-relay");
        Ok(())
    }

    fn release_physical(
        &mut self,
        _: d2b_provider_device_usbip::PhysicalAuthorityLease,
    ) -> Result<(), ServiceLifecycleError> {
        self.calls.push("release-physical");
        Ok(())
    }
}

impl BindingPort for FakePort {
    fn acquire_slot(
        &mut self,
        _: &BindingIdentity,
    ) -> Result<BindingSlotLease, BindingLifecycleError> {
        self.calls.push("slot");
        Ok(BindingSlotLease::from_adapter([4; 16]))
    }

    fn start_proxy(
        &mut self,
        _: &BindingIdentity,
        _: &BindingSlotLease,
    ) -> Result<BindingProxyLease, BindingLifecycleError> {
        self.calls.push("proxy");
        Ok(BindingProxyLease::from_adapter([5; 16]))
    }

    fn spawn_attach_runner(
        &mut self,
        _: &BindingIdentity,
        _: &BindingProxyLease,
    ) -> Result<AttachProcessIdentity, BindingLifecycleError> {
        self.calls.push("spawn-attach");
        Ok(AttachProcessIdentity::from_adapter(7, 11))
    }

    fn observe_attach_runner(
        &mut self,
        _: &BindingIdentity,
        _: &AttachProcessIdentity,
    ) -> Result<AttachmentObservation, BindingLifecycleError> {
        self.calls.push("observe-attach");
        Ok(self.observation.clone())
    }

    fn detach_guest(
        &mut self,
        _: &BindingIdentity,
        _: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError> {
        self.calls.push("detach-guest");
        Ok(())
    }

    fn close_attach_runner(
        &mut self,
        _: &BindingIdentity,
        _: &AttachProcessIdentity,
    ) -> Result<(), BindingLifecycleError> {
        self.calls.push("close-attach");
        Ok(())
    }

    fn close_proxy(
        &mut self,
        _: &BindingIdentity,
        _: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError> {
        self.calls.push("close-proxy");
        Ok(())
    }

    fn release_slot(
        &mut self,
        _: &BindingIdentity,
        _: &BindingSlotLease,
    ) -> Result<(), BindingLifecycleError> {
        self.calls.push("release-slot");
        Ok(())
    }
}

#[test]
fn wrong_zone_and_opt_out_refuse_before_authority_or_bind() {
    let service_zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let mut port = FakePort::default();
    let mut service = ServiceLifecycle::new(
        service_zone.clone(),
        uid("223e4567-e89b-42d3-a456-426614174001"),
    );

    assert_eq!(
        service.activate(false, service_zone.clone(), &mut port),
        Err(ServiceLifecycleError::ZoneNotOptedIn)
    );
    assert!(port.calls.is_empty());
    assert_eq!(
        service.activate(true, uid("323e4567-e89b-42d3-a456-426614174002"), &mut port),
        Err(ServiceLifecycleError::WrongZone)
    );
    assert!(port.calls.is_empty());
}

#[test]
fn authority_conflicts_happen_before_bind() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let mut physical_conflict = FakePort {
        fail_physical: true,
        ..Default::default()
    };
    let mut service =
        ServiceLifecycle::new(zone.clone(), uid("223e4567-e89b-42d3-a456-426614174001"));
    assert_eq!(
        service.activate(true, zone.clone(), &mut physical_conflict),
        Err(ServiceLifecycleError::PhysicalAuthorityConflict)
    );
    assert_eq!(physical_conflict.calls, ["reserve-physical"]);

    let mut relay_conflict = FakePort {
        fail_relay: true,
        ..Default::default()
    };
    let mut service =
        ServiceLifecycle::new(zone.clone(), uid("223e4567-e89b-42d3-a456-426614174001"));
    assert_eq!(
        service.activate(true, zone, &mut relay_conflict),
        Err(ServiceLifecycleError::RelayAuthorityConflict)
    );
    assert_eq!(relay_conflict.calls, ["reserve-physical", "reserve-relay"]);
}

#[test]
fn matching_restart_adopts_and_stale_identity_quarantines_without_effects() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let mut port = FakePort::default();
    let service = ServiceLifecycle::new(zone.clone(), uid("223e4567-e89b-42d3-a456-426614174001"));
    let mut supervisor = UsbipSupervisor::new(service);
    supervisor
        .add_binding(BindingLifecycle::new(
            zone.clone(),
            zone.clone(),
            BindingIdentity::from_controller(uid("323e4567-e89b-42d3-a456-426614174002")),
        ))
        .unwrap();
    supervisor
        .adopt_binding(0, AttachProcessIdentity::from_adapter(7, 11), &mut port)
        .unwrap();
    assert_eq!(port.calls, ["observe-attach"]);
    supervisor.finalize(&mut port).unwrap();
    assert_eq!(
        port.calls,
        [
            "observe-attach",
            "detach-guest",
            "close-attach",
            "close-proxy",
            "release-slot"
        ]
    );

    let service = ServiceLifecycle::new(zone.clone(), uid("423e4567-e89b-42d3-a456-426614174003"));
    let mut supervisor = UsbipSupervisor::new(service);
    supervisor
        .add_binding(BindingLifecycle::new(
            zone.clone(),
            zone,
            BindingIdentity::from_controller(uid("523e4567-e89b-42d3-a456-426614174004")),
        ))
        .unwrap();
    port.calls.clear();
    port.observation = AttachmentObservation::StaleIdentity;
    supervisor
        .adopt_binding(0, AttachProcessIdentity::from_adapter(8, 12), &mut port)
        .unwrap();
    assert_eq!(port.calls, ["observe-attach"]);
    assert_eq!(
        supervisor.activate_binding(0, &mut port),
        Err(BindingLifecycleError::Quarantined)
    );
    assert_eq!(
        supervisor.finalize(&mut port),
        Err(d2b_provider_device_usbip::SupervisorFinalizeError::Binding(
            BindingLifecycleError::Quarantined
        ))
    );
    assert_eq!(port.calls, ["observe-attach"]);
}

#[test]
fn missing_restart_identity_drops_slot_and_proxy_before_reactivate() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let mut port = FakePort::default();
    let mut service =
        ServiceLifecycle::new(zone.clone(), uid("223e4567-e89b-42d3-a456-426614174001"));
    service.activate(true, zone.clone(), &mut port).unwrap();
    let mut supervisor = UsbipSupervisor::new(service);
    supervisor
        .add_binding(BindingLifecycle::new(
            zone.clone(),
            zone,
            BindingIdentity::from_controller(uid("323e4567-e89b-42d3-a456-426614174002")),
        ))
        .unwrap();
    supervisor.activate_binding(0, &mut port).unwrap();
    port.calls.clear();
    port.observation = AttachmentObservation::Missing;
    supervisor
        .adopt_binding(0, AttachProcessIdentity::from_adapter(7, 11), &mut port)
        .unwrap();
    supervisor.activate_binding(0, &mut port).unwrap();
    assert_eq!(
        port.calls,
        ["observe-attach", "slot", "proxy", "spawn-attach",]
    );
}

#[test]
fn binding_closes_its_process_before_service_unbinds_and_releases_authority() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let mut port = FakePort::default();
    let mut service =
        ServiceLifecycle::new(zone.clone(), uid("223e4567-e89b-42d3-a456-426614174001"));
    service.activate(true, zone.clone(), &mut port).unwrap();
    let binding = BindingLifecycle::new(
        zone.clone(),
        zone,
        BindingIdentity::from_controller(uid("323e4567-e89b-42d3-a456-426614174002")),
    );
    let mut supervisor = UsbipSupervisor::new(service);
    supervisor.add_binding(binding).unwrap();
    supervisor.activate_binding(0, &mut port).unwrap();
    supervisor.finalize(&mut port).unwrap();

    assert_eq!(supervisor.service().phase(), ServicePhase::Closed);
    assert_eq!(
        port.calls,
        [
            "reserve-physical",
            "reserve-relay",
            "bind",
            "slot",
            "proxy",
            "spawn-attach",
            "detach-guest",
            "close-attach",
            "close-proxy",
            "release-slot",
            "unbind",
            "release-relay",
            "release-physical",
        ]
    );
}

#[test]
fn one_binding_can_finalize_without_unbinding_the_shared_service() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let mut port = FakePort::default();
    let mut service =
        ServiceLifecycle::new(zone.clone(), uid("223e4567-e89b-42d3-a456-426614174001"));
    service.activate(true, zone.clone(), &mut port).unwrap();
    let mut supervisor = UsbipSupervisor::new(service);
    for value in [
        "323e4567-e89b-42d3-a456-426614174002",
        "423e4567-e89b-42d3-a456-426614174003",
    ] {
        supervisor
            .add_binding(BindingLifecycle::new(
                zone.clone(),
                zone.clone(),
                BindingIdentity::from_controller(uid(value)),
            ))
            .unwrap();
    }
    supervisor.activate_binding(0, &mut port).unwrap();
    supervisor.activate_binding(1, &mut port).unwrap();
    supervisor.finalize_binding(0, &mut port).unwrap();

    assert_eq!(supervisor.service().phase(), ServicePhase::Bound);
    assert!(!port.calls.contains(&"unbind"));

    supervisor.finalize(&mut port).unwrap();
    assert_eq!(supervisor.service().phase(), ServicePhase::Closed);
}

#[test]
fn foreign_zone_binding_is_refused_before_recovery_observation() {
    let service_zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let foreign_zone = uid("223e4567-e89b-42d3-a456-426614174001");
    let service = ServiceLifecycle::new(
        service_zone.clone(),
        uid("323e4567-e89b-42d3-a456-426614174002"),
    );
    let mut supervisor = UsbipSupervisor::new(service);
    assert_eq!(
        supervisor.add_binding(BindingLifecycle::new(
            service_zone,
            foreign_zone,
            BindingIdentity::from_controller(uid("423e4567-e89b-42d3-a456-426614174003")),
        )),
        Err(BindingLifecycleError::WrongZone)
    );
    let mut port = FakePort::default();
    assert_eq!(
        supervisor.adopt_binding(0, AttachProcessIdentity::from_adapter(7, 11), &mut port),
        Err(BindingLifecycleError::AdmissionDenied)
    );
    assert!(port.calls.is_empty());
}
