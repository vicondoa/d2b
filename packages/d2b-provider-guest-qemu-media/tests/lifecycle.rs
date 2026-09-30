use d2b_contracts_resource::v3::{BindingLifecycleState, ResourceRef};
use d2b_provider_guest_qemu_media::{
    DeviceObservation, DevicePhase, GuestMediaBindings, GuestProviderSpecSettings,
    ImplementationLeg, LaunchTicket, MediaAdmissionError, PlatformClass, ProcessIdentity,
    ProviderConfig,
    QemuMediaController, QemuMediaEffectPort, QemuMediaError, QemuMediaPhase,
    QemuMediaReconcileOutcome, QemuMediaRecoveryState, test_fixtures,
};

#[derive(Default)]
struct FakeEffect {
    observed: Option<ProcessIdentity>,
    launched: usize,
    pidfd_opens: usize,
    events: Vec<&'static str>,
    launch_slots: Vec<String>,
    stop_clears_observation: bool,
    legs: Vec<ImplementationLeg>,
    detached_legs: usize,
}

impl QemuMediaEffectPort for FakeEffect {
    fn launch(&mut self, ticket: &LaunchTicket) -> Result<ProcessIdentity, QemuMediaError> {
        self.launched += 1;
        self.events.push("launch");
        self.launch_slots = ticket.attachments.labels().iter().map(|label| (*label).to_owned()).collect();
        let identity = ProcessIdentity::for_test("qemu-media-runner");
        self.observed = Some(identity.clone());
        Ok(identity)
    }

    fn observe(&mut self) -> Result<Option<ProcessIdentity>, QemuMediaError> {
        Ok(self.observed.clone())
    }

    fn open_pidfd(&mut self, _identity: &ProcessIdentity) -> Result<(), QemuMediaError> {
        self.pidfd_opens += 1;
        self.events.push("open-pidfd");
        Ok(())
    }

    fn reserve_device_authority(
        &mut self,
        _authority_key: [u8; 32],
        _owner_ref: &ResourceRef,
    ) -> Result<(), QemuMediaError> {
        self.events.push("reserve-device");
        Ok(())
    }

    fn attach_implementation_leg(&mut self, leg: &ImplementationLeg) -> Result<(), QemuMediaError> {
        self.legs.push(leg.clone());
        self.events.push("attach-leg");
        Ok(())
    }

    fn detach_implementation_legs(&mut self) -> Result<(), QemuMediaError> {
        self.detached_legs = self.legs.len();
        self.legs.clear();
        self.events.push("detach-legs");
        Ok(())
    }

    fn close_media_effects(&mut self) -> Result<(), QemuMediaError> {
        self.events.push("close-media");
        Ok(())
    }

    fn continue_guest(&mut self) -> Result<(), QemuMediaError> {
        self.events.push("continue");
        Ok(())
    }

    fn stop(&mut self, _identity: &ProcessIdentity) -> Result<(), QemuMediaError> {
        self.events.push("stop");
        if self.stop_clears_observation {
            self.observed = None;
        }
        Ok(())
    }

    fn release_device_authority(&mut self) -> Result<(), QemuMediaError> {
        self.events.push("release-device");
        Ok(())
    }

    fn delete_runtime_volume(&mut self) -> Result<(), QemuMediaError> {
        self.events.push("delete-volume");
        Ok(())
    }
}

fn config() -> ProviderConfig {
    ProviderConfig::new(
        "Host/host-system",
        "qemu-system-x86-64",
        "Provider/network-local",
        "Provider/volume-local",
        None,
    )
    .unwrap()
}

fn controller() -> QemuMediaController<FakeEffect> {
    let settings = GuestProviderSpecSettings::default();
    let process = d2b_provider_guest_qemu_media::build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        Vec::<ResourceRef>::new(),
    )
    .unwrap();
    QemuMediaController::new(
        config(),
        settings,
        process,
        ResourceRef::parse("Guest/media-vm").unwrap(),
    )
    .unwrap()
}

/// The Guest's admitted relationship set: the KVM acceleration Device and one
/// boot media Volume, which is exactly what this Guest's own spec requires.
fn bindings() -> GuestMediaBindings {
    let guest = ResourceRef::parse("Guest/media-vm").unwrap();
    let consumer = test_fixtures::guest_uid();
    GuestMediaBindings::new(
        test_fixtures::zone(),
        consumer.clone(),
        [
            test_fixtures::admitted(
                test_fixtures::kvm_request(&guest),
                &test_fixtures::uid(2),
                &consumer,
            ),
            test_fixtures::admitted(
                test_fixtures::media_request(&guest, "boot", 0),
                &test_fixtures::uid(3),
                &consumer,
            ),
        ],
    )
}

/// A controller whose Guest declared a boot media Volume, so its spec
/// requires the media relationship alongside the KVM Device.
fn media_controller() -> QemuMediaController<FakeEffect> {
    let settings = GuestProviderSpecSettings {
        boot_media_ref: Some(ResourceRef::parse("Volume/boot").unwrap()),
        ..GuestProviderSpecSettings::default()
    };
    let process = d2b_provider_guest_qemu_media::build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        Vec::<ResourceRef>::new(),
    )
    .unwrap();
    QemuMediaController::new(
        config(),
        settings,
        process,
        ResourceRef::parse("Guest/media-vm").unwrap(),
    )
    .unwrap()
}

/// A controller whose Guest declared a host display window, so its spec
/// requires the display Endpoint alongside the KVM and media relationships.
fn display_controller() -> QemuMediaController<FakeEffect> {
    let settings = GuestProviderSpecSettings {
        display_window: true,
        ..GuestProviderSpecSettings::default()
    };
    let process = d2b_provider_guest_qemu_media::build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        Vec::<ResourceRef>::new(),
    )
    .unwrap();
    QemuMediaController::new(
        config(),
        settings,
        process,
        ResourceRef::parse("Guest/media-vm").unwrap(),
    )
    .unwrap()
}

fn device() -> DeviceObservation {
    DeviceObservation {
        device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
        phase: DevicePhase::Ready,
        owner_ref: Some(ResourceRef::parse("Guest/media-vm").unwrap()),
        platform: PlatformClass::X86_64Linux,
        authority_key: [4; 32],
        process_identity: Some("qemu-media-runner".to_owned()),
        media_contract: "qemu-media/v1".to_owned(),
    }
}

#[test]
fn ready_requires_process_device_and_qmp_health() {
    let mut controller = controller();
    let mut effect = FakeEffect::default();
    let pending = controller
        .reconcile(&Default::default(), &mut effect)
        .unwrap();
    assert!(matches!(pending, QemuMediaReconcileOutcome::Retry { .. }));
    assert_eq!(controller.phase(), QemuMediaPhase::Pending);

    let device = device();
    let deps = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device, bindings());
    let ready = controller.reconcile(&deps, &mut effect).unwrap();
    assert_eq!(ready, QemuMediaReconcileOutcome::Ready);
    assert_eq!(controller.phase(), QemuMediaPhase::PausedAtBoot);
    assert_eq!(effect.launch_slots, vec!["kvm", "media-0"]);
}

#[test]
fn pause_at_boot_is_initial_proof_then_running_is_ready() {
    let mut controller = controller();
    let mut effect = FakeEffect::default();
    let device = device();
    let mut dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device, bindings());

    assert_eq!(
        controller.reconcile(&dependencies, &mut effect).unwrap(),
        QemuMediaReconcileOutcome::Ready
    );
    assert_eq!(controller.phase(), QemuMediaPhase::PausedAtBoot);

    dependencies.qmp_status = Some(d2b_provider_guest_qemu_media::QmpVmStatus::Running);
    assert_eq!(
        controller.reconcile(&dependencies, &mut effect).unwrap(),
        QemuMediaReconcileOutcome::Ready
    );
    assert_eq!(controller.phase(), QemuMediaPhase::Ready);
    assert_eq!(
        effect.events,
        vec![
            "reserve-device",
            "attach-leg",
            "attach-leg",
            "launch",
            "open-pidfd"
        ]
    );
}

#[test]
fn pause_at_boot_rejects_running_before_pause_proof() {
    let mut controller = controller();
    let mut effect = FakeEffect::default();
    let device = device();
    let mut dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device, bindings());
    dependencies.qmp_status = Some(d2b_provider_guest_qemu_media::QmpVmStatus::Running);

    assert_eq!(
        controller
            .reconcile(&dependencies, &mut effect)
            .unwrap_err(),
        QemuMediaError::QmpNotReady
    );
    assert_eq!(controller.phase(), QemuMediaPhase::Degraded);
    assert_eq!(
        effect.events,
        vec![
            "reserve-device",
            "attach-leg",
            "attach-leg",
            "launch",
            "open-pidfd"
        ]
    );
}

#[test]
fn matching_restart_process_is_adopted_without_launch() {
    let mut controller = controller();
    let identity = ProcessIdentity::for_test("qemu-media-runner");
    let mut effect = FakeEffect {
        observed: Some(identity.clone()),
        stop_clears_observation: true,
        ..FakeEffect::default()
    };
    let device = device();
    let deps = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device, bindings());
    controller.set_expected_identity(identity);
    assert_eq!(
        controller.reconcile(&deps, &mut effect).unwrap(),
        QemuMediaReconcileOutcome::Ready
    );
    assert_eq!(effect.launched, 0);
    assert_eq!(effect.pidfd_opens, 1);
}

#[test]
fn finalization_closes_media_before_releasing_authority() {
    let mut controller = controller();
    let identity = ProcessIdentity::for_test("media-process");
    let mut effect = FakeEffect {
        observed: Some(identity.clone()),
        stop_clears_observation: true,
        ..FakeEffect::default()
    };
    controller.set_expected_identity(identity);
    controller.mark_ready_for_test();
    controller.finalize(&mut effect).unwrap();
    assert_eq!(
        effect.events,
        vec![
            "close-media",
            "open-pidfd",
            "stop",
            "release-device",
            "delete-volume",
        ]
    );
}

#[test]
fn qmp_timeout_retains_authority_until_process_exit_is_proven() {
    let mut controller = controller();
    let mut effect = FakeEffect::default();
    let mut device = device();
    device.authority_key = [9; 32];
    let mut dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device, bindings());
    dependencies.qmp_ready = false;
    dependencies.qmp_status = None;
    dependencies.qmp_elapsed_seconds = 30;

    assert_eq!(
        controller
            .reconcile(&dependencies, &mut effect)
            .unwrap_err(),
        QemuMediaError::QmpNotReady
    );
    assert_eq!(
        effect.events,
        vec![
            "reserve-device",
            "attach-leg",
            "attach-leg",
            "launch",
            "open-pidfd",
            "stop"
        ]
    );
    assert!(controller.recovery_state().authority_reserved);

    effect.observed = None;
    controller.finalize(&mut effect).unwrap();
    assert_eq!(
        effect.events,
        vec![
            "reserve-device",
            "attach-leg",
            "attach-leg",
            "launch",
            "open-pidfd",
            "stop",
            "close-media",
            "detach-legs",
            "release-device",
            "delete-volume",
        ]
    );
}

#[test]
fn failed_qmp_timeout_does_not_adopt_a_stopping_runner() {
    let mut controller = controller();
    let mut effect = FakeEffect::default();
    let mut device = device();
    device.authority_key = [9; 32];
    let mut dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device, bindings());
    dependencies.qmp_ready = false;
    dependencies.qmp_status = None;
    dependencies.qmp_elapsed_seconds = 30;

    assert_eq!(
        controller
            .reconcile(&dependencies, &mut effect)
            .unwrap_err(),
        QemuMediaError::QmpNotReady
    );
    assert_eq!(controller.phase(), QemuMediaPhase::Failed);
    let events_before_reconcile = effect.events.clone();

    dependencies.qmp_ready = true;
    dependencies.qmp_status = Some(d2b_provider_guest_qemu_media::QmpVmStatus::Running);
    assert_eq!(
        controller
            .reconcile(&dependencies, &mut effect)
            .unwrap_err(),
        QemuMediaError::InvalidState
    );
    assert_eq!(controller.phase(), QemuMediaPhase::Failed);
    assert_eq!(effect.events, events_before_reconcile);
    assert!(controller.recovery_state().authority_reserved);
}

#[test]
fn failed_qmp_timeout_with_exit_proven_does_not_rereserve_on_reconcile() {
    let mut controller = controller();
    let mut effect = FakeEffect {
        stop_clears_observation: true,
        ..FakeEffect::default()
    };
    let mut device = device();
    device.authority_key = [9; 32];
    let mut dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device, bindings());
    dependencies.qmp_ready = false;
    dependencies.qmp_status = None;
    dependencies.qmp_elapsed_seconds = 30;

    assert_eq!(
        controller
            .reconcile(&dependencies, &mut effect)
            .unwrap_err(),
        QemuMediaError::QmpNotReady
    );
    assert_eq!(controller.phase(), QemuMediaPhase::Failed);
    assert!(!controller.recovery_state().authority_reserved);
    assert_eq!(
        effect.events,
        vec![
            "reserve-device",
            "attach-leg",
            "attach-leg",
            "launch",
            "open-pidfd",
            "stop",
            "release-device",
        ]
    );

    dependencies.qmp_ready = true;
    dependencies.qmp_status = Some(d2b_provider_guest_qemu_media::QmpVmStatus::Paused);
    assert_eq!(
        controller
            .reconcile(&dependencies, &mut effect)
            .unwrap_err(),
        QemuMediaError::InvalidState
    );
    assert_eq!(
        effect.events,
        vec![
            "reserve-device",
            "attach-leg",
            "attach-leg",
            "launch",
            "open-pidfd",
            "stop",
            "release-device",
        ]
    );

    controller.finalize(&mut effect).unwrap();
    assert_eq!(
        effect.events,
        vec![
            "reserve-device",
            "attach-leg",
            "attach-leg",
            "launch",
            "open-pidfd",
            "stop",
            "release-device",
            "close-media",
            "detach-legs",
            "delete-volume",
        ]
    );
    assert_eq!(
        effect
            .events
            .iter()
            .filter(|event| **event == "release-device")
            .count(),
        1
    );
    assert_eq!(controller.phase(), QemuMediaPhase::Finalized);
}

#[test]
fn adopted_runner_qmp_timeout_uses_health_retry_not_launch_age() {
    let mut controller = controller();
    let identity = ProcessIdentity::for_test("qemu-media-runner");
    let mut effect = FakeEffect {
        observed: Some(identity.clone()),
        ..FakeEffect::default()
    };
    controller.set_expected_identity(identity);
    let mut device = device();
    device.authority_key = [9; 32];
    let mut dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device, bindings());
    dependencies.qmp_ready = false;
    dependencies.qmp_status = None;
    dependencies.qmp_elapsed_seconds = 30;

    assert_eq!(
        controller.reconcile(&dependencies, &mut effect).unwrap(),
        QemuMediaReconcileOutcome::Retry { after_ms: 250 }
    );
    assert_eq!(controller.phase(), QemuMediaPhase::Degraded);
    assert_eq!(effect.events, vec!["reserve-device", "open-pidfd"]);
    assert!(controller.recovery_state().authority_reserved);
}

#[test]
fn finalized_recovery_state_cannot_retain_device_authority() {
    let recovery = QemuMediaRecoveryState {
        phase: QemuMediaPhase::Finalized,
        finalizer_installed: false,
        expected_identity: None,
        authority_reserved: true,
        initial_pause_observed: false,
    };
    let restored = controller()
        .restore_recovery_state(recovery)
        .unwrap()
        .recovery_state();
    assert!(!restored.authority_reserved);
}

/// Scenario 2 through the controller: a Guest whose own spec requires a KVM,
/// media, or display relationship the graph did not admit refuses preparation
/// and mutates nothing at all - no authority reserved, no leg attached, no
/// process launched.
#[test]
fn a_missing_admitted_binding_refuses_preparation_without_mutation() {
    let guest = ResourceRef::parse("Guest/media-vm").unwrap();
    let consumer = test_fixtures::guest_uid();

    for (label, relationships) in [
        (
            "kvm",
            vec![test_fixtures::admitted(
                test_fixtures::media_request(&guest, "boot", 0),
                &test_fixtures::uid(3),
                &consumer,
            )],
        ),
        (
            "media",
            vec![test_fixtures::admitted(
                test_fixtures::kvm_request(&guest),
                &test_fixtures::uid(2),
                &consumer,
            )],
        ),
    ] {
        let mut controller = media_controller();
        let mut effect = FakeEffect::default();
        let dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(
            device(),
            GuestMediaBindings::new(test_fixtures::zone(), consumer.clone(), relationships),
        );
        let error = controller.reconcile(&dependencies, &mut effect).unwrap_err();
        assert!(
            matches!(&error, QemuMediaError::Binding(MediaAdmissionError::MissingBinding)),
            "{label} binding absence reported {error:?}"
        );
        assert!(
            effect.events.is_empty(),
            "{label} binding absence mutated {events:?}",
            events = effect.events
        );
        assert_eq!(effect.launched, 0);
        assert_eq!(effect.legs.len(), 0);
        // A refused preparation does not advance the lifecycle either.
        assert_eq!(controller.phase(), QemuMediaPhase::Pending);
    }

    // A display window the Guest declared but whose endpoint was not admitted
    // is refused on the same terms.
    let mut controller = display_controller();
    let mut effect = FakeEffect::default();
    let dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(
        device(),
        GuestMediaBindings::new(test_fixtures::zone(), consumer, []),
    );
    assert!(matches!(
        controller.reconcile(&dependencies, &mut effect),
        Err(QemuMediaError::Binding(MediaAdmissionError::MissingBinding))
    ));
    assert!(effect.events.is_empty());
    assert_eq!(effect.launched, 0);
}

/// Scenario 2, second half: a relationship whose source decision no longer
/// matches the committed rows refuses preparation just as a missing one does.
#[test]
fn a_stale_admitted_binding_refuses_preparation_without_mutation() {
    let guest = ResourceRef::parse("Guest/media-vm").unwrap();
    let consumer = test_fixtures::guest_uid();
    let mut controller = controller();
    let mut effect = FakeEffect::default();
    let dependencies = d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(
        device(),
        GuestMediaBindings::new(
            test_fixtures::zone(),
            consumer.clone(),
            [
                test_fixtures::admitted_with(
                    test_fixtures::kvm_request(&guest),
                    &test_fixtures::uid(2),
                    &consumer,
                    None,
                    BindingLifecycleState::Released,
                ),
                test_fixtures::admitted(
                    test_fixtures::media_request(&guest, "boot", 0),
                    &test_fixtures::uid(3),
                    &consumer,
                ),
            ],
        ),
    );
    assert!(matches!(
        controller.reconcile(&dependencies, &mut effect),
        Err(QemuMediaError::Binding(MediaAdmissionError::StaleAuthority))
    ));
    assert!(effect.events.is_empty());
    assert_eq!(effect.launched, 0);
}

/// Scenario 3 through the controller: the runner is attached to the Guest's
/// own reservation as a leg, and the controller never asks for a second
/// reservation on the runner's behalf.
#[test]
fn the_runner_holds_a_leg_on_the_guests_own_reservation() {
    let mut controller = controller();
    let mut effect = FakeEffect::default();
    let dependencies =
        d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device(), bindings());
    assert_eq!(
        controller.reconcile(&dependencies, &mut effect).unwrap(),
        QemuMediaReconcileOutcome::Ready
    );

    // One reservation for the Guest, one leg per admitted descriptor, and the
    // only reservation call the controller made is the Host-global Device
    // authority it already made before this unit's work.
    assert_eq!(
        effect.events,
        vec!["reserve-device", "attach-leg", "attach-leg", "launch", "open-pidfd"]
    );
    assert_eq!(effect.legs.len(), effect.launch_slots.len());
    let kvm_leg = &effect.legs[0];
    assert_eq!(kvm_leg.parent.kind(), d2b_contracts_resource::v3::BindingKind::Device);
    assert_eq!(kvm_leg.parent.consumer_ref().resource_type().as_str(), "Guest");
    assert_eq!(
        kvm_leg.reservation.source_uid(),
        kvm_leg.parent.source_uid(),
        "the leg must hold the parent's source, never one of its own"
    );
    let media_leg = &effect.legs[1];
    assert_eq!(media_leg.parent.kind(), d2b_contracts_resource::v3::BindingKind::Volume);
    assert_ne!(media_leg.identity, kvm_leg.identity);

    // A second reconcile re-adopts the running process and attaches no new
    // reservation and no duplicate leg.
    assert_eq!(
        controller.reconcile(&dependencies, &mut effect).unwrap(),
        QemuMediaReconcileOutcome::Ready
    );
    assert_eq!(effect.legs.len(), 2);
    assert_eq!(effect.launched, 1);
}

/// Scenario 4: shutdown closes the consumer's descriptors, drops the runner's
/// legs, stops the process, and only then releases the source.
#[test]
fn shutdown_closes_consumer_descriptors_before_releasing_the_source() {
    let mut controller = controller();
    let mut effect = FakeEffect {
        stop_clears_observation: true,
        ..FakeEffect::default()
    };
    let dependencies =
        d2b_provider_guest_qemu_media::QemuMediaDependencies::ready(device(), bindings());
    assert_eq!(
        controller.reconcile(&dependencies, &mut effect).unwrap(),
        QemuMediaReconcileOutcome::Ready
    );
    let leg_count = effect.legs.len();
    assert_eq!(leg_count, 2);
    effect.events.clear();

    controller.finalize(&mut effect).unwrap();
    assert_eq!(
        effect.events,
        vec![
            "close-media",
            "detach-legs",
            "stop",
            "release-device",
            "delete-volume",
        ]
    );
    assert_eq!(effect.detached_legs, leg_count);
    assert!(effect.legs.is_empty());
    assert_eq!(controller.phase(), QemuMediaPhase::Finalized);
}
