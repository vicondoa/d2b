use d2b_provider_device_tpm::{
    BinaryKind, FlushLaunchTicket, SignedBinaryRef, StateDirIntent, StateDirectoryToken,
    StateOwnerToken, SwtpmSettings, SwtpmStartLaunchTicket, TpmController, TpmEffectError,
    TpmEffectPort, TpmPhase, TpmStateObservation, TpmStateObservationKind,
    TpmStatePreparationResult,
};

struct MissingSwtpm;

impl TpmEffectPort for MissingSwtpm {
    fn prepare_state_dir(
        &mut self,
        _: &StateDirIntent,
    ) -> Result<TpmStatePreparationResult, TpmEffectError> {
        Ok(TpmStatePreparationResult {
            observation: TpmStateObservation::from_core(
                TpmStateObservationKind::ExistingWithMarker,
            ),
            flush_ticket: FlushLaunchTicket::from_core([1; 16]),
            swtpm_ticket: SwtpmStartLaunchTicket::from_core([2; 16]),
        })
    }

    fn flush(&mut self, _: &FlushLaunchTicket) -> Result<(), TpmEffectError> {
        Ok(())
    }

    fn start(
        &mut self,
        _: &SwtpmStartLaunchTicket,
        _: SwtpmSettings,
        _: &SignedBinaryRef,
    ) -> Result<(), TpmEffectError> {
        Err(TpmEffectError::SwtpmMissing)
    }

    fn stop(&mut self) -> Result<(), TpmEffectError> {
        Ok(())
    }
}

#[test]
fn missing_swtpm_never_reports_ready() {
    let mut controller = TpmController::new(
        StateDirIntent::new(
            StateDirectoryToken::from_core([3; 32]),
            d2b_provider_device_tpm::TamperMarkerToken::from_core([4; 32]),
            StateOwnerToken::from_core([5; 16]),
        ),
        SwtpmSettings::default(),
        SignedBinaryRef::from_core(BinaryKind::Swtpm, [6; 32]),
    )
    .unwrap();
    assert!(controller.reconcile(&mut MissingSwtpm).is_err());
    assert_eq!(controller.phase(), TpmPhase::Failed);
}
