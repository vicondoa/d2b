use d2b_provider_device_tpm::{
    BinaryKind, FlushLaunchTicket, LegacyMigrationOutcome, SignedBinaryRef, StateDirIntent,
    StateDirectoryToken, StateOwnerToken, SwtpmSettings, SwtpmStartLaunchTicket, TpmController,
    TpmControllerError, TpmEffectError, TpmEffectPort, TpmPhase, TpmReconcileOutcome,
    TpmStateObservation, TpmStateObservationKind, TpmStatePreparationResult,
};

#[derive(Default)]
struct FakePort {
    calls: Vec<&'static str>,
    stop_calls: usize,
    migration: Option<LegacyMigrationOutcome>,
    migration_required: bool,
}

impl TpmEffectPort for FakePort {
    fn legacy_migration_required(&self) -> bool {
        self.migration_required
    }

    fn migrate_legacy_state(&mut self) -> Result<LegacyMigrationOutcome, TpmEffectError> {
        self.calls.push("migrate");
        Ok(self
            .migration
            .unwrap_or(LegacyMigrationOutcome::NotApplicable))
    }

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
    TpmController::new_for_tests(
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
fn no_migration_skips_migration_and_flush_precedes_long_lived_start() {
    let mut controller = controller();
    let mut port = FakePort {
        migration: Some(LegacyMigrationOutcome::NotApplicable),
        ..FakePort::default()
    };
    controller.reconcile(&mut port).unwrap();
    assert_eq!(controller.phase(), TpmPhase::Ready);
    assert_eq!(port.calls, ["prepare", "flush", "start"]);
}

#[test]
fn required_migration_precedes_state_preparation() {
    let mut controller = controller();
    let mut port = FakePort {
        migration: Some(LegacyMigrationOutcome::Migrated),
        migration_required: true,
        ..FakePort::default()
    };
    assert_eq!(
        controller.reconcile(&mut port),
        Ok(TpmReconcileOutcome::Converged)
    );
    assert_eq!(port.calls, ["migrate", "prepare", "flush", "start"]);
}

#[test]
fn already_migrated_and_not_applicable_allow_state_preparation() {
    for outcome in [
        LegacyMigrationOutcome::AlreadyMigrated,
        LegacyMigrationOutcome::NotApplicable,
    ] {
        let mut controller = controller();
        let mut port = FakePort {
            migration: Some(outcome),
            migration_required: true,
            ..FakePort::default()
        };
        assert_eq!(
            controller.reconcile(&mut port),
            Ok(TpmReconcileOutcome::Converged)
        );
        assert_eq!(port.calls, ["migrate", "prepare", "flush", "start"]);
    }
}

#[test]
fn pending_migration_retries_without_preparing_state() {
    let mut controller = controller();
    let mut port = FakePort {
        migration: Some(LegacyMigrationOutcome::Pending),
        migration_required: true,
        ..FakePort::default()
    };
    assert_eq!(
        controller.reconcile(&mut port),
        Ok(TpmReconcileOutcome::Transient)
    );
    assert_eq!(controller.phase(), TpmPhase::MigratingLegacyState);
    assert_eq!(port.calls, ["migrate"]);
}

#[test]
fn failed_and_ambiguous_migrations_fail_closed_before_preparation() {
    for outcome in [
        LegacyMigrationOutcome::Failed,
        LegacyMigrationOutcome::Ambiguous,
    ] {
        let mut controller = controller();
        let mut port = FakePort {
            migration: Some(outcome),
            migration_required: true,
            ..FakePort::default()
        };
        assert_eq!(
            controller.reconcile(&mut port),
            Err(TpmControllerError::LegacyMigration(outcome))
        );
        assert_eq!(controller.phase(), TpmPhase::Failed);
        assert_eq!(port.calls, ["migrate"]);
    }
}

#[test]
fn finalizer_stops_worker_but_preserves_volume() {
    let mut controller = controller();
    let mut port = FakePort {
        migration: Some(LegacyMigrationOutcome::NotApplicable),
        ..FakePort::default()
    };
    controller.reconcile(&mut port).unwrap();
    controller.finalize(&mut port).unwrap();
    assert_eq!(port.stop_calls, 1);
    assert!(!controller.finalizer_installed());
    assert!(controller.volume_preserved());
    assert_eq!(controller.phase(), TpmPhase::Finalized);
}
