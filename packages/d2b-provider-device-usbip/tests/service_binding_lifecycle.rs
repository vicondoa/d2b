//! USB Service and Binding lifecycle ownership.
//!
//! A Service owns the physical backing and the per-Network relay; a Binding owns
//! its Guest attachment, private proxy, and Service slot. The converted drain
//! keeps that split and proves the order under the graph's ownership: every
//! Binding closes its Guest Endpoint and private proxy first, the relay leg is
//! stopped next, and only then is the `Device` source's reservation handed back.
//! A step that does not confirm stops the drain, so the reservation is never
//! released while something can still reach the backing.

use d2b_contracts_resource::v3::{
    BindingArbitration, BindingAuthorization, BindingContractError, BindingEvidence,
    BindingLifecycleState, BindingObservation, CompletionCondition, DesiredDigest, DesiredRevision,
    DeviceAuthorityKey, DeviceClaimRequest, DeviceFunction, EndpointAttachmentKind, FreshnessTuple,
    ReleaseOutcome, RequestedRights, ResourceRef, ResourceUid, SourceAdmission, SourceReservation,
    StoreIncarnation, ZoneId, admit_binding_request, execution_policy::BoundedToken,
};
use d2b_provider_device_usbip::{
    AdmittedDeviceClaim, AttachProcessIdentity, AttachmentObservation, BindingIdentity,
    BindingLifecycle, BindingLifecycleError, BindingPort, BindingProxyLease, BindingSlotLease,
    ClaimProjectionFence, FirewallConfirmation, FirewallDigest, FirewallObservation,
    FirewallProjectionAction, FirewallToken, RelayAuthorityLease, ServiceLifecycle,
    ServiceLifecycleError, ServicePhase, ServicePort, SupervisorFinalizeError,
    USBIP_RELAY_OPERATIONS, UsbipBindingAdmission, UsbipBindingController, UsbipBindingPhase,
    UsbipClaimPort, UsbipEffectError, UsbipSupervisor, binding_child_resources,
    usbip_guest_endpoint_request, usbip_relay_endpoint_request, usbip_relay_network_request,
    usbip_service_device_request,
};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[test]
fn explicit_binding_children_are_resource_backed_and_ordered_for_teardown() {
    let children = binding_child_resources(
        &ResourceRef::parse("usb.d2bus.org.UsbBinding/keyboard").unwrap(),
        &ResourceRef::parse("usb.d2bus.org.UsbService/usb-bus").unwrap(),
        &ResourceRef::parse("Guest/guest-a").unwrap(),
    )
    .unwrap();

    assert_eq!(children.iter().count(), 2);
    assert_eq!(children.at(d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildPlacement::Host).count(), 0);
    assert_eq!(children.at(d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildPlacement::Guest).count(), 2);
    assert_eq!(
        children
            .teardown_order()
            .iter()
            .map(|child| child.role())
            .collect::<Vec<_>>(),
        vec!["guest-endpoint", "guest-proxy"]
    );
    assert_eq!(
        children.child("guest-endpoint").unwrap().producer_ref(),
        Some(children.child("guest-proxy").unwrap().resource_ref())
    );
}

#[test]
fn binding_controller_only_observes_core_managed_children() {
    let binding = ResourceRef::parse("usb.d2bus.org.UsbBinding/keyboard").unwrap();
    let service = ResourceRef::parse("usb.d2bus.org.UsbService/usb-bus").unwrap();
    let target = ResourceRef::parse("Guest/guest-a").unwrap();
    let mut controller = UsbipBindingController::new(&binding, &service, &target).unwrap();

    assert_eq!(controller.phase(), UsbipBindingPhase::Pending);
    assert_eq!(
        controller.observe_children(true).unwrap().phase,
        UsbipBindingPhase::Ready
    );
    controller.finalize();
    assert_eq!(controller.phase(), UsbipBindingPhase::Deleted);
    assert!(controller.observe_children(true).is_err());
}

struct FakePort {
    calls: Vec<&'static str>,
    fail_physical: bool,
    fail_relay: bool,
    fail_endpoint_delete: bool,
    fail_claim_release: bool,
    observation: AttachmentObservation,
}

impl Default for FakePort {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            fail_physical: false,
            fail_relay: false,
            fail_endpoint_delete: false,
            fail_claim_release: false,
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

    fn ensure_attach_process(
        &mut self,
        _: &BindingIdentity,
        _: &BindingProxyLease,
    ) -> Result<AttachProcessIdentity, BindingLifecycleError> {
        self.calls.push("ensure-attach-process");
        Ok(AttachProcessIdentity::from_adapter(7, 11))
    }

    fn observe_attach_process(
        &mut self,
        _: &BindingIdentity,
        _: &AttachProcessIdentity,
    ) -> Result<AttachmentObservation, BindingLifecycleError> {
        self.calls.push("observe-attach-process");
        Ok(self.observation.clone())
    }

    fn delete_guest_endpoint(
        &mut self,
        _: &BindingIdentity,
        _: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError> {
        self.calls.push("delete-guest-endpoint");
        if self.fail_endpoint_delete {
            return Err(BindingLifecycleError::ForeignIdentity);
        }
        Ok(())
    }

    fn delete_attach_process(
        &mut self,
        _: &BindingIdentity,
        _: &AttachProcessIdentity,
    ) -> Result<(), BindingLifecycleError> {
        self.calls.push("delete-attach-process");
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

impl UsbipClaimPort for FakePort {
    fn start_relay_leg(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceRef,
        _: &ResourceUid,
        _: &ClaimProjectionFence,
    ) -> Result<RelayAuthorityLease, UsbipEffectError> {
        self.calls.push("start-relay-leg");
        Ok(RelayAuthorityLease::from_adapter([9; 16]))
    }

    fn mutate_claim_firewall(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceUid,
        action: FirewallProjectionAction,
        _: &ClaimProjectionFence,
        _: Option<&FirewallToken>,
    ) -> Result<FirewallConfirmation, UsbipEffectError> {
        match action {
            FirewallProjectionAction::Apply => {
                self.calls.push("apply-firewall");
                Ok(FirewallConfirmation::applied(
                    FirewallToken::from_adapter([7; 16]),
                    FirewallDigest::from_adapter([8; 32]),
                ))
            }
            FirewallProjectionAction::Remove => {
                self.calls.push("remove-firewall");
                Ok(FirewallConfirmation::removed())
            }
        }
    }

    fn observe_claim_firewall(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceUid,
        _: &ClaimProjectionFence,
        _: &FirewallToken,
    ) -> Result<FirewallObservation, UsbipEffectError> {
        self.calls.push("observe-firewall");
        Ok(FirewallObservation::new(
            true,
            FirewallDigest::from_adapter([8; 32]),
        ))
    }

    fn stop_relay_leg(
        &mut self,
        _: &AdmittedDeviceClaim,
        _: &ResourceRef,
    ) -> Result<(), UsbipEffectError> {
        self.calls.push("stop-relay-leg");
        Ok(())
    }

    fn release_claim(&mut self, _: &AdmittedDeviceClaim) -> Result<(), UsbipEffectError> {
        self.calls.push("release-claim");
        if self.fail_claim_release {
            return Err(UsbipEffectError::Transient);
        }
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
    assert_eq!(port.calls, ["observe-attach-process"]);
    supervisor.finalize(&mut port).unwrap();
    assert_eq!(
        port.calls,
        [
            "observe-attach-process",
            "delete-guest-endpoint",
            "delete-attach-process",
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
    assert_eq!(port.calls, ["observe-attach-process"]);
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
    assert_eq!(port.calls, ["observe-attach-process"]);
}

#[test]
fn binding_is_not_attached_until_the_guest_process_is_ready() {
    let zone = uid("123e4567-e89b-42d3-a456-426614174000");
    let mut port = FakePort {
        observation: AttachmentObservation::Missing,
        ..Default::default()
    };
    let mut service =
        ServiceLifecycle::new(zone.clone(), uid("223e4567-e89b-42d3-a456-426614174001"));
    service.activate(true, zone.clone(), &mut port).unwrap();
    port.calls.clear();
    let mut supervisor = UsbipSupervisor::new(service);
    supervisor
        .add_binding(BindingLifecycle::new(
            zone.clone(),
            zone,
            BindingIdentity::from_controller(uid("323e4567-e89b-42d3-a456-426614174002")),
        ))
        .unwrap();

    assert_eq!(
        supervisor.activate_binding(0, &mut port),
        Err(BindingLifecycleError::Transient)
    );
    assert_eq!(
        port.calls,
        [
            "slot",
            "proxy",
            "ensure-attach-process",
            "observe-attach-process",
        ]
    );
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
    port.observation = AttachmentObservation::Matching {
        slot: BindingSlotLease::from_adapter([4; 16]),
        proxy: BindingProxyLease::from_adapter([5; 16]),
    };
    supervisor.activate_binding(0, &mut port).unwrap();
    assert_eq!(
        port.calls,
        [
            "observe-attach-process",
            "slot",
            "proxy",
            "ensure-attach-process",
            "observe-attach-process",
        ]
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
            "ensure-attach-process",
            "observe-attach-process",
            "delete-guest-endpoint",
            "delete-attach-process",
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

#[test]
fn binding_admission_fences_stale_assignment_and_rejects_volume_ownership() {
    let binding = ResourceRef::parse("usb.d2bus.org.UsbBinding/keyboard").unwrap();
    let service = ResourceRef::parse("usb.d2bus.org.UsbService/usb-bus").unwrap();
    let target = ResourceRef::parse("Guest/guest-a").unwrap();
    let admission = UsbipBindingAdmission::new(
        uid("123e4567-e89b-42d3-a456-426614174000"),
        uid("223e4567-e89b-42d3-a456-426614174001"),
        uid("323e4567-e89b-42d3-a456-426614174002"),
        uid("423e4567-e89b-42d3-a456-426614174003"),
        d2b_contracts_resource::v3::ResourceGeneration::new(2).unwrap(),
        7,
    )
    .unwrap();
    let mut controller =
        UsbipBindingController::new_admitted(&binding, &service, &target, admission.clone())
            .unwrap();

    assert!(!controller.owns_child(&ResourceRef::parse("Volume/foreign").unwrap()));
    controller.observe_children_with_admission(admission.clone(), true).unwrap();

    let stale = UsbipBindingAdmission::new(
        uid("123e4567-e89b-42d3-a456-426614174000"),
        uid("223e4567-e89b-42d3-a456-426614174001"),
        uid("323e4567-e89b-42d3-a456-426614174002"),
        uid("423e4567-e89b-42d3-a456-426614174003"),
        d2b_contracts_resource::v3::ResourceGeneration::new(2).unwrap(),
        8,
    )
    .unwrap();
    assert_eq!(
        controller.observe_children_with_admission(stale, true),
        Err(d2b_provider_device_usbip::UsbipBindingControllerError::StaleAssignment)
    );
}

#[test]
fn usbip_runner_contract_keeps_service_and_binding_on_one_runner() {
    let contract = d2b_provider_device_usbip::usbip_runner_contract();
    assert_eq!(
        contract.service_resource_type(),
        d2b_provider_device_usbip::USB_SERVICE_RESOURCE_TYPE
    );
    assert_eq!(
        contract.binding_resource_type(),
        d2b_provider_device_usbip::USB_BINDING_RESOURCE_TYPE
    );
    assert!(contract.watched_configuration_is_dependency());
    assert!((30..=60).contains(&contract.repair_interval_secs()));
}

// ---------------------------------------------------------------------------
// The converted drain: an admitted `Device` claim and bounded helper legs
// ---------------------------------------------------------------------------

const BUS_KEY: [u8; 32] = [7; 32];
const DEVICE: &str = "Device/usb-bus";
const CONTROLLER: &str = "Process/device-usbip-service-controller";
const RELAY: &str = "Process/usbip-relay";

fn zone() -> ZoneId {
    ZoneId::parse("dev").expect("bounded zone")
}

fn store(value: &str) -> StoreIncarnation {
    StoreIncarnation::parse(value).expect("bounded store incarnation")
}

/// Admit the family's own device request through the shared binding contract.
fn admitted_claim(
    store: &StoreIncarnation,
    state: BindingLifecycleState,
) -> AdmittedDeviceClaim {
    let device_ref = ResourceRef::parse(DEVICE).expect("typed reference");
    let request = usbip_service_device_request(&device_ref, DeviceClaimRequest::Exclusive)
        .expect("a USB Service's device request is constructible");
    let key = request
        .key(
            zone(),
            uid("123e4567-e89b-42d3-a456-426614174000"),
            uid("223e4567-e89b-42d3-a456-426614174001"),
        )
        .expect("the request keys a relationship");
    let source = SourceAdmission::new(
        key.clone(),
        vec![RequestedRights::Exclusive],
        BindingArbitration::Exclusive,
    )
    .expect("the source decision is constructible");
    let admission = admit_binding_request(
        &key,
        request.requested_rights(),
        request.required_facets(),
        &BindingAuthorization::granted(),
        &source,
        &AdmittedDeviceClaim::support().expect("the attachment facet is supported"),
        &[FreshnessTuple::new(
            zone(),
            store.clone(),
            device_ref,
            uid("123e4567-e89b-42d3-a456-426614174000"),
            DesiredRevision::INITIAL,
            DesiredDigest::of(b"device"),
        )],
    )
    .expect("an exclusive claim on a free authority is admitted");
    let reservation = SourceReservation::new(
        zone(),
        uid("123e4567-e89b-42d3-a456-426614174000"),
        BoundedToken::parse("usb-res").expect("bounded reservation id"),
    );
    AdmittedDeviceClaim::new(
        BindingEvidence::admitted(admission, reservation).observed(BindingObservation::new(
            state,
            CompletionCondition::Complete,
            CompletionCondition::Pending,
            ReleaseOutcome::Outstanding,
        )),
        DeviceFunction::parse("usb-bus").expect("bounded function token"),
        DeviceAuthorityKey::from_core(BUS_KEY),
        USBIP_RELAY_OPERATIONS.to_vec(),
    )
    .expect("the admitted relationship carries the relay's operation classes")
}

/// Relay shutdown and Guest Endpoint closure both precede the source release.
///
/// The supervisor drains each Binding's own effects first - the Guest Endpoint,
/// then the attach Process, then the private proxy, then the Service slot -
/// stops the relay leg while the `Device` reservation is still held, and only
/// then hands the relationship back. Nothing reaches `release-claim` before the
/// relay is down and every Guest attachment is closed.
#[test]
fn relay_shutdown_and_endpoint_closure_precede_source_release() {
    let admitted_store = store("store-one");
    let claim = admitted_claim(&admitted_store, BindingLifecycleState::Active);
    let mut port = FakePort::default();
    let mut service =
        ServiceLifecycle::new(uid("a23e4567-e89b-42d3-a456-426614174000"), uid("b23e4567-e89b-42d3-a456-426614174001"));
    service
        .admit_claim(
            &zone(),
            &ResourceRef::parse(DEVICE).unwrap(),
            &admitted_store,
            &ResourceRef::parse(RELAY).unwrap(),
            &claim,
        )
        .expect("the Service realizes the claim the source admitted");
    assert_eq!(service.claim().map(AdmittedDeviceClaim::key), Some(claim.key()));
    assert_eq!(service.relay_helper(), Some(&ResourceRef::parse(RELAY).unwrap()));
    assert_eq!(service.claim_store(), Some(&admitted_store));

    let mut supervisor = UsbipSupervisor::new(service);
    supervisor
        .add_binding(BindingLifecycle::new(
            uid("a23e4567-e89b-42d3-a456-426614174000"),
            uid("a23e4567-e89b-42d3-a456-426614174000"),
            BindingIdentity::from_controller(uid("c23e4567-e89b-42d3-a456-426614174002")),
        ))
        .unwrap();
    supervisor
        .activate_binding(0, &mut port)
        .expect("the Binding attaches through its declared children");
    port.calls.clear();

    supervisor
        .finalize_claim(&mut port)
        .expect("the converted drain completes");
    assert_eq!(
        port.calls,
        [
            "delete-guest-endpoint",
            "delete-attach-process",
            "close-proxy",
            "release-slot",
            "stop-relay-leg",
            "release-claim",
        ]
    );
    assert_eq!(supervisor.service().phase(), ServicePhase::Closed);
    assert!(supervisor.service().claim().is_none());
    let stopped = port
        .calls
        .iter()
        .position(|call| *call == "stop-relay-leg")
        .expect("the relay leg stopped");
    let released = port
        .calls
        .iter()
        .position(|call| *call == "release-claim")
        .expect("the claim was handed back");
    assert!(
        stopped < released,
        "the relay must be down before the reservation moves: {stopped} then {released}"
    );
}

/// A Binding that has not closed stops the drain before the reservation moves.
///
/// The order is enforced, not merely observed: a Guest Endpoint that could not
/// be deleted leaves the relay running and the source's claim retained, because
/// releasing it while a proxy can still reach the backing is the same
/// use-after-release as a mount that outlives its volume.
#[test]
fn an_unclosed_binding_stops_the_drain_before_the_source_release() {
    let admitted_store = store("store-one");
    let claim = admitted_claim(&admitted_store, BindingLifecycleState::Active);
    let mut port = FakePort {
        fail_endpoint_delete: true,
        ..Default::default()
    };
    let mut service = ServiceLifecycle::new(
        uid("a23e4567-e89b-42d3-a456-426614174000"),
        uid("b23e4567-e89b-42d3-a456-426614174001"),
    );
    service
        .admit_claim(
            &zone(),
            &ResourceRef::parse(DEVICE).unwrap(),
            &admitted_store,
            &ResourceRef::parse(RELAY).unwrap(),
            &claim,
        )
        .expect("the Service realizes the admitted claim");
    let mut supervisor = UsbipSupervisor::new(service);
    supervisor
        .add_binding(BindingLifecycle::new(
            uid("a23e4567-e89b-42d3-a456-426614174000"),
            uid("a23e4567-e89b-42d3-a456-426614174000"),
            BindingIdentity::from_controller(uid("c23e4567-e89b-42d3-a456-426614174002")),
        ))
        .unwrap();
    supervisor.activate_binding(0, &mut port).unwrap();
    port.calls.clear();

    assert_eq!(
        supervisor.finalize_claim(&mut port),
        Err(SupervisorFinalizeError::Binding(
            BindingLifecycleError::ForeignIdentity
        ))
    );
    assert_eq!(port.calls, ["delete-guest-endpoint"]);
    assert!(
        !port.calls.contains(&"stop-relay-leg"),
        "the relay must keep running while a Guest attachment is still open"
    );
    assert!(
        !port.calls.contains(&"release-claim"),
        "the source reservation must not be released before the drain finishes"
    );
    assert!(
        supervisor.service().claim().is_some(),
        "the retained claim stays held for a retry"
    );
    assert_eq!(supervisor.service().phase(), ServicePhase::DrainingBindings);
}

/// The Service accepts exactly one claim, in its own Zone, under its own store.
///
/// A second claim while one is retained, a claim from another Zone, and a claim
/// admitted under a previous store incarnation are all refused, so a Service
/// cannot quietly swap the physical device under a live relay.
#[test]
fn the_service_refuses_a_second_claim_and_a_stale_store() {
    let admitted_store = store("store-one");
    let claim = admitted_claim(&admitted_store, BindingLifecycleState::Active);
    let mut service = ServiceLifecycle::new(
        uid("a23e4567-e89b-42d3-a456-426614174000"),
        uid("b23e4567-e89b-42d3-a456-426614174001"),
    );
    let device_ref = ResourceRef::parse(DEVICE).unwrap();
    let relay_ref = ResourceRef::parse(RELAY).unwrap();

    assert_eq!(
        service.admit_claim(&zone(), &device_ref, &store("store-two"), &relay_ref, &claim),
        Err(ServiceLifecycleError::PhysicalAuthorityConflict),
        "a claim admitted under another store is not this Service's authority"
    );
    assert_eq!(
        service.admit_claim(
            &ZoneId::parse("other").unwrap(),
            &device_ref,
            &admitted_store,
            &relay_ref,
            &claim,
        ),
        Err(ServiceLifecycleError::PhysicalAuthorityConflict)
    );
    assert!(service.claim().is_none());
    assert_eq!(service.phase(), ServicePhase::WaitingForOptIn);

    service
        .admit_claim(&zone(), &device_ref, &admitted_store, &relay_ref, &claim)
        .expect("the Service's own claim is accepted");
    assert_eq!(
        service.phase(),
        ServicePhase::Bound,
        "an admitted claim is the bound device, so Bindings may attach"
    );

    let replacement = admitted_claim(&store("store-two"), BindingLifecycleState::Active);
    assert_eq!(
        service.admit_claim(&zone(), &device_ref, &store("store-two"), &relay_ref, &replacement),
        Err(ServiceLifecycleError::PhysicalAuthorityConflict),
        "a reappearing device is not adopted while the retained claim stands"
    );
    assert_eq!(
        service.claim().map(AdmittedDeviceClaim::key),
        Some(claim.key())
    );
}

/// A semantic Service and Binding request Device, Endpoint, and Network
/// relationships rather than taking a host grant.
///
/// The Service requests its exclusive `Device` claim, its relay's `Network`
/// membership, and its relay `Endpoint`; the Binding requests the `Endpoint`
/// its own declared guest-proxy child consumes. Every request is canonical and
/// keyed by a stable slot, and none of them is a path, a bus id, or a host
/// permission.
#[test]
fn semantic_bindings_request_device_endpoint_and_network_relationships() {
    let device_request = usbip_service_device_request(
        &ResourceRef::parse(DEVICE).unwrap(),
        DeviceClaimRequest::Exclusive,
    )
    .expect("the Service requests its device claim");
    assert_eq!(device_request.source_ref(), &ResourceRef::parse(DEVICE).unwrap());
    assert_eq!(
        device_request.consumer_ref(),
        &ResourceRef::parse(CONTROLLER).unwrap()
    );
    assert_eq!(device_request.slot().as_str(), "usb-backing");

    let network_request = usbip_relay_network_request(&ResourceRef::parse("Network/work").unwrap())
        .expect("the relay requests its network membership");
    assert_eq!(network_request.source_ref(), &ResourceRef::parse("Network/work").unwrap());
    assert_eq!(network_request.slot().as_str(), "relay-network");
    assert!(
        !network_request.membership().allow_egress(),
        "the relay receives device traffic; it does not declare egress"
    );
    assert!(network_request.membership().ports().is_empty());

    let relay_endpoint =
        usbip_relay_endpoint_request(&ResourceRef::parse("Endpoint/usbip-relay").unwrap())
            .expect("the Service requests its relay endpoint");
    assert_eq!(relay_endpoint.attachment(), EndpointAttachmentKind::Listen);
    assert_eq!(
        relay_endpoint.consumer_ref(),
        &ResourceRef::parse(CONTROLLER).unwrap()
    );

    let binding = UsbipBindingController::new(
        &ResourceRef::parse("usb.d2bus.org.UsbBinding/keyboard").unwrap(),
        &ResourceRef::parse("usb.d2bus.org.UsbService/usb-bus").unwrap(),
        &ResourceRef::parse("Guest/guest-a").unwrap(),
    )
    .unwrap();
    let guest_endpoint = binding
        .endpoint_request(&ResourceRef::parse("Endpoint/keyboard-in-guest").unwrap())
        .expect("the Binding requests its guest endpoint");
    assert_eq!(guest_endpoint.attachment(), EndpointAttachmentKind::Connect);
    assert_eq!(guest_endpoint.slot().as_str(), "guest-endpoint");
    assert_eq!(
        guest_endpoint.consumer_ref(),
        binding
            .children()
            .child("guest-proxy")
            .expect("the Binding declares its guest proxy")
            .resource_ref(),
        "the attachment is delivered to the Binding's own declared helper"
    );

    // The same builder refuses a source of the wrong kind rather than widening
    // the relationship to whatever was named.
    assert_eq!(
        usbip_guest_endpoint_request(
            &ResourceRef::parse("Network/work").unwrap(),
            &ResourceRef::parse(CONTROLLER).unwrap(),
        )
        .expect_err("a Network is not an Endpoint source"),
        BindingContractError::WrongResourceType
    );
}
