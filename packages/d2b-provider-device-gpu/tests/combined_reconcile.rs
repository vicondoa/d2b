use d2b_contracts::v3::{ResourceUid, device::DeviceArbitration};
use d2b_provider_device_gpu::{
    GpuController, GpuEffectError, GpuEffectPort, GpuEffectToken, GpuEffectTokenSet,
    GpuLaunchTicket, GpuPhase, GpuProcessRole, GpuReconcileOutcome, GpuSettings,
};

#[derive(Default)]
struct FakePort {
    starts: Vec<GpuProcessRole>,
    stops: Vec<GpuProcessRole>,
}

impl GpuEffectPort for FakePort {
    fn open_devices(
        &mut self,
        _: &ResourceUid,
        _: &GpuEffectTokenSet,
    ) -> Result<GpuLaunchTicket, GpuEffectError> {
        Ok(GpuLaunchTicket::from_core([1; 16]))
    }

    fn start(&mut self, role: GpuProcessRole, _: &GpuLaunchTicket) -> Result<(), GpuEffectError> {
        self.starts.push(role);
        Ok(())
    }

    fn stop(&mut self, role: GpuProcessRole) -> Result<(), GpuEffectError> {
        self.stops.push(role);
        Ok(())
    }
}

#[test]
fn video_starts_only_after_gpu_worker_is_ready() {
    let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let settings = GpuSettings {
        video_sidecar: true,
        ..GpuSettings::default()
    };
    let tokens = GpuEffectTokenSet::from_core(vec![GpuEffectToken::from_core([2; 32])]).unwrap();
    let mut controller =
        GpuController::new(uid, DeviceArbitration::Exclusive, settings, tokens).unwrap();
    let mut port = FakePort::default();
    assert_eq!(
        controller.reconcile(&mut port).unwrap(),
        GpuReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), GpuPhase::Ready);
    assert_eq!(
        port.starts,
        [GpuProcessRole::FullGpu, GpuProcessRole::Video]
    );
}
