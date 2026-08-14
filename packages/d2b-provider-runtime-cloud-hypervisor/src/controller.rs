//! Cloud Hypervisor Guest lifecycle controller.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use tokio::time::timeout;

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
    /// Launch or initial guest-control probing exceeded the startup deadline.
    StartupDeadlineExceeded,
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
            Self::StartupDeadlineExceeded => "cloud-hypervisor-startup-deadline-exceeded",
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
    expected_identity: Option<ProcessIdentity>,
    health_failures: u8,
    finalizer: bool,
    adoption_started: Instant,
    startup_started: Option<Instant>,
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
            expected_identity: None,
            health_failures: 0,
            finalizer: true,
            adoption_started: Instant::now(),
            startup_started: None,
        })
    }

    /// Bind the controller to the durable process identity used for restart
    /// adoption.
    pub fn with_expected_identity(mut self, expected: ProcessIdentity) -> Self {
        self.expected_identity = Some(expected);
        self
    }

    /// Return the current phase.
    pub const fn phase(&self) -> CloudHypervisorPhase {
        self.phase
    }

    /// Return whether the finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    fn apply_health(
        &mut self,
        health: GuestControlHealth,
    ) -> Result<CloudHypervisorReconcileOutcome, CloudHypervisorError> {
        match health {
            GuestControlHealth::Ready => {
                self.health_failures = 0;
                self.phase = CloudHypervisorPhase::Ready;
                self.startup_started = None;
                Ok(CloudHypervisorReconcileOutcome::Converged)
            }
            GuestControlHealth::Degraded => {
                self.health_failures = self.health_failures.saturating_add(1);
                if self.health_failures >= self.config.health_check_failure_threshold {
                    self.phase = CloudHypervisorPhase::Degraded;
                }
                Ok(CloudHypervisorReconcileOutcome::Retry {
                    after_ms: self.config.health_check_interval_ms,
                })
            }
            GuestControlHealth::Failed => {
                self.health_failures = self.config.health_check_failure_threshold;
                self.phase = CloudHypervisorPhase::Failed;
                Err(CloudHypervisorError::GuestControl(
                    GuestControlHealthError::AuthenticationFailed,
                ))
            }
        }
    }

    fn startup_remaining(&mut self) -> Result<Duration, CloudHypervisorError> {
        let started = *self.startup_started.get_or_insert_with(Instant::now);
        Duration::from_millis(u64::from(self.config.startup_deadline_ms))
            .checked_sub(started.elapsed())
            .ok_or(CloudHypervisorError::StartupDeadlineExceeded)
    }

    fn startup_timeout(&mut self) -> CloudHypervisorError {
        self.phase = CloudHypervisorPhase::Failed;
        CloudHypervisorError::StartupDeadlineExceeded
    }

    fn startup_budget(&mut self) -> Result<Duration, CloudHypervisorError> {
        match self.startup_remaining() {
            Ok(remaining) => Ok(remaining),
            Err(_) => Err(self.startup_timeout()),
        }
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
                if self.adoption_started.elapsed()
                    > Duration::from_millis(u64::from(self.config.adoption_window_ms))
                {
                    self.phase = CloudHypervisorPhase::Degraded;
                    return Err(CloudHypervisorError::AdoptionAmbiguous);
                }
                let Some(expected) = self.expected_identity else {
                    self.phase = CloudHypervisorPhase::Degraded;
                    return Err(CloudHypervisorError::AdoptionAmbiguous);
                };
                match verify_identity(&expected, &candidate) {
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
                let identity = match timeout(
                    self.startup_budget()?,
                    self.effect
                        .launch(&self.graph, &self.config, &self.settings),
                )
                .await
                {
                    Ok(result) => result?,
                    Err(_) => return Err(self.startup_timeout()),
                };
                self.expected_identity = Some(identity);
                self.identity = Some(identity);
                self.phase = CloudHypervisorPhase::VmmReady;
            }
        }
        if self.phase != CloudHypervisorPhase::Ready {
            self.phase = CloudHypervisorPhase::Bootstrapping;
        }
        let probe_timeout = if self.phase == CloudHypervisorPhase::Bootstrapping {
            self.startup_budget()?
        } else {
            Duration::from_millis(u64::from(self.config.health_check_timeout_ms))
        };
        let health = match timeout(
            probe_timeout,
            self.probe
                .probe(expected_cid, self.config.health_check_timeout_ms),
        )
        .await
        {
            Ok(result) => result.map_err(CloudHypervisorError::GuestControl)?,
            Err(_) if self.phase == CloudHypervisorPhase::Bootstrapping => {
                return Err(self.startup_timeout());
            }
            Err(_) => {
                return Err(CloudHypervisorError::GuestControl(
                    GuestControlHealthError::Timeout,
                ));
            }
        };
        self.apply_health(health)
    }

    /// Adopt a process after the caller has rehydrated the expected identity
    /// from the durable Process record.
    pub async fn adopt(
        &mut self,
        expected: ProcessIdentity,
        expected_cid: u32,
    ) -> Result<CloudHypervisorReconcileOutcome, CloudHypervisorError> {
        if !self.finalizer {
            return Err(CloudHypervisorError::InvalidState);
        }
        if self.adoption_started.elapsed()
            > Duration::from_millis(u64::from(self.config.adoption_window_ms))
        {
            self.phase = CloudHypervisorPhase::Degraded;
            return Err(CloudHypervisorError::AdoptionAmbiguous);
        }
        let Some(candidate) = self.effect.observe().await? else {
            self.phase = CloudHypervisorPhase::Degraded;
            return Ok(CloudHypervisorReconcileOutcome::Retry { after_ms: 1_000 });
        };
        if verify_identity(&expected, &candidate) != AdoptionOutcome::Adopted {
            self.phase = CloudHypervisorPhase::Degraded;
            return Err(CloudHypervisorError::AdoptionAmbiguous);
        }
        self.expected_identity = Some(expected);
        self.effect.open_pidfd(&candidate).await?;
        self.identity = Some(candidate);
        self.phase = CloudHypervisorPhase::VmmReady;
        let health = match timeout(
            self.startup_budget()?,
            self.probe
                .probe(expected_cid, self.config.health_check_timeout_ms),
        )
        .await
        {
            Ok(result) => result.map_err(CloudHypervisorError::GuestControl)?,
            Err(_) => return Err(self.startup_timeout()),
        };
        self.apply_health(health)
    }

    /// Stop guest-control first, then the VMM process.
    pub async fn finalize(&mut self) -> Result<(), CloudHypervisorError> {
        if !self.finalizer {
            return Ok(());
        }
        self.phase = CloudHypervisorPhase::Finalizing;
        let Some(candidate) = self.effect.observe().await? else {
            self.finalizer = false;
            self.phase = CloudHypervisorPhase::Finalized;
            return Ok(());
        };
        let Some(expected) = self.expected_identity else {
            self.phase = CloudHypervisorPhase::Degraded;
            return Err(CloudHypervisorError::AdoptionAmbiguous);
        };
        if verify_identity(&expected, &candidate) != AdoptionOutcome::Adopted {
            self.phase = CloudHypervisorPhase::Degraded;
            return Err(CloudHypervisorError::AdoptionAmbiguous);
        }
        self.effect.open_pidfd(&candidate).await?;
        self.identity = Some(candidate);
        self.effect.stop(&candidate).await?;
        self.identity = None;
        self.finalizer = false;
        self.phase = CloudHypervisorPhase::Finalized;
        Ok(())
    }
}
