use d2b_provider_device_tpm::{
    BinaryKind, FlushLaunchTicket, SignedBinaryRef, StateDirIntent, StateDirectoryToken,
    StateOwnerToken, SwtpmSettings, SwtpmStartLaunchTicket, TpmController, TpmEffectError,
    TpmEffectPort, TpmPhase, TpmStateObservation, TpmStateObservationKind,
    TpmStatePreparationResult,
};

#[derive(Default)]
struct FakePort {
    calls: Vec<&'static str>,
    stop_calls: usize,
}

impl TpmEffectPort for FakePort {
    fn prepare_state_dir(
        &mut self,
        _: &StateDirIntent,
    ) -> Result<TpmStatePreparationResult, TpmEffectError> {
        self.calls.push("prepare");
        Ok(TpmStatePreparationResult {
            observation: TpmStateObservation::from_core(TpmStateObservationKind::Fresh),
            flush_ticket: FlushLaunchTicket::from_core([1; 16]),
            swtpm_ticket: SwtpmStartLaunchTicket::from_core([2; 16]),
        })
    }

    fn flush(&mut self, _: &FlushLaunchTicket) -> Result<(), TpmEffectError> {
        self.calls.push("flush");
        Ok(())
    }

    fn start(
        &mut self,
        _: &SwtpmStartLaunchTicket,
        _: SwtpmSettings,
        _: &SignedBinaryRef,
    ) -> Result<(), TpmEffectError> {
        self.calls.push("start");
        Ok(())
    }

    fn stop(&mut self) -> Result<(), TpmEffectError> {
        self.stop_calls += 1;
        self.calls.push("stop");
        Ok(())
    }
}

fn controller() -> TpmController {
    TpmController::new(
        StateDirIntent::new(
            StateDirectoryToken::from_core([3; 32]),
            d2b_provider_device_tpm::TamperMarkerToken::from_core([4; 32]),
            StateOwnerToken::from_core([5; 16]),
        ),
        SwtpmSettings::default(),
        SignedBinaryRef::from_core(BinaryKind::Swtpm, [6; 32]),
    )
    .unwrap()
}

#[test]
fn flush_precedes_long_lived_swtpm_start() {
    let mut controller = controller();
    let mut port = FakePort::default();
    controller.reconcile(&mut port).unwrap();
    assert_eq!(controller.phase(), TpmPhase::Ready);
    assert_eq!(port.calls, ["prepare", "flush", "start"]);
}

#[test]
fn finalizer_stops_worker_but_preserves_volume() {
    let mut controller = controller();
    let mut port = FakePort::default();
    controller.reconcile(&mut port).unwrap();
    controller.finalize(&mut port).unwrap();
    assert_eq!(port.stop_calls, 1);
    assert!(!controller.finalizer_installed());
    assert!(controller.volume_preserved());
    assert_eq!(controller.phase(), TpmPhase::Finalized);
}
