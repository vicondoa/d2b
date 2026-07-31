//! Fixed core-controller startup and restart policy.
//!
//! This is a library module rather than a binary entrypoint. The production
//! ResourceClient, authenticated local ComponentSession connector, and store
//! watch dispatcher are intentionally absent, so exposing an executable would
//! advertise a process that cannot perform its specified startup contract.

use crate::controllers::{AggregateHealth, CoreHandlerRegistry};

/// Fixed process startup stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StartupStage {
    WaitingForStore,
    WaitingForResourceApi,
    WaitingForAuthenticatedSession,
    RecoveringHandlers,
    PublishingConfiguration,
    ReconcilingSystemCore,
    Ready,
    Degraded,
}

/// Trusted Zone runtime readiness observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeReadiness {
    pub store_ready: bool,
    pub resource_api_ready: bool,
    pub local_bus_ready: bool,
    pub authenticated_system_core_session: bool,
}

/// Bounded recovery facts read from authoritative resources and revision log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoverySnapshot {
    pub checkpoint_revision: u64,
    pub active_configuration_revision: u64,
    pub provider_lease_count: u32,
    pub controller_lease_count: u32,
    pub ambiguous_operation_count: u32,
}

/// Closed startup refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupError {
    RuntimeNotReady,
    AuthenticationUnavailable,
    InvalidRecoverySnapshot,
    MandatoryHandlerNotReady,
}

impl StartupError {
    /// Return a stable, identity-free reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::RuntimeNotReady => "core-runtime-not-ready",
            Self::AuthenticationUnavailable => "core-session-authentication-unavailable",
            Self::InvalidRecoverySnapshot => "core-recovery-snapshot-invalid",
            Self::MandatoryHandlerNotReady => "core-mandatory-handler-not-ready",
        }
    }
}

impl core::fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for StartupError {}

/// Fixed process coordinator. It owns no private authoritative ledger.
#[derive(Debug, Default)]
pub struct CoreProcess {
    handlers: CoreHandlerRegistry,
    stage: StartupStage,
    recovery: Option<RecoverySnapshot>,
}

impl Default for StartupStage {
    fn default() -> Self {
        Self::WaitingForStore
    }
}

impl CoreProcess {
    /// Construct the single fixed handler process.
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the fixed handler registry.
    pub const fn handlers(&self) -> &CoreHandlerRegistry {
        &self.handlers
    }

    /// Mutably borrow the fixed handler registry for isolated handler updates.
    pub const fn handlers_mut(&mut self) -> &mut CoreHandlerRegistry {
        &mut self.handlers
    }

    /// Return the current startup stage.
    pub const fn stage(&self) -> StartupStage {
        self.stage
    }

    /// Advance only through runtime-owned service readiness and exact session
    /// authentication. No caller-provided subject claim is accepted here.
    pub fn connect_runtime(&mut self, readiness: RuntimeReadiness) -> Result<(), StartupError> {
        self.stage = if !readiness.store_ready {
            StartupStage::WaitingForStore
        } else if !readiness.resource_api_ready || !readiness.local_bus_ready {
            StartupStage::WaitingForResourceApi
        } else if !readiness.authenticated_system_core_session {
            StartupStage::WaitingForAuthenticatedSession
        } else {
            self.handlers.begin_recovery();
            StartupStage::RecoveringHandlers
        };
        match self.stage {
            StartupStage::RecoveringHandlers => Ok(()),
            StartupStage::WaitingForAuthenticatedSession => {
                Err(StartupError::AuthenticationUnavailable)
            }
            _ => Err(StartupError::RuntimeNotReady),
        }
    }

    /// Accept one authoritative relist/checkpoint snapshot.
    pub fn recover(&mut self, snapshot: RecoverySnapshot) -> Result<(), StartupError> {
        if self.stage != StartupStage::RecoveringHandlers
            || snapshot.checkpoint_revision == 0
            || snapshot.active_configuration_revision == 0
        {
            return Err(StartupError::InvalidRecoverySnapshot);
        }
        self.recovery = Some(snapshot);
        self.stage = StartupStage::PublishingConfiguration;
        Ok(())
    }

    /// Mark configuration recovery complete and admit system-core reconcile.
    pub fn configuration_published(&mut self) -> Result<(), StartupError> {
        if self.stage != StartupStage::PublishingConfiguration || self.recovery.is_none() {
            return Err(StartupError::InvalidRecoverySnapshot);
        }
        self.stage = StartupStage::ReconcilingSystemCore;
        Ok(())
    }

    /// Publish process readiness only from aggregate mandatory handler health.
    pub fn publish_readiness(&mut self) -> Result<StartupStage, StartupError> {
        if self.stage != StartupStage::ReconcilingSystemCore {
            return Err(StartupError::MandatoryHandlerNotReady);
        }
        self.stage = match self.handlers.aggregate_health() {
            AggregateHealth::Ready => StartupStage::Ready,
            AggregateHealth::Degraded => StartupStage::Degraded,
            AggregateHealth::Pending | AggregateHealth::Failed | AggregateHealth::Unknown => {
                return Err(StartupError::MandatoryHandlerNotReady);
            }
        };
        Ok(self.stage)
    }

    /// Begin restart recovery without cleanup or trusting prior observations.
    pub fn restart(&mut self) {
        self.handlers.begin_recovery();
        self.recovery = None;
        self.stage = StartupStage::WaitingForAuthenticatedSession;
    }
}

#[cfg(test)]
mod tests {
    use crate::controllers::{CoreHandlerKind, HandlerOutcome, HandlerPhase, HandlerStatus};

    use super::*;

    fn ready_status() -> HandlerStatus {
        HandlerStatus {
            phase: HandlerPhase::Ready,
            outcome: HandlerOutcome::Converged,
            observed_generation: 1,
            queued: 0,
            running: 0,
            last_watch_revision: 1,
            checkpoint_revision: 1,
            last_reconciled_tick: 1,
            retry_after_tick: None,
        }
    }

    fn runtime_ready(authenticated: bool) -> RuntimeReadiness {
        RuntimeReadiness {
            store_ready: true,
            resource_api_ready: true,
            local_bus_ready: true,
            authenticated_system_core_session: authenticated,
        }
    }

    fn recovery() -> RecoverySnapshot {
        RecoverySnapshot {
            checkpoint_revision: 1,
            active_configuration_revision: 1,
            provider_lease_count: 0,
            controller_lease_count: 0,
            ambiguous_operation_count: 0,
        }
    }

    #[test]
    fn startup_reaches_ready_only_after_runtime_recovery_and_mandatory_handlers() {
        let mut process = CoreProcess::new();
        process.connect_runtime(runtime_ready(true)).unwrap();
        process.recover(recovery()).unwrap();
        process.configuration_published().unwrap();
        for kind in CoreHandlerKind::ALL {
            if kind.mandatory() {
                process.handlers_mut().update(kind, ready_status()).unwrap();
            }
        }
        assert_eq!(process.publish_readiness(), Ok(StartupStage::Ready));
    }

    #[test]
    fn unauthenticated_runtime_is_rejected_before_handler_recovery() {
        let mut process = CoreProcess::new();
        assert_eq!(
            process.connect_runtime(runtime_ready(false)),
            Err(StartupError::AuthenticationUnavailable)
        );
        assert_eq!(
            process.stage(),
            StartupStage::WaitingForAuthenticatedSession
        );
    }

    #[test]
    fn invalid_recovery_snapshot_is_rejected() {
        let mut process = CoreProcess::new();
        process.connect_runtime(runtime_ready(true)).unwrap();
        assert_eq!(
            process.recover(RecoverySnapshot {
                checkpoint_revision: 0,
                ..recovery()
            }),
            Err(StartupError::InvalidRecoverySnapshot)
        );
    }

    #[test]
    fn restart_discards_process_local_recovery_and_preserves_unknown() {
        let mut process = CoreProcess::new();
        process.connect_runtime(runtime_ready(true)).unwrap();
        process.recover(recovery()).unwrap();
        process.restart();
        assert_eq!(
            process.stage(),
            StartupStage::WaitingForAuthenticatedSession
        );
        for kind in CoreHandlerKind::ALL {
            assert_eq!(
                process.handlers().status(kind).phase,
                HandlerPhase::Recovering
            );
        }
    }

    #[test]
    fn a_missing_mandatory_handler_blocks_readiness() {
        let mut process = CoreProcess::new();
        process.connect_runtime(runtime_ready(true)).unwrap();
        process.recover(recovery()).unwrap();
        process.configuration_published().unwrap();
        for kind in CoreHandlerKind::ALL {
            if kind.mandatory() && kind != CoreHandlerKind::Authorization {
                process.handlers_mut().update(kind, ready_status()).unwrap();
            }
        }
        assert_eq!(
            process.publish_readiness(),
            Err(StartupError::MandatoryHandlerNotReady)
        );
    }
}
