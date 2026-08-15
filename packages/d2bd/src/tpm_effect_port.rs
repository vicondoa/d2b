//! Core-owned production adapter for the Device TPM Provider effect boundary.
//!
//! The Provider receives no broker handle, host locator, or Core migration
//! receipt. Core supplies the migration decision and an effect executor for
//! the state/runner operations; this adapter is the only place that maps the
//! private decision to the typed broker operation.

use std::time::Duration;

use d2b_contracts::{
    broker_wire::{
        BrokerCallerRole, BrokerRequest, BrokerResponse, RunnerRole, SpawnRunnerRequest,
    },
    types::{BundleOpId, PathClass, RoleId, VmId},
};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core_controller::migration::LegacyTpmMigrationDecision;
use d2b_provider_device_tpm::{
    BinaryKind, FlushLaunchTicket, LegacyMigrationOutcome, SignedBinaryRef, StateDirIntent,
    SwtpmSettings, SwtpmStartLaunchTicket, TpmController, TpmEffectError, TpmEffectPort,
    TpmReconcileOutcome, TpmStateObservation, TpmStateObservationKind, TpmStatePreparationResult,
};
use sha2::{Digest, Sha256};

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
        d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome::AdoptionRequired
        | d2b_contracts::broker_wire::LegacySwtpmMigrationOutcome::NeverProvisioned => {
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

/// Concrete daemon-side TPM effect executor.
///
/// All host paths, binaries, state markers, and pidfds remain inside the
/// trusted bundle/broker boundary. The Provider sees only the opaque tickets
/// returned by this executor.
pub(crate) struct LiveTpmEffectExecutor<'a> {
    state: &'a crate::ServerState,
    resolver: &'a BundleResolver,
    vm_id: VmId,
    caller_role: BrokerCallerRole,
    legacy_migration_required: bool,
    prepared_flush_ticket: Option<FlushLaunchTicket>,
    prepared_swtpm_ticket: Option<SwtpmStartLaunchTicket>,
}

impl<'a> LiveTpmEffectExecutor<'a> {
    pub(crate) fn new(
        state: &'a crate::ServerState,
        resolver: &'a BundleResolver,
        vm_id: VmId,
        caller_role: BrokerCallerRole,
        legacy_migration_required: bool,
    ) -> Self {
        Self {
            state,
            resolver,
            vm_id,
            caller_role,
            legacy_migration_required,
            prepared_flush_ticket: None,
            prepared_swtpm_ticket: None,
        }
    }

    fn ticket_bytes(&self, domain: &str, intent: &StateDirIntent) -> [u8; 16] {
        let mut hasher = Sha256::new();
        hasher.update(domain.as_bytes());
        hasher.update([0]);
        hasher.update(self.vm_id.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(intent.directory().as_bytes());
        hasher.update(intent.marker().as_bytes());
        hasher.update(intent.owner().as_bytes());
        let digest = hasher.finalize();
        let mut out = [0; 16];
        out.copy_from_slice(&digest[..16]);
        out
    }

    fn runner_intent(&self, role: &str) -> Result<BundleOpId, TpmEffectError> {
        let intent_id = crate::intent_id_runner(self.vm_id.as_str(), role);
        self.resolver
            .find_runner_intent(&intent_id)
            .map(|intent| BundleOpId::new(intent.intent_id.clone()))
            .ok_or(TpmEffectError::SpawnRejected)
    }

    fn spawn(
        &self,
        role: RunnerRole,
        role_id: &str,
        intent: BundleOpId,
        timeout: Duration,
    ) -> Result<
        (
            d2b_contracts::broker_wire::SpawnRunnerResponse,
            Vec<std::os::fd::RawFd>,
        ),
        TpmEffectError,
    > {
        crate::dispatch_broker_request_with_fds_timeout_as(
            self.state,
            BrokerRequest::SpawnRunner(SpawnRunnerRequest {
                vm_id: self.vm_id.clone(),
                role_id: RoleId::new(role_id),
                role,
                bundle_runner_intent_ref: intent,
                runtime_allocations: Vec::new(),
                tracing_span_id: None,
                workload_identity: None,
            }),
            self.caller_role.clone(),
            timeout,
        )
        .map_err(|_| TpmEffectError::Transient)
        .and_then(|(response, fds)| match response {
            BrokerResponse::SpawnRunner(response) => Ok((response, fds)),
            BrokerResponse::Error(_) => {
                crate::close_received_fds(&fds);
                Err(TpmEffectError::SpawnRejected)
            }
            _ => {
                crate::close_received_fds(&fds);
                Err(TpmEffectError::SpawnRejected)
            }
        })
    }

    fn cleanup_failed_start(
        &self,
        response: &d2b_contracts::broker_wire::SpawnRunnerResponse,
        received_fds: &[std::os::fd::RawFd],
    ) {
        let removed = {
            let _guard = self.state.pidfd_table.mutation_guard();
            let removed = self.state.pidfd_table.deregister_if_matches(
                self.vm_id.as_str(),
                "swtpm",
                response.pid,
                response.start_time_ticks,
            );
            if removed {
                let _ = self.state.pidfd_table.snapshot();
            }
            removed
        };
        if removed {
            tracing::warn!(
                vm = %self.vm_id,
                role = "swtpm",
                pid = response.pid,
                "removed failed TPM runner registration"
            );
        }
        crate::stop_unregistered_spawned_runner(
            self.state,
            self.vm_id.as_str(),
            "swtpm",
            response,
            received_fds,
            self.caller_role.clone(),
        );
        crate::close_received_fds(received_fds);
    }
}

impl CoreTpmEffectExecutor for LiveTpmEffectExecutor<'_> {
    fn prepare_state_dir(
        &mut self,
        intent: &StateDirIntent,
    ) -> Result<TpmStatePreparationResult, TpmEffectError> {
        let response = crate::dispatch_broker_request_as(
            self.state,
            BrokerRequest::PrepareStateDir(d2b_contracts::broker_wire::PrepareDirRequest {
                vm_id: self.vm_id.clone(),
                path_class: PathClass::Vm,
                tracing_span_id: None,
            }),
            self.caller_role.clone(),
        )
        .map_err(|_| TpmEffectError::Transient)?;
        if !matches!(response, BrokerResponse::Ack(_)) {
            return Err(TpmEffectError::StateIntegrity);
        }
        let flush_ticket =
            FlushLaunchTicket::from_core(self.ticket_bytes("d2b:tpm-flush-ticket/v2", intent));
        let swtpm_ticket =
            SwtpmStartLaunchTicket::from_core(self.ticket_bytes("d2b:tpm-start-ticket/v2", intent));
        self.prepared_flush_ticket = Some(flush_ticket.clone());
        self.prepared_swtpm_ticket = Some(swtpm_ticket.clone());
        Ok(TpmStatePreparationResult {
            observation: TpmStateObservation::from_core(if self.legacy_migration_required {
                TpmStateObservationKind::ExistingWithMarker
            } else {
                TpmStateObservationKind::Fresh
            }),
            flush_ticket,
            swtpm_ticket,
        })
    }

    fn flush(&mut self, ticket: &FlushLaunchTicket) -> Result<(), TpmEffectError> {
        if self.prepared_flush_ticket.as_ref() != Some(ticket) {
            return Err(TpmEffectError::StateIntegrity);
        }
        let intent = self.runner_intent("swtpm-flush")?;
        let (response, fds) = self.spawn(
            RunnerRole::SwtpmFlush,
            "swtpm-flush",
            intent,
            Duration::from_secs(30),
        )?;
        crate::close_received_fds(&fds);
        crate::wait_for_one_shot_exit(
            response.pid,
            response.start_time_ticks,
            Duration::from_secs(30),
        )
        .map_err(|_| TpmEffectError::FlushFailed)
    }

    fn start(
        &mut self,
        ticket: &SwtpmStartLaunchTicket,
        settings: SwtpmSettings,
        binary: &SignedBinaryRef,
    ) -> Result<(), TpmEffectError> {
        if self.prepared_swtpm_ticket.as_ref() != Some(ticket)
            || binary.kind() != BinaryKind::Swtpm
            || d2b_provider_device_tpm::SwtpmArgv::for_settings(settings).is_err()
        {
            return Err(TpmEffectError::SpawnRejected);
        }
        if self
            .state
            .pidfd_table
            .still_alive_same_start_time(self.vm_id.as_str(), "swtpm")
        {
            return Ok(());
        }
        {
            let _mguard = self.state.pidfd_table.mutation_guard();
            if self
                .state
                .pidfd_table
                .deregister(self.vm_id.as_str(), "swtpm")
                .is_some()
            {
                let _ = self.state.pidfd_table.snapshot();
            }
        }
        let intent = self.runner_intent("swtpm")?;
        let (response, fds) =
            self.spawn(RunnerRole::Swtpm, "swtpm", intent, Duration::from_secs(30))?;
        let pidfd = match crate::duplicate_received_fd(&fds, response.pidfd_index, "TPM pidfd") {
            Ok(pidfd) => pidfd,
            Err(_) => {
                self.cleanup_failed_start(&response, &fds);
                return Err(TpmEffectError::Transient);
            }
        };
        let registration_result = {
            let _guard = self.state.pidfd_table.mutation_guard();
            (|| {
                self.state.pidfd_table.register(
                    self.vm_id.as_str().to_owned(),
                    "swtpm".to_owned(),
                    crate::supervisor::pidfd_table::PidfdEntry {
                        pidfd,
                        pid: response.pid,
                        start_time_ticks: response.start_time_ticks,
                    },
                )?;
                self.state.pidfd_table.snapshot()
            })()
        };
        if let Err(error) = registration_result {
            let duplicate = matches!(
                error,
                crate::supervisor::pidfd_table::PidfdTableError::DuplicateRegistration { .. }
            );
            self.cleanup_failed_start(&response, &fds);
            return Err(if duplicate {
                TpmEffectError::SpawnRejected
            } else {
                TpmEffectError::Transient
            });
        }
        if let Err(error) = crate::write_runner_snapshot(
            self.state,
            self.vm_id.as_str(),
            "swtpm",
            RunnerRole::Swtpm,
            response.pid,
            response.start_time_ticks,
        ) {
            self.cleanup_failed_start(&response, &fds);
            tracing::warn!(error = %error, "TPM runner snapshot persistence failed");
            return Err(TpmEffectError::Transient);
        }
        crate::close_received_fds(&fds);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), TpmEffectError> {
        crate::stop_vm_pidfd_role(
            self.state,
            self.caller_role.clone(),
            "device-tpm",
            self.vm_id.as_str(),
            "swtpm",
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .map(|_| ())
        .map_err(|_| TpmEffectError::Transient)
    }
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

/// Production Device controller reconcile callsite.
///
/// Core supplies the migration decision and opaque state intent; the daemon
/// supplies only the concrete broker-backed executor. The migration receipt
/// never crosses into the Provider crate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconcile_device_tpm(
    state: &crate::ServerState,
    resolver: &BundleResolver,
    vm_id: VmId,
    migration_intent_ref: BundleOpId,
    migration_decision: LegacyTpmMigrationDecision,
    state_intent: StateDirIntent,
    settings: SwtpmSettings,
    binary: SignedBinaryRef,
    caller_role: BrokerCallerRole,
) -> Result<TpmReconcileOutcome, d2b_provider_device_tpm::TpmControllerError> {
    let executor = LiveTpmEffectExecutor::new(
        state,
        resolver,
        vm_id.clone(),
        caller_role,
        migration_decision.requires_migration(),
    );
    let mut effect = ProductionTpmEffectPort::new(
        state,
        vm_id,
        migration_intent_ref,
        migration_decision,
        executor,
    );
    let mut controller = TpmController::new(state_intent, settings, binary)?;
    controller.reconcile(&mut effect)
}

/// Registered production Device controller entry point.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DeviceTpmControllerRegistration {
    registered: bool,
}

impl DeviceTpmControllerRegistration {
    pub(crate) const fn is_registered(self) -> bool {
        self.registered
    }

    /// Reconcile one Core-admitted Device through the live broker executor.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconcile(
        self,
        state: &crate::ServerState,
        resolver: &BundleResolver,
        vm_id: VmId,
        migration_intent_ref: BundleOpId,
        migration_decision: LegacyTpmMigrationDecision,
        state_intent: StateDirIntent,
        settings: SwtpmSettings,
        binary: SignedBinaryRef,
        caller_role: BrokerCallerRole,
    ) -> Result<TpmReconcileOutcome, d2b_provider_device_tpm::TpmControllerError> {
        reconcile_device_tpm(
            state,
            resolver,
            vm_id,
            migration_intent_ref,
            migration_decision,
            state_intent,
            settings,
            binary,
            caller_role,
        )
    }
}

/// Register the real Device TPM controller at the daemon/Core composition
/// boundary. The returned registration is retained by the Zone runtime.
pub(crate) fn register_device_tpm_controller() -> DeviceTpmControllerRegistration {
    DeviceTpmControllerRegistration { registered: true }
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
