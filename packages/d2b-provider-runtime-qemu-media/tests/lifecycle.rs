use d2b_contracts::v3::ResourceRef;
use d2b_provider_runtime_qemu_media::{
    DeviceObservation, DevicePhase, GuestProviderSpecSettings, LaunchTicket, PlatformClass,
    ProcessIdentity, ProviderConfig, QemuMediaController, QemuMediaEffectPort, QemuMediaError,
    QemuMediaPhase, QemuMediaReconcileOutcome, QemuMediaRecoveryState,
};

#[derive(Default)]
struct FakeEffect {
    observed: Option<ProcessIdentity>,
    launched: usize,
    pidfd_opens: usize,
    events: Vec<&'static str>,
    launch_slots: Vec<String>,
    stop_clears_observation: bool,
}

impl QemuMediaEffectPort for FakeEffect {
    fn launch(&mut self, ticket: &LaunchTicket) -> Result<ProcessIdentity, QemuMediaError> {
        self.launched += 1;
        self.events.push("launch");
        self.launch_slots = ticket
            .attachments
            .iter()
            .map(|attachment| attachment.slot.clone())
            .collect();
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
    let process = d2b_provider_runtime_qemu_media::ProcessSpec::new(
        ResourceRef::parse("Guest/media-vm").unwrap(),
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

#[test]
fn ready_requires_process_device_and_qmp_health() {
    let mut controller = controller();
    let mut effect = FakeEffect::default();
    let pending = controller
        .reconcile(&Default::default(), &mut effect)
        .unwrap();
    assert!(matches!(pending, QemuMediaReconcileOutcome::Retry { .. }));
    assert_eq!(controller.phase(), QemuMediaPhase::Pending);

    let device = DeviceObservation {
        device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
        phase: DevicePhase::Ready,
        owner_ref: Some(ResourceRef::parse("Guest/media-vm").unwrap()),
        platform: PlatformClass::X86_64Linux,
        authority_key: [4; 32],
        process_identity: Some("qemu-media-runner".to_owned()),
        media_contract: "qemu-media/v1".to_owned(),
    };
    let mut deps = d2b_provider_runtime_qemu_media::QemuMediaDependencies::ready(device);
    deps.media_refs = vec![ResourceRef::parse("Volume/boot-media").unwrap()];
    deps.display_ref = Some(ResourceRef::parse("Endpoint/display").unwrap());
    let ready = controller.reconcile(&deps, &mut effect).unwrap();
    assert_eq!(ready, QemuMediaReconcileOutcome::Ready);
    assert_eq!(controller.phase(), QemuMediaPhase::PausedAtBoot);
    assert_eq!(effect.launch_slots, vec!["kvm", "media-0", "display"]);
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
    let device = DeviceObservation {
        device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
        phase: DevicePhase::Ready,
        owner_ref: Some(ResourceRef::parse("Guest/media-vm").unwrap()),
        platform: PlatformClass::X86_64Linux,
        authority_key: [4; 32],
        process_identity: Some("qemu-media-runner".to_owned()),
        media_contract: "qemu-media/v1".to_owned(),
    };
    let deps = d2b_provider_runtime_qemu_media::QemuMediaDependencies::ready(device);
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
    let identity = ProcessIdentity::for_test("qemu-media-runner");
    let mut effect = FakeEffect {
        observed: Some(identity.clone()),
        ..FakeEffect::default()
    };
    controller.set_expected_identity(identity);
    let device = DeviceObservation {
        device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
        phase: DevicePhase::Ready,
        owner_ref: Some(ResourceRef::parse("Guest/media-vm").unwrap()),
        platform: PlatformClass::X86_64Linux,
        authority_key: [9; 32],
        process_identity: Some("qemu-media-runner".to_owned()),
        media_contract: "qemu-media/v1".to_owned(),
    };
    let mut dependencies = d2b_provider_runtime_qemu_media::QemuMediaDependencies::ready(device);
    dependencies.qmp_ready = false;
    dependencies.qmp_status = None;
    dependencies.qmp_elapsed_seconds = 30;

    assert_eq!(
        controller
            .reconcile(&dependencies, &mut effect)
            .unwrap_err(),
        QemuMediaError::QmpNotReady
    );
    assert_eq!(effect.events, vec!["reserve-device", "open-pidfd", "stop"]);
    assert!(controller.recovery_state().authority_reserved);

    effect.observed = None;
    controller.finalize(&mut effect).unwrap();
    assert_eq!(
        effect.events,
        vec![
            "reserve-device",
            "open-pidfd",
            "stop",
            "close-media",
            "release-device",
            "delete-volume",
        ]
    );
}

#[test]
fn finalized_recovery_state_cannot_retain_device_authority() {
    let recovery = QemuMediaRecoveryState {
        phase: QemuMediaPhase::Finalized,
        finalizer_installed: false,
        expected_identity: None,
        authority_reserved: true,
    };
    let restored = controller()
        .restore_recovery_state(recovery)
        .unwrap()
        .recovery_state();
    assert!(!restored.authority_reserved);
}
