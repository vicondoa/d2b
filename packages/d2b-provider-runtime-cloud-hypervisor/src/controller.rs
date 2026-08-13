//! Cloud Hypervisor Guest lifecycle controller.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    adoption::{AdoptionOutcome, ProcessIdentity, verify_identity},
    bootstrap_graph::{BootstrapGraph, DependencyReadiness},
    config::{CloudHypervisorConfig, CloudHypervisorGuestSettings},
    health::{GuestControlHealth, GuestControlHealthError, GuestControlProbe},
};

/// Cloud Hypervisor lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudHypervisorPhase {
    /// Dependencies are pending.
    Pending,
    /// VMM process is starting.
    Starting,
    /// VMM process is ready.
    VmmReady,
    /// Guest-control is probing.
    Bootstrapping,
    /// Guest and guest-control are ready.
    Ready,
    /// Restart adoption or health is degraded.
    Degraded,
    /// Failed closed.
    Failed,
    /// Finalizing guest-control before process stop.
    Finalizing,
    /// Finalized.
    Finalized,
}

/// Reconcile result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudHypervisorReconcileOutcome {
    /// Guest is ready.
    Converged,
    /// Dependencies or health require a retry.
    Retry {
        /// Delay in milliseconds.
        after_ms: u32,
    },
    /// A VMM effect is progressing.
    Progressing {
        /// Delay in milliseconds.
        after_ms: u32,
    },
}

/// Stable controller errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudHypervisorError {
    /// Configuration or dependency graph is invalid.
    InvalidConfiguration,
    /// Dependencies are not ready.
    DependencyNotReady,
    /// Process identity was ambiguous.
    AdoptionAmbiguous,
    /// Guest-control authentication failed.
    GuestControl(GuestControlHealthError),
    /// VMM effect failed.
    Effect,
    /// Finalization was requested after completion.
    InvalidState,
}

impl std::fmt::Display for CloudHypervisorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfiguration => "cloud-hypervisor-invalid-configuration",
            Self::DependencyNotReady => "dependency-not-ready",
            Self::AdoptionAmbiguous => "process-adoption-ambiguous",
            Self::GuestControl(error) => error.code(),
            Self::Effect => "cloud-hypervisor-effect-failed",
            Self::InvalidState => "cloud-hypervisor-invalid-state",
        })
    }
}

impl std::error::Error for CloudHypervisorError {}

/// Typed VMM effect boundary.
#[async_trait]
pub trait CloudHypervisorEffectPort: Send + Sync {
    /// Launch the broker-spawned VMM Process.
    async fn launch(
        &self,
        graph: &BootstrapGraph,
        config: &CloudHypervisorConfig,
        settings: &CloudHypervisorGuestSettings,
    ) -> Result<ProcessIdentity, CloudHypervisorError>;
    /// Observe a candidate before pidfd adoption.
    async fn observe(&self) -> Result<Option<ProcessIdentity>, CloudHypervisorError>;
    /// Open a pidfd after identity verification.
    async fn open_pidfd(&self, identity: &ProcessIdentity) -> Result<(), CloudHypervisorError>;
    /// Stop exactly one adopted process.
    async fn stop(&self, identity: &ProcessIdentity) -> Result<(), CloudHypervisorError>;
}

/// Cloud Hypervisor controller.
pub struct CloudHypervisorController<E, P> {
    config: CloudHypervisorConfig,
    settings: CloudHypervisorGuestSettings,
    graph: BootstrapGraph,
    effect: Arc<E>,
    probe: Arc<P>,
    phase: CloudHypervisorPhase,
    identity: Option<ProcessIdentity>,
    finalizer: bool,
}

impl<E, P> CloudHypervisorController<E, P>
where
    E: CloudHypervisorEffectPort + 'static,
    P: GuestControlProbe + 'static,
{
    /// Construct a controller with explicit dependency graph and ports.
    pub fn new(
        config: CloudHypervisorConfig,
        settings: CloudHypervisorGuestSettings,
        graph: BootstrapGraph,
        effect: Arc<E>,
        probe: Arc<P>,
    ) -> Result<Self, CloudHypervisorError> {
        config
            .validate()
            .map_err(|_| CloudHypervisorError::InvalidConfiguration)?;
        settings
            .validate()
            .map_err(|_| CloudHypervisorError::InvalidConfiguration)?;
        Ok(Self {
            config,
            settings,
            graph,
            effect,
            probe,
            phase: CloudHypervisorPhase::Pending,
            identity: None,
            finalizer: true,
        })
    }

    /// Return the current phase.
    pub const fn phase(&self) -> CloudHypervisorPhase {
        self.phase
    }

    /// Return whether the finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Reconcile after dependency readiness has been observed.
    pub async fn reconcile(
        &mut self,
        devices_ready: bool,
        networks_ready: bool,
        volumes_ready: bool,
        expected_cid: u32,
    ) -> Result<CloudHypervisorReconcileOutcome, CloudHypervisorError> {
        if !self.finalizer {
            return Err(CloudHypervisorError::InvalidState);
        }
        if self
            .graph
            .readiness(devices_ready, networks_ready, volumes_ready)
            != DependencyReadiness::Ready
        {
            self.phase = CloudHypervisorPhase::Pending;
            return Ok(CloudHypervisorReconcileOutcome::Retry { after_ms: 500 });
        }
        if self.identity.is_none() {
            if let Some(candidate) = self.effect.observe().await? {
                match verify_identity(&candidate, &candidate) {
                    AdoptionOutcome::Adopted => {
                        self.effect.open_pidfd(&candidate).await?;
                        self.identity = Some(candidate);
                        self.phase = CloudHypervisorPhase::VmmReady;
                    }
                    AdoptionOutcome::Quarantined | AdoptionOutcome::Absent => {
                        self.phase = CloudHypervisorPhase::Degraded;
                        return Err(CloudHypervisorError::AdoptionAmbiguous);
                    }
                }
            } else {
                self.phase = CloudHypervisorPhase::Starting;
                self.identity = Some(
                    self.effect
                        .launch(&self.graph, &self.config, &self.settings)
                        .await?,
                );
                self.phase = CloudHypervisorPhase::VmmReady;
            }
        }
        self.phase = CloudHypervisorPhase::Bootstrapping;
        match self
            .probe
            .probe(expected_cid, self.config.health_check_timeout_ms)
            .await
            .map_err(CloudHypervisorError::GuestControl)?
        {
            GuestControlHealth::Ready => {
                self.phase = CloudHypervisorPhase::Ready;
                Ok(CloudHypervisorReconcileOutcome::Converged)
            }
            GuestControlHealth::Degraded => {
                self.phase = CloudHypervisorPhase::Degraded;
                Ok(CloudHypervisorReconcileOutcome::Retry {
                    after_ms: self.config.health_check_interval_ms,
                })
            }
            GuestControlHealth::Failed => {
                self.phase = CloudHypervisorPhase::Failed;
                Err(CloudHypervisorError::GuestControl(
                    GuestControlHealthError::AuthenticationFailed,
                ))
            }
        }
    }

    /// Adopt a process after the caller has rehydrated the expected identity
    /// from the durable Process record.
    pub async fn adopt(
        &mut self,
        expected: ProcessIdentity,
        expected_cid: u32,
    ) -> Result<CloudHypervisorReconcileOutcome, CloudHypervisorError> {
        let Some(candidate) = self.effect.observe().await? else {
            self.phase = CloudHypervisorPhase::Degraded;
            return Ok(CloudHypervisorReconcileOutcome::Retry { after_ms: 1_000 });
        };
        if verify_identity(&expected, &candidate) != AdoptionOutcome::Adopted {
            self.phase = CloudHypervisorPhase::Degraded;
            return Err(CloudHypervisorError::AdoptionAmbiguous);
        }
        self.effect.open_pidfd(&candidate).await?;
        self.identity = Some(candidate);
        self.phase = CloudHypervisorPhase::VmmReady;
        match self
            .probe
            .probe(expected_cid, self.config.health_check_timeout_ms)
            .await
            .map_err(CloudHypervisorError::GuestControl)?
        {
            GuestControlHealth::Ready => {
                self.phase = CloudHypervisorPhase::Ready;
                Ok(CloudHypervisorReconcileOutcome::Converged)
            }
            GuestControlHealth::Degraded => {
                self.phase = CloudHypervisorPhase::Degraded;
                Ok(CloudHypervisorReconcileOutcome::Retry {
                    after_ms: self.config.health_check_interval_ms,
                })
            }
            GuestControlHealth::Failed => {
                self.phase = CloudHypervisorPhase::Failed;
                Err(CloudHypervisorError::GuestControl(
                    GuestControlHealthError::AuthenticationFailed,
                ))
            }
        }
    }

    /// Stop guest-control first, then the VMM process.
    pub async fn finalize(&mut self) -> Result<(), CloudHypervisorError> {
        if !self.finalizer {
            return Ok(());
        }
        self.phase = CloudHypervisorPhase::Finalizing;
        if let Some(identity) = self.identity.take() {
            self.effect.stop(&identity).await?;
        }
        self.finalizer = false;
        self.phase = CloudHypervisorPhase::Finalized;
        Ok(())
    }
}
