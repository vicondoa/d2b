//! Fixed core-controller startup and restart policy.
//!
//! This module is the fixed production process coordinator owned by the Zone
//! runtime. The daemon supplies the opened store, registered ResourceService
//! endpoint, authenticated local session, and provider path; this entrypoint
//! owns only startup ordering and handler readiness, never a private
//! replacement ledger.

use crate::controllers::{AggregateHealth, CoreHandlerKind, CoreHandlerRegistry, HandlerPhase};

/// Fixed process startup stage.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StartupStage {
    #[default]
    WaitingForStore,
    WaitingForResourceApi,
    WaitingForControllerEndpoint,
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
    /// Set only after the production ResourceService/controller endpoint is
    /// registered on the Zone-local path.
    pub controller_endpoint_registered: bool,
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
    /// Set only after the registered store watch has accepted its cursor.
    pub watch_admitted: bool,
}

/// Closed startup refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupError {
    RuntimeNotReady,
    ControllerEndpointUnavailable,
    AuthenticationUnavailable,
    WatchAdmissionUnavailable,
    InvalidRecoverySnapshot,
    MandatoryHandlerNotReady,
}

impl StartupError {
    /// Return a stable, identity-free reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::RuntimeNotReady => "core-runtime-not-ready",
            Self::ControllerEndpointUnavailable => "core-controller-endpoint-unavailable",
            Self::AuthenticationUnavailable => "core-session-authentication-unavailable",
            Self::WatchAdmissionUnavailable => "core-watch-admission-unavailable",
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
        } else if !readiness.controller_endpoint_registered {
            StartupStage::WaitingForControllerEndpoint
        } else if !readiness.authenticated_system_core_session {
            StartupStage::WaitingForAuthenticatedSession
        } else {
            self.handlers.begin_recovery();
            StartupStage::RecoveringHandlers
        };
        match self.stage {
            StartupStage::RecoveringHandlers => Ok(()),
            StartupStage::WaitingForControllerEndpoint => {
                Err(StartupError::ControllerEndpointUnavailable)
            }
            StartupStage::WaitingForAuthenticatedSession => {
                Err(StartupError::AuthenticationUnavailable)
            }
            _ => Err(StartupError::RuntimeNotReady),
        }
    }

    /// Accept one authoritative relist/checkpoint snapshot.
    pub fn recover(&mut self, snapshot: RecoverySnapshot) -> Result<(), StartupError> {
        if self.stage != StartupStage::RecoveringHandlers {
            return Err(StartupError::InvalidRecoverySnapshot);
        }
        if !snapshot.watch_admitted {
            return Err(StartupError::WatchAdmissionUnavailable);
        }
        if snapshot.active_configuration_revision == 0 || snapshot.ambiguous_operation_count != 0 {
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

    /// Publish process readiness only after watch admission and aggregate
    /// mandatory handler health.
    pub fn publish_readiness(&mut self) -> Result<StartupStage, StartupError> {
        if self.stage != StartupStage::ReconcilingSystemCore {
            return Err(StartupError::MandatoryHandlerNotReady);
        }
        if self.handlers.status(CoreHandlerKind::Watches).phase != HandlerPhase::Ready {
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

    /// Admit the fixed controller set after the Zone runtime has registered
    /// its production endpoint and admitted the store watch.
    ///
    /// This method only advances startup through controller reconciliation.
    /// The caller must update every handler from the real controller/watch
    /// path before calling [`Self::publish_readiness`].
    pub fn start_production(
        &mut self,
        readiness: RuntimeReadiness,
        recovery: RecoverySnapshot,
    ) -> Result<StartupStage, StartupError> {
        self.connect_runtime(readiness)?;
        self.recover(recovery)?;
        self.configuration_published()?;
        Ok(self.stage)
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
            controller_endpoint_registered: true,
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
            watch_admitted: true,
        }
    }

    #[test]
    fn startup_reaches_ready_only_after_runtime_recovery_and_mandatory_handlers() {
        let mut process = CoreProcess::new();
        process.connect_runtime(runtime_ready(true)).unwrap();
        process.recover(recovery()).unwrap();
        process.configuration_published().unwrap();
        for kind in CoreHandlerKind::ALL {
            if kind.mandatory() || kind == CoreHandlerKind::Watches {
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
    fn an_empty_store_checkpoint_is_not_fabricated() {
        let mut process = CoreProcess::new();
        process.connect_runtime(runtime_ready(true)).unwrap();
        assert_eq!(
            process.recover(RecoverySnapshot {
                checkpoint_revision: 0,
                ..recovery()
            }),
            Ok(())
        );
    }

    #[test]
    fn ambiguous_recovery_is_rejected_before_configuration_publication() {
        let mut process = CoreProcess::new();
        process.connect_runtime(runtime_ready(true)).unwrap();
        assert_eq!(
            process.recover(RecoverySnapshot {
                ambiguous_operation_count: 1,
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
            if (kind.mandatory() || kind == CoreHandlerKind::Watches)
                && kind != CoreHandlerKind::Authorization
            {
                process.handlers_mut().update(kind, ready_status()).unwrap();
            }
        }
        assert_eq!(
            process.publish_readiness(),
            Err(StartupError::MandatoryHandlerNotReady)
        );
    }

    #[test]
    fn production_startup_waits_for_real_handler_admission() {
        let mut process = CoreProcess::new();
        assert_eq!(
            process.start_production(runtime_ready(true), recovery()),
            Ok(StartupStage::ReconcilingSystemCore),
        );
        assert_eq!(
            process.publish_readiness(),
            Err(StartupError::MandatoryHandlerNotReady)
        );
        for kind in CoreHandlerKind::ALL {
            if kind.mandatory() || kind == CoreHandlerKind::Watches {
                process.handlers_mut().update(kind, ready_status()).unwrap();
            }
        }
        assert_eq!(process.publish_readiness(), Ok(StartupStage::Ready));
        assert_eq!(process.stage(), StartupStage::Ready);
    }

    #[test]
    fn unregistered_controller_endpoint_blocks_recovery() {
        let mut process = CoreProcess::new();
        assert_eq!(
            process.connect_runtime(RuntimeReadiness {
                controller_endpoint_registered: false,
                ..runtime_ready(true)
            }),
            Err(StartupError::ControllerEndpointUnavailable)
        );
        assert_eq!(process.stage(), StartupStage::WaitingForControllerEndpoint);
    }

    #[test]
    fn watch_must_be_admitted_before_recovery() {
        let mut process = CoreProcess::new();
        process.connect_runtime(runtime_ready(true)).unwrap();
        assert_eq!(
            process.recover(RecoverySnapshot {
                watch_admitted: false,
                ..recovery()
            }),
            Err(StartupError::WatchAdmissionUnavailable)
        );
    }
}
