use d2b_contracts_resource::v3::ResourceUid;
use d2b_provider_device_usbip::{
    AttachProcessIdentity, AttachmentObservation, BindingIdentity, BindingLifecycleError,
    BindingProxyLease, BindingSlotLease, OwnedBusBinding, PhysicalAuthorityLease, ProductionPort,
    ServiceLifecycleError, ServiceRelayLease, UsbipBrokerDispatcher, UsbipSupervisor,
};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[derive(Default)]
struct RecordingDispatcher {
    calls: Vec<&'static str>,
}

impl UsbipBrokerDispatcher for RecordingDispatcher {
    fn reserve_physical(
        &mut self,
        _: &ResourceUid,
    ) -> Result<PhysicalAuthorityLease, ServiceLifecycleError> {
        self.calls.push("reserve-physical");
        Ok(PhysicalAuthorityLease::from_adapter([1; 16]))
    }

    fn reserve_relay(
        &mut self,
        _: &ResourceUid,
    ) -> Result<ServiceRelayLease, ServiceLifecycleError> {
        self.calls.push("reserve-relay");
        Ok(ServiceRelayLease::from_adapter([2; 16]))
    }

    fn bind_owned(
        &mut self,
        _: &PhysicalAuthorityLease,
    ) -> Result<OwnedBusBinding, ServiceLifecycleError> {
        self.calls.push("bind");
        Ok(OwnedBusBinding::from_adapter([3; 16]))
    }

    fn unbind_owned(&mut self, _: &OwnedBusBinding) -> Result<(), ServiceLifecycleError> {
        self.calls.push("unbind");
        Ok(())
    }

    fn release_relay(&mut self, _: ServiceRelayLease) -> Result<(), ServiceLifecycleError> {
        self.calls.push("release-relay");
        Ok(())
    }

    fn release_physical(&mut self, _: PhysicalAuthorityLease) -> Result<(), ServiceLifecycleError> {
        self.calls.push("release-physical");
        Ok(())
    }

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

    fn ensure_attach_process(
        &mut self,
        _: &BindingIdentity,
        _: &BindingProxyLease,
    ) -> Result<AttachProcessIdentity, BindingLifecycleError> {
        self.calls.push("ensure-process");
        Ok(AttachProcessIdentity::from_adapter(7, 11))
    }

    fn observe_attach_process(
        &mut self,
        _: &BindingIdentity,
        _: &AttachProcessIdentity,
    ) -> Result<AttachmentObservation, BindingLifecycleError> {
        self.calls.push("observe-process");
        Ok(AttachmentObservation::Matching {
            slot: BindingSlotLease::from_adapter([4; 16]),
            proxy: BindingProxyLease::from_adapter([5; 16]),
        })
    }

    fn delete_guest_endpoint(
        &mut self,
        _: &BindingIdentity,
        _: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError> {
        self.calls.push("delete-endpoint");
        Ok(())
    }

    fn delete_attach_process(
        &mut self,
        _: &BindingIdentity,
        _: &AttachProcessIdentity,
    ) -> Result<(), BindingLifecycleError> {
        self.calls.push("delete-process");
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
fn production_port_keeps_binding_teardown_before_service_release() {
    let mut port = ProductionPort::new(RecordingDispatcher::default());
    let mut service = d2b_provider_device_usbip::ServiceLifecycle::new(
        uid("123e4567-e89b-42d3-a456-426614174000"),
        uid("223e4567-e89b-42d3-a456-426614174001"),
    );
    service
        .activate(true, uid("123e4567-e89b-42d3-a456-426614174000"), &mut port)
        .unwrap();
    let mut supervisor = UsbipSupervisor::new(service);
    supervisor
        .add_binding(d2b_provider_device_usbip::BindingLifecycle::new(
            uid("123e4567-e89b-42d3-a456-426614174000"),
            uid("123e4567-e89b-42d3-a456-426614174000"),
            BindingIdentity::from_controller(uid("323e4567-e89b-42d3-a456-426614174002")),
        ))
        .unwrap();
    supervisor.activate_binding(0, &mut port).unwrap();
    supervisor.finalize(&mut port).unwrap();

    assert_eq!(
        port.dispatcher().calls,
        [
            "reserve-physical",
            "reserve-relay",
            "bind",
            "slot",
            "proxy",
            "ensure-process",
            "observe-process",
            "delete-endpoint",
            "delete-process",
            "close-proxy",
            "release-slot",
            "unbind",
            "release-relay",
            "release-physical",
        ]
    );
}
