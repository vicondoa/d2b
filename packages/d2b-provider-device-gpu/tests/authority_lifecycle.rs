use d2b_contracts_zone_session::v3::{ResourceGeneration, ResourceRef, ResourceUid, device::DeviceArbitration};
use d2b_provider_device_gpu::{
    GpuAuthorityAdmission, GpuAuthorityError, GpuAuthorityIndex, GpuAuthorityLease,
    GpuBackingToken, GpuClosureProof, GpuController, GpuEffectError, GpuEffectToken,
    GpuEffectTokenSet, GpuLifecycleEffectPort, GpuOwnerProof, GpuPlatformToken, GpuPrincipalToken,
    GpuProcessIdentity, GpuProcessObservation, GpuProcessRole, GpuReconcileOutcome,
    GpuRecoveryRecord, GpuRecoverySnapshot, GpuSettings, GpuWorkerSpec, VideoWorkerSpec,
};

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

fn admission_with_backing(backing: [u8; 32], generation: u64) -> GpuAuthorityAdmission {
    let base = admission(DeviceArbitration::Exclusive, false, generation);
    GpuAuthorityAdmission::new(
        base.owner().clone(),
        GpuBackingToken::from_core(backing),
        base.platform().clone(),
        base.arbitration(),
        base.max_holders(),
        base.render_node_only(),
        base.gpu_principal().clone(),
    )
    .unwrap()
}

fn recovered_authority() -> (GpuAuthorityAdmission, GpuProcessIdentity, GpuAuthorityIndex) {
    let current = admission(DeviceArbitration::Exclusive, false, 1);
    let process = GpuProcessIdentity::from_core(
        [2; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(1).unwrap(),
    );
    let record = GpuRecoveryRecord::from_core(
        current.clone(),
        process.clone(),
        GpuAuthorityLease::from_core([3; 16]),
    );
    let index = GpuAuthorityIndex::rehydrate(GpuRecoverySnapshot::from_core(vec![record])).unwrap();
    (current, process, index)
}

#[test]
fn conflicting_full_device_claim_is_rejected_before_effects() {
    let first = admission(DeviceArbitration::Exclusive, false, 1);
    let second = admission(DeviceArbitration::Exclusive, false, 2);
    let mut index = GpuAuthorityIndex::new_for_tests_ready();
    index.reserve(first).unwrap();
    assert_eq!(
        index.reserve(second).unwrap_err(),
        GpuAuthorityError::ClaimConflict
    );
}

#[test]
fn leases_are_unique_across_shared_authority_backings() {
    let first = admission_with_backing([1; 32], 1);
    let second = admission_with_backing([2; 32], 2);
    let mut index = GpuAuthorityIndex::new_for_tests_ready();

    let first_lease = index.reserve(first).unwrap();
    let second_lease = index.reserve(second).unwrap();

    assert_ne!(first_lease, second_lease);
}

#[test]
fn duplicate_persisted_leases_quarantine_the_shared_backing() {
    let first = admission(DeviceArbitration::Exclusive, false, 1);
    let second = admission(DeviceArbitration::Exclusive, false, 2);
    let process_one = GpuProcessIdentity::from_core(
        [1; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(1).unwrap(),
    );
    let process_two = GpuProcessIdentity::from_core(
        [2; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(2).unwrap(),
    );
    let backing = first.backing().clone();
    let duplicate_lease = GpuAuthorityLease::from_core([3; 16]);
    let snapshot = GpuRecoverySnapshot::from_core(vec![
        GpuRecoveryRecord::from_core(first, process_one, duplicate_lease.clone()),
        GpuRecoveryRecord::from_core(second, process_two, duplicate_lease),
    ]);

    let index = GpuAuthorityIndex::rehydrate(snapshot).unwrap();
    assert!(index.is_quarantined(&backing));
    assert_eq!(index.holder_count(&backing), 0);
}

#[test]
fn wrong_principal_platform_and_generation_fail_before_process_binding() {
    let current = admission(DeviceArbitration::Exclusive, false, 4)
        .with_video_principal(GpuPrincipalToken::from_core([10; 32]))
        .unwrap();
    let mut index = GpuAuthorityIndex::new_for_tests_ready();
    let lease = index.reserve(current.clone()).unwrap();

    let wrong_principal = GpuProcessIdentity::from_core(
        [1; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([4; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(4).unwrap(),
    );
    assert_eq!(
        index.bind_process(&lease, wrong_principal),
        Err(GpuAuthorityError::ProcessPrincipalMismatch)
    );

    let video_using_gpu_principal = GpuProcessIdentity::from_core(
        [1; 16],
        GpuProcessRole::Video,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(4).unwrap(),
    );
    assert_eq!(
        index.bind_process(&lease, video_using_gpu_principal),
        Err(GpuAuthorityError::ProcessPrincipalMismatch)
    );

    let wrong_platform = GpuProcessIdentity::from_core(
        [1; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([5; 32]),
        ResourceGeneration::new(4).unwrap(),
    );
    assert_eq!(
        index.bind_process(&lease, wrong_platform),
        Err(GpuAuthorityError::PlatformMismatch)
    );

    let stale_generation = GpuProcessIdentity::from_core(
        [1; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(3).unwrap(),
    );
    assert_eq!(
        index.bind_process(&lease, stale_generation),
        Err(GpuAuthorityError::GenerationMismatch)
    );
}

#[test]
fn same_owner_rehydration_rejects_conflicting_gpu_video_admission() {
    let current = admission(DeviceArbitration::Exclusive, false, 1)
        .with_video_principal(GpuPrincipalToken::from_core([10; 32]))
        .unwrap();
    let conflicting = admission(DeviceArbitration::Exclusive, false, 1)
        .with_video_principal(GpuPrincipalToken::from_core([11; 32]))
        .unwrap();
    let gpu = GpuProcessIdentity::from_core(
        [2; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(1).unwrap(),
    );
    let video = GpuProcessIdentity::from_core(
        [3; 16],
        GpuProcessRole::Video,
        GpuPrincipalToken::from_core([11; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(1).unwrap(),
    );
    let backing = current.backing().clone();
    let records = vec![
        GpuRecoveryRecord::from_core(current, gpu, GpuAuthorityLease::from_core([3; 16])),
        GpuRecoveryRecord::from_core(conflicting, video, GpuAuthorityLease::from_core([3; 16])),
    ];

    let index = GpuAuthorityIndex::rehydrate(GpuRecoverySnapshot::from_core(records)).unwrap();
    assert!(index.is_quarantined(&backing));
}

#[test]
fn restart_adopts_one_matching_worker_and_quarantines_ambiguity() {
    let (current, process, mut index) = recovered_authority();
    assert!(matches!(
        index.adopt(
            &current,
            &[GpuProcessObservation::Matching(process.clone())]
        ),
        Ok(d2b_provider_device_gpu::GpuAdoption::Adopted(_))
    ));
    assert!(matches!(
        index.adopt(
            &current,
            &[
                GpuProcessObservation::Matching(process.clone()),
                GpuProcessObservation::Matching(process)
            ]
        ),
        Ok(d2b_provider_device_gpu::GpuAdoption::Quarantined)
    ));
    assert!(index.is_quarantined(current.backing()));
}

#[test]
fn ambiguous_restart_observation_quarantines_backing() {
    let (current, _, mut index) = recovered_authority();

    assert!(matches!(
        index.adopt(&current, &[GpuProcessObservation::Ambiguous]),
        Ok(d2b_provider_device_gpu::GpuAdoption::Quarantined)
    ));
    assert!(index.is_quarantined(current.backing()));
    assert_eq!(index.holder_count(current.backing()), 1);
}

#[test]
fn matching_observation_cannot_escape_ambiguous_adoption() {
    let (current, process, mut index) = recovered_authority();

    assert!(matches!(
        index.adopt(
            &current,
            &[
                GpuProcessObservation::Matching(process),
                GpuProcessObservation::Ambiguous
            ]
        ),
        Ok(d2b_provider_device_gpu::GpuAdoption::Quarantined)
    ));
    assert!(index.is_quarantined(current.backing()));
    assert_eq!(index.holder_count(current.backing()), 1);
}

#[test]
fn quarantined_restart_adoption_rejects_later_matching_worker() {
    let (current, process, mut index) = recovered_authority();
    assert!(matches!(
        index.adopt(&current, &[GpuProcessObservation::Ambiguous]),
        Ok(d2b_provider_device_gpu::GpuAdoption::Quarantined)
    ));

    assert!(matches!(
        index.adopt(&current, &[GpuProcessObservation::Matching(process)]),
        Ok(d2b_provider_device_gpu::GpuAdoption::Quarantined)
    ));
    assert_eq!(index.holder_count(current.backing()), 1);
    assert_eq!(index.reserve(current), Err(GpuAuthorityError::Quarantined));
}

#[test]
fn video_sidecar_recovery_keeps_one_host_authority() {
    let current = admission(DeviceArbitration::Exclusive, false, 1)
        .with_video_principal(GpuPrincipalToken::from_core([10; 32]))
        .unwrap();
    let gpu = GpuProcessIdentity::from_core(
        [2; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(1).unwrap(),
    );
    let video = GpuProcessIdentity::from_core(
        [3; 16],
        GpuProcessRole::Video,
        GpuPrincipalToken::from_core([10; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(1).unwrap(),
    );
    let record = GpuRecoveryRecord::from_core(
        current.clone(),
        gpu.clone(),
        GpuAuthorityLease::from_core([3; 16]),
    )
    .with_process(video.clone());
    let mut index =
        GpuAuthorityIndex::rehydrate(GpuRecoverySnapshot::from_core(vec![record])).unwrap();
    assert!(matches!(
        index.adopt(
            &current,
            &[
                GpuProcessObservation::Matching(gpu),
                GpuProcessObservation::Matching(video)
            ]
        ),
        Ok(d2b_provider_device_gpu::GpuAdoption::Adopted(_))
    ));
    assert!(!index.is_quarantined(current.backing()));
}

#[test]
fn cleanup_requires_the_owned_process_closure_proof() {
    let current = admission(DeviceArbitration::Exclusive, false, 1);
    let process = GpuProcessIdentity::from_core(
        [2; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(1).unwrap(),
    );
    let mut index = GpuAuthorityIndex::new_for_tests_ready();
    let lease = index.reserve(current.clone()).unwrap();
    index.bind_process(&lease, process.clone()).unwrap();
    let foreign = GpuProcessIdentity::from_core(
        [4; 16],
        GpuProcessRole::FullGpu,
        GpuPrincipalToken::from_core([9; 32]),
        GpuPlatformToken::from_core([8; 32]),
        ResourceGeneration::new(1).unwrap(),
    );
    assert_eq!(
        index.release_after_close(&lease, &GpuClosureProof::from_core(foreign)),
        Err(GpuAuthorityError::CloseUnconfirmed)
    );
    assert_eq!(index.holder_count(current.backing()), 1);
    index
        .release_after_close(&lease, &GpuClosureProof::from_core(process))
        .unwrap();
    assert_eq!(index.holder_count(current.backing()), 0);
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
        GpuController::new_authorized(admission, GpuSettings::default(), tokens).unwrap();
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
        GpuController::new_authorized(admission, GpuSettings::default(), tokens).unwrap();
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
