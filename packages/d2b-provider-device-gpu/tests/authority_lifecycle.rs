use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, device::DeviceArbitration,
};
use d2b_provider_device_gpu::{
    GpuAuthorityAdmission, GpuAuthorityError, GpuAuthorityLease, GpuBackingToken, GpuClosureProof,
    GpuController, GpuEffectError, GpuEffectToken, GpuEffectTokenSet, GpuLifecycleEffectPort,
    GpuDeviceGrants, GpuOwnerProof, GpuPlatformToken, GpuPrincipalToken, GpuProcessIdentity,
    GpuProcessObservation, GpuProcessRole, GpuReconcileOutcome, GpuSettings, GpuWorkerSpec,
    VideoWorkerSpec,
};
use d2b_contracts_resource::v3::DeviceFunction;

/// The capabilities a fully admitted Device reaches.
///
/// Every declared shape the settings can select is covered, so a fixture that
/// is about worker lifecycle is not also about capability admission.
fn full_grants() -> GpuDeviceGrants {
    let mut names: Vec<DeviceFunction> = ["render-node", "dri", "udmabuf", "nvidia-ctl", "nvidia-uvm", "nvidia-device"]
        .iter()
        .filter_map(|name| DeviceFunction::parse(*name).ok())
        .collect();
    names.dedup();
    GpuDeviceGrants::new(names).expect("distinct capability names")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

fn admission(
    arbitration: DeviceArbitration,
    render_node_only: bool,
    generation: u64,
) -> GpuAuthorityAdmission {
    let owner = GpuOwnerProof::new(
        ResourceRef::parse("Zone/dev").unwrap(),
        ResourceRef::parse("Guest/workload").unwrap(),
        uid("123e4567-e89b-42d3-a456-426614174000"),
        uid("223e4567-e89b-42d3-a456-426614174001"),
        ResourceGeneration::new(generation).unwrap(),
    )
    .unwrap();
    GpuAuthorityAdmission::new(
        owner,
        GpuBackingToken::from_core([7; 32]),
        GpuPlatformToken::from_core([8; 32]),
        arbitration,
        if arbitration == DeviceArbitration::Shared {
            2
        } else {
            1
        },
        render_node_only,
        GpuPrincipalToken::from_core([9; 32]),
    )
    .unwrap()
}

#[derive(Default)]
struct LifecyclePort {
    events: Vec<&'static str>,
    next: u8,
    wrong_gpu_principal: bool,
    wrong_closure: bool,
}

impl GpuLifecycleEffectPort for LifecyclePort {
    fn reserve_authority(
        &mut self,
        _: &GpuAuthorityAdmission,
    ) -> Result<GpuAuthorityLease, GpuEffectError> {
        self.events.push("reserve");
        Ok(GpuAuthorityLease::from_core([1; 16]))
    }

    fn open_authorized_devices(
        &mut self,
        _: &GpuAuthorityAdmission,
        _: &GpuEffectTokenSet,
    ) -> Result<d2b_provider_device_gpu::GpuLaunchTicket, GpuEffectError> {
        self.events.push("open");
        Ok(d2b_provider_device_gpu::GpuLaunchTicket::from_core([2; 16]))
    }

    fn start_gpu_worker(
        &mut self,
        spec: &GpuWorkerSpec,
        _: &d2b_provider_device_gpu::GpuLaunchTicket,
        principal: &GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError> {
        self.events.push("gpu");
        self.next = self.next.saturating_add(1);
        let principal = if self.wrong_gpu_principal {
            GpuPrincipalToken::from_core([11; 32])
        } else {
            principal.clone()
        };
        Ok(GpuProcessIdentity::from_core(
            [self.next; 16],
            spec.process().role(),
            principal,
            platform.clone(),
            generation,
        ))
    }

    fn start_video_worker(
        &mut self,
        _: &VideoWorkerSpec,
        _: &d2b_provider_device_gpu::GpuLaunchTicket,
        principal: &GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError> {
        self.events.push("video");
        self.next = self.next.saturating_add(1);
        Ok(GpuProcessIdentity::from_core(
            [self.next; 16],
            GpuProcessRole::Video,
            principal.clone(),
            platform.clone(),
            generation,
        ))
    }

    fn observe_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuProcessObservation, GpuEffectError> {
        Ok(GpuProcessObservation::Matching(identity.clone()))
    }

    fn stop_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuClosureProof, GpuEffectError> {
        self.events.push(match identity.role() {
            GpuProcessRole::Video => "stop-video",
            _ => "stop-gpu",
        });
        if self.wrong_closure {
            Ok(GpuClosureProof::from_core(GpuProcessIdentity::from_core(
                [99; 16],
                identity.role(),
                identity.principal().clone(),
                identity.platform().clone(),
                identity.generation(),
            )))
        } else {
            Ok(GpuClosureProof::from_core(identity.clone()))
        }
    }

    fn release_authority(
        &mut self,
        _: GpuAuthorityLease,
        _: &[GpuClosureProof],
    ) -> Result<(), GpuEffectError> {
        self.events.push("release");
        Ok(())
    }
}

#[test]
fn lifecycle_reserves_before_effects_and_closes_video_before_gpu() {
    let admission = admission(DeviceArbitration::Exclusive, false, 1)
        .with_video_principal(GpuPrincipalToken::from_core([10; 32]))
        .unwrap();
    let tokens = GpuEffectTokenSet::from_core(vec![GpuEffectToken::from_core([1; 32])]).unwrap();
    let mut controller = GpuController::new_authorized(
        admission,
        GpuSettings {
            video_sidecar: true,
            ..GpuSettings::default()
        },
        full_grants(),
        tokens,
    )
    .unwrap();
    let mut port = LifecyclePort::default();
    assert_eq!(
        controller.reconcile_lifecycle(&mut port).unwrap(),
        GpuReconcileOutcome::Converged
    );
    assert_eq!(port.events, ["reserve", "open", "gpu", "video"]);
    controller.finalize_lifecycle(&mut port).unwrap();
    assert_eq!(
        port.events,
        [
            "reserve",
            "open",
            "gpu",
            "video",
            "stop-video",
            "stop-gpu",
            "release"
        ]
    );
}

#[test]
fn lifecycle_rejects_worker_identity_and_finalizes_owned_process() {
    let admission = admission(DeviceArbitration::Exclusive, false, 1);
    let tokens = GpuEffectTokenSet::from_core(vec![GpuEffectToken::from_core([1; 32])]).unwrap();
    let mut controller =
        GpuController::new_authorized(admission, GpuSettings::default(), full_grants(), tokens).unwrap();
    let mut port = LifecyclePort {
        wrong_gpu_principal: true,
        ..LifecyclePort::default()
    };

    assert_eq!(
        controller.reconcile_lifecycle(&mut port),
        Err(d2b_provider_device_gpu::GpuControllerError::Effect(
            GpuEffectError::WrongPrincipal
        ))
    );
    assert_eq!(
        controller.phase(),
        d2b_provider_device_gpu::GpuPhase::Failed
    );
    assert!(controller.gpu_identity().is_some());
    assert_eq!(
        controller.reconcile_lifecycle(&mut port),
        Err(d2b_provider_device_gpu::GpuControllerError::InvalidState)
    );
    assert_eq!(port.events, ["reserve", "open", "gpu"]);

    controller.finalize_lifecycle(&mut port).unwrap();
    assert_eq!(
        port.events,
        ["reserve", "open", "gpu", "stop-gpu", "release"]
    );
    assert_eq!(
        controller.phase(),
        d2b_provider_device_gpu::GpuPhase::Finalized
    );
    assert!(!controller.finalizer_installed());
    assert!(!controller.authority_reserved());
}

/// A video worker cannot obtain a different render node through its
/// arguments, and a decode mode cannot select a device its Device has no
/// admitted binding for.
///
/// The device set used to be a property of the launch template: the
/// `videoNvidiaDecode` setting chose `video-worker-nvidia`, and the closed
/// posture table for that name supplied the node list. Here the setting still
/// chooses which named capability the worker asks for, but the name has to be
/// an admitted `DeviceBinding` first. A Device with only the DRM node admits
/// no NVIDIA decode, and the refusal lands before any reservation, open, or
/// spawn.
#[test]
fn a_video_worker_needs_an_admitted_binding_for_its_decode_capabilities() {
    let admission = admission(DeviceArbitration::Exclusive, false, 1)
        .with_video_principal(GpuPrincipalToken::from_core([10; 32]))
        .unwrap();
    let tokens = GpuEffectTokenSet::from_core(vec![GpuEffectToken::from_core([1; 32])]).unwrap();
    let settings = GpuSettings {
        video_sidecar: true,
        video_nvidia_decode: true,
        ..GpuSettings::default()
    };
    let drm_only: Vec<DeviceFunction> = ["render-node", "dri", "udmabuf"]
        .iter()
        .filter_map(|name| DeviceFunction::parse(*name).ok())
        .collect();
    let mut controller = GpuController::new_authorized(
        admission.clone(),
        settings.clone(),
        GpuDeviceGrants::new(drm_only).unwrap(),
        tokens.clone(),
    )
    .unwrap();
    let mut port = LifecyclePort::default();

    assert_eq!(
        controller.missing_capabilities().len(),
        3,
        "the NVIDIA decode mode needs three capabilities this device never admitted"
    );
    assert_eq!(
        controller.reconcile_lifecycle(&mut port),
        Err(d2b_provider_device_gpu::GpuControllerError::DeviceCapabilityRefused {
            missing: 3
        })
    );
    assert!(
        port.events.is_empty(),
        "a capability refusal lands before any reservation, open, or spawn: {:?}",
        port.events
    );

    // The same settings, with the NVIDIA capabilities actually admitted,
    // start both workers. The decode mode selects the capability; the
    // binding decides whether it exists.
    let mut controller = GpuController::new_authorized(
        admission,
        settings,
        full_grants(),
        tokens,
    )
    .unwrap();
    assert!(controller.missing_capabilities().is_empty());
    assert_eq!(
        controller.reconcile_lifecycle(&mut port).unwrap(),
        GpuReconcileOutcome::Converged
    );
    assert_eq!(port.events, ["reserve", "open", "gpu", "video"]);
}

/// A GPU worker whose declared shape needs an unadmitted capability is
/// refused the same way.
///
/// The render-node-only shape asks for the DRM render node alone, so a Device
/// that admitted it starts; the full shape additionally needs the DRM device
/// and the cross-domain buffer node, and a Device that admitted neither is
/// refused rather than launched with a smaller grant than it declared.
#[test]
fn a_gpu_worker_shape_needs_every_capability_it_declares() {
    let admission = admission(DeviceArbitration::Exclusive, false, 1);
    let tokens = GpuEffectTokenSet::from_core(vec![GpuEffectToken::from_core([1; 32])]).unwrap();
    let render_node_only: Vec<DeviceFunction> = ["render-node"]
        .iter()
        .filter_map(|name| DeviceFunction::parse(*name).ok())
        .collect();

    let mut controller = GpuController::new_authorized(
        admission.clone(),
        GpuSettings {
            render_node_only: true,
            ..GpuSettings::default()
        },
        GpuDeviceGrants::new(render_node_only).unwrap(),
        tokens.clone(),
    )
    .unwrap();
    let mut port = LifecyclePort::default();
    assert!(controller.missing_capabilities().is_empty());
    assert_eq!(
        controller.reconcile_lifecycle(&mut port).unwrap(),
        GpuReconcileOutcome::Converged
    );
    assert_eq!(port.events, ["reserve", "open", "gpu"]);

    let full = GpuController::new_authorized(
        admission,
        GpuSettings::default(),
        full_grants(),
        tokens,
    )
    .unwrap();
    assert!(full.missing_capabilities().is_empty());
}

#[test]
fn lifecycle_rejects_video_without_a_separate_principal_before_effects() {
    let admission = admission(DeviceArbitration::Exclusive, false, 1);
    let tokens = GpuEffectTokenSet::from_core(vec![GpuEffectToken::from_core([1; 32])]).unwrap();
    let mut controller = GpuController::new_authorized(
        admission,
        GpuSettings {
            video_sidecar: true,
            ..GpuSettings::default()
        },
        full_grants(),
        tokens,
    )
    .unwrap();
    let mut port = LifecyclePort::default();

    assert_eq!(
        controller.reconcile_lifecycle(&mut port),
        Err(d2b_provider_device_gpu::GpuControllerError::Authority(
            GpuAuthorityError::PrincipalNotSeparated
        ))
    );
    assert!(port.events.is_empty());
}

#[test]
fn lifecycle_rejects_a_closure_proof_for_another_process() {
    let admission = admission(DeviceArbitration::Exclusive, false, 1);
    let tokens = GpuEffectTokenSet::from_core(vec![GpuEffectToken::from_core([1; 32])]).unwrap();
    let mut controller =
        GpuController::new_authorized(admission, GpuSettings::default(), full_grants(), tokens).unwrap();
    let mut port = LifecyclePort::default();
    controller.reconcile_lifecycle(&mut port).unwrap();
    port.wrong_closure = true;

    assert_eq!(
        controller.finalize_lifecycle(&mut port),
        Err(d2b_provider_device_gpu::GpuControllerError::Effect(
            GpuEffectError::CloseUnconfirmed
        ))
    );
    assert_eq!(
        controller.phase(),
        d2b_provider_device_gpu::GpuPhase::Failed
    );
}
