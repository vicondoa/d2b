//! Systemd Process Provider reconcile adapter.

use std::sync::Arc;

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
    launch_slots: Arc<tokio::sync::Semaphore>,
}

impl<P: d2b_process_conformance::ProcessLaunchEffectPort> SystemdProcessController<P> {
    /// Construct a controller over the core-owned effect port.
    pub fn new(provider: SystemdProcessProvider<P>, config: SystemdProviderConfig) -> Self {
        Self {
            launch_slots: Arc::new(tokio::sync::Semaphore::new(
                config.max_concurrent_launches as usize,
            )),
            provider,
            config,
        }
    }

    /// Borrow bounded controller config.
    pub const fn config(&self) -> SystemdProviderConfig {
        self.config
    }

    /// Borrow the wrapped process Provider for status and test inspection.
    pub const fn provider(&self) -> &SystemdProcessProvider<P> {
        &self.provider
    }

    /// Reconcile one action without opening a systemd connection.
    pub async fn reconcile(
        &self,
        action: SystemdReconcileAction<'_>,
    ) -> Result<SystemdReconcileResult, ProcessConformanceError> {
        let timeout = match action {
            SystemdReconcileAction::Start(_) => self.config.launch_timeout_sec,
            SystemdReconcileAction::Adopt(_) => self.config.user_manager_check_timeout,
            SystemdReconcileAction::Stop(_, _) => self.config.termination_grace_sec.max(1),
        };
        let permit = Arc::clone(&self.launch_slots)
            .try_acquire_owned()
            .map_err(|_| ProcessConformanceError::DeadlineExceeded)?;
        let operation = async {
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
        };
        let result = if tokio::runtime::Handle::try_current().is_ok() {
            tokio::time::timeout(
                std::time::Duration::from_secs(u64::from(timeout)),
                operation,
            )
            .await
            .map_err(|_| ProcessConformanceError::DeadlineExceeded)?
        } else {
            operation.await
        };
        drop(permit);
        result
    }
}
