//! Systemd Process Provider reconcile adapter.

use d2b_process_conformance::{
    AdoptionOutcome, LaunchTicket, ProcessConformanceError, ProcessIdentityDigest, ProcessProvider,
    ProcessStatusReport, StopClass,
};

use crate::{SystemdProcessProvider, lifecycle::SystemdProviderConfig};

/// One typed reconcile request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemdReconcileAction<'a> {
    /// Start a new transient unit through the effect port.
    Start(&'a LaunchTicket),
    /// Adopt an already running transient unit after identity verification.
    Adopt(&'a LaunchTicket),
    /// Stop an exact identity through the effect port.
    Stop(&'a ProcessIdentityDigest, StopClass),
}

/// Typed reconcile result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemdReconcileResult {
    /// Process was launched.
    Started(ProcessStatusReport),
    /// Process adoption was evaluated.
    Adoption(AdoptionOutcome),
    /// Process stop was accepted.
    Stopped,
}

/// Controller wrapper retaining only bounded config and the injected port.
#[derive(Debug)]
pub struct SystemdProcessController<P: d2b_process_conformance::ProcessLaunchEffectPort> {
    provider: SystemdProcessProvider<P>,
    config: SystemdProviderConfig,
}

impl<P: d2b_process_conformance::ProcessLaunchEffectPort> SystemdProcessController<P> {
    /// Construct a controller over the core-owned effect port.
    pub fn new(provider: SystemdProcessProvider<P>, config: SystemdProviderConfig) -> Self {
        Self { provider, config }
    }

    /// Borrow bounded controller config.
    pub const fn config(&self) -> SystemdProviderConfig {
        self.config
    }

    /// Reconcile one action without opening a systemd connection.
    pub async fn reconcile(
        &self,
        action: SystemdReconcileAction<'_>,
    ) -> Result<SystemdReconcileResult, ProcessConformanceError> {
        match action {
            SystemdReconcileAction::Start(ticket) => self
                .provider
                .launch(ticket)
                .await
                .map(SystemdReconcileResult::Started),
            SystemdReconcileAction::Adopt(ticket) => self
                .provider
                .adopt(ticket)
                .await
                .map(SystemdReconcileResult::Adoption),
            SystemdReconcileAction::Stop(identity, class) => self
                .provider
                .stop(identity, class)
                .await
                .map(|_| SystemdReconcileResult::Stopped),
        }
    }
}
