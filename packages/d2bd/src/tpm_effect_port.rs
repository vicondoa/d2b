//! Core-owned production adapter for the Device TPM Provider effect boundary.
//!
//! The Provider receives no broker handle, host locator, or Core migration
//! receipt. Core supplies the migration decision and an effect executor for
//! the state/runner operations; this adapter is the only place that maps the
//! private decision to the typed broker operation.

use d2b_contracts::types::{BundleOpId, VmId};
use d2b_core_controller::migration::LegacyTpmMigrationDecision;
use d2b_provider_device_tpm::{
    FlushLaunchTicket, LegacyMigrationOutcome, SignedBinaryRef, StateDirIntent, SwtpmSettings,
    SwtpmStartLaunchTicket, TpmEffectError, TpmEffectPort, TpmStatePreparationResult,
};

#[allow(dead_code)]
fn map_legacy_migration_outcome(
    outcome: d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome,
) -> LegacyMigrationOutcome {
    match outcome {
        d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome::Migrated => {
            LegacyMigrationOutcome::Migrated
        }
        d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome::AlreadyMigrated => {
            LegacyMigrationOutcome::AlreadyMigrated
        }
        d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome::NotApplicable => {
            LegacyMigrationOutcome::NotApplicable
        }
        d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome::Pending => {
            LegacyMigrationOutcome::Pending
        }
        d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome::Failed => {
            LegacyMigrationOutcome::Failed
        }
        d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome::Ambiguous => {
            LegacyMigrationOutcome::Ambiguous
        }
    }
}

/// Core-side executor for the non-migration TPM effects.
pub trait CoreTpmEffectExecutor {
    fn prepare_state_dir(
        &mut self,
        intent: &StateDirIntent,
    ) -> Result<TpmStatePreparationResult, TpmEffectError>;
    fn flush(&mut self, ticket: &FlushLaunchTicket) -> Result<(), TpmEffectError>;
    fn start(
        &mut self,
        ticket: &SwtpmStartLaunchTicket,
        settings: SwtpmSettings,
        binary: &SignedBinaryRef,
    ) -> Result<(), TpmEffectError>;
    fn stop(&mut self) -> Result<(), TpmEffectError>;
}

/// Production adapter bound to one Core-issued migration decision.
#[allow(dead_code)]
pub(crate) struct ProductionTpmEffectPort<'a, E> {
    state: &'a crate::ServerState,
    vm_id: VmId,
    migration_intent_ref: BundleOpId,
    migration_decision: LegacyTpmMigrationDecision,
    executor: E,
}

#[allow(dead_code)]
impl<'a, E> ProductionTpmEffectPort<'a, E> {
    pub(crate) fn new(
        state: &'a crate::ServerState,
        vm_id: VmId,
        migration_intent_ref: BundleOpId,
        migration_decision: LegacyTpmMigrationDecision,
        executor: E,
    ) -> Self {
        Self {
            state,
            vm_id,
            migration_intent_ref,
            migration_decision,
            executor,
        }
    }

    pub(crate) fn into_executor(self) -> E {
        self.executor
    }
}

impl<E: CoreTpmEffectExecutor> TpmEffectPort for ProductionTpmEffectPort<'_, E> {
    fn legacy_migration_required(&self) -> bool {
        self.migration_decision.requires_migration()
    }

    fn migrate_legacy_state(&mut self) -> Result<LegacyMigrationOutcome, TpmEffectError> {
        if !self.migration_decision.requires_migration()
            || !self
                .migration_decision
                .validates_binding(self.vm_id.as_str(), self.migration_intent_ref.as_str())
        {
            return Err(TpmEffectError::StateIntegrity);
        }
        let outcome = crate::dispatch_broker_legacy_tpm_migration(
            self.state,
            self.vm_id.clone(),
            self.migration_intent_ref.clone(),
        )
        .map_err(|_| TpmEffectError::Transient)?;
        Ok(map_legacy_migration_outcome(outcome))
    }

    fn prepare_state_dir(
        &mut self,
        intent: &StateDirIntent,
    ) -> Result<TpmStatePreparationResult, TpmEffectError> {
        self.executor.prepare_state_dir(intent)
    }

    fn flush(&mut self, ticket: &FlushLaunchTicket) -> Result<(), TpmEffectError> {
        self.executor.flush(ticket)
    }

    fn start(
        &mut self,
        ticket: &SwtpmStartLaunchTicket,
        settings: SwtpmSettings,
        binary: &SignedBinaryRef,
    ) -> Result<(), TpmEffectError> {
        self.executor.start(ticket, settings, binary)
    }

    fn stop(&mut self) -> Result<(), TpmEffectError> {
        self.executor.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome;

    #[test]
    fn broker_migration_outcomes_are_preserved_at_the_provider_boundary() {
        for (broker, provider) in [
            (
                LegacySwtpmMigrationOutcome::Migrated,
                LegacyMigrationOutcome::Migrated,
            ),
            (
                LegacySwtpmMigrationOutcome::AlreadyMigrated,
                LegacyMigrationOutcome::AlreadyMigrated,
            ),
            (
                LegacySwtpmMigrationOutcome::NotApplicable,
                LegacyMigrationOutcome::NotApplicable,
            ),
            (
                LegacySwtpmMigrationOutcome::Pending,
                LegacyMigrationOutcome::Pending,
            ),
            (
                LegacySwtpmMigrationOutcome::Failed,
                LegacyMigrationOutcome::Failed,
            ),
            (
                LegacySwtpmMigrationOutcome::Ambiguous,
                LegacyMigrationOutcome::Ambiguous,
            ),
        ] {
            assert_eq!(map_legacy_migration_outcome(broker), provider);
        }
    }
}
