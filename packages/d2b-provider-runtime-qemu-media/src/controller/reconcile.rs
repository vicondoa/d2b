//! QEMU media Guest lifecycle controller.

use crate::{
    adoption::{AdoptionOutcome, ProcessIdentity, verify_identity},
    config::{ProviderConfig, ProviderConfigError},
    controller::{
        DeviceAdmission, DeviceAdmissionError, DeviceObservation, LaunchTicket, ProcessSpec,
        ProcessSpecError,
    },
    qmp::QmpVmStatus,
    types::{GuestProviderSpecSettings, GuestSpecError},
};
use d2b_contracts::v3::ResourceRef;
use std::marker::PhantomData;

/// QEMU media lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuMediaPhase {
    /// Dependencies are pending.
    Pending,
    /// The QEMU Process is starting.
    Starting,
    /// Waiting for QMP greeting and capability negotiation.
    WaitingQmp,
    /// QEMU is paused after QMP readiness.
    PausedAtBoot,
    /// QEMU is running.
    Ready,
    /// A retryable observation or cleanup failed.
    Degraded,
    /// The current generation failed closed.
    Failed,
    /// Finalizer cleanup is in progress.
    Finalizing,
    /// Finalizer cleanup completed.
    Finalized,
}

/// Reconcile result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuMediaReconcileOutcome {
    /// Process and device health converged.
    Ready,
    /// Dependencies or health require a retry.
    Retry {
        /// Suggested retry delay in milliseconds.
        after_ms: u32,
    },
    /// The current state was degraded but not terminal.
    Degraded,
}

/// Closed QEMU media controller failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuMediaError {
    /// Provider or Process configuration is invalid.
    InvalidConfiguration,
    /// A dependency is absent or not ready.
    DependencyNotReady,
    /// Device admission failed.
    Device(DeviceAdmissionError),
    /// Process identity was ambiguous.
    AdoptionAmbiguous,
    /// A typed effect failed.
    Effect,
    /// QMP health did not become ready.
    QmpNotReady,
    /// Finalization could not prove process closure.
    FinalizationIncomplete,
    /// The controller was already finalized.
    InvalidState,
}

impl QemuMediaError {
    /// Return the stable Provider error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "runtime-qemu-media-invalid-configuration",
            Self::DependencyNotReady => "dependency-not-ready",
            Self::Device(error) => error.code(),
            Self::AdoptionAmbiguous => "process-adoption-ambiguous",
            Self::Effect => "runtime-qemu-media-effect-failed",
            Self::QmpNotReady => "qmp-greeting-timeout",
            Self::FinalizationIncomplete => "runtime-qemu-media-finalization-incomplete",
            Self::InvalidState => "runtime-qemu-media-invalid-state",
        }
    }
}

impl core::fmt::Display for QemuMediaError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for QemuMediaError {}

impl From<ProviderConfigError> for QemuMediaError {
    fn from(_: ProviderConfigError) -> Self {
        Self::InvalidConfiguration
    }
}

impl From<GuestSpecError> for QemuMediaError {
    fn from(_: GuestSpecError) -> Self {
        Self::InvalidConfiguration
    }
}

impl From<ProcessSpecError> for QemuMediaError {
    fn from(_: ProcessSpecError) -> Self {
        Self::InvalidConfiguration
    }
}

/// Dependency snapshot supplied by Core's authenticated watch path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuMediaDependencies {
    /// KVM Device observation.
    pub device: Option<DeviceObservation>,
    /// Network dependencies are all ready.
    pub network_ready: bool,
    /// Media Volume dependencies are all ready.
    pub media_ready: bool,
    /// Optional display dependency is ready.
    pub display_ready: bool,
    /// QMP Endpoint greeting and capability exchange completed.
    pub qmp_ready: bool,
    /// Current QMP VM state.
    pub qmp_status: Option<QmpVmStatus>,
}

impl Default for QemuMediaDependencies {
    fn default() -> Self {
        Self {
            device: None,
            network_ready: false,
            media_ready: false,
            display_ready: true,
            qmp_ready: false,
            qmp_status: None,
        }
    }
}

impl QemuMediaDependencies {
    /// Construct a fully-ready dependency snapshot.
    pub fn ready(device: DeviceObservation) -> Self {
        Self {
            device: Some(device),
            network_ready: true,
            media_ready: true,
            display_ready: true,
            qmp_ready: true,
            qmp_status: Some(QmpVmStatus::Paused),
        }
    }
}

/// Typed effect boundary owned by Core/ProviderSupervisor.
pub trait QemuMediaEffectPort {
    /// Launch the broker-spawned Process from an opaque LaunchTicket.
    fn launch(&mut self, ticket: &LaunchTicket) -> Result<ProcessIdentity, QemuMediaError>;
    /// Observe an existing candidate without opening a pidfd.
    fn observe(&mut self) -> Result<Option<ProcessIdentity>, QemuMediaError>;
    /// Open a pidfd after identity verification.
    fn open_pidfd(&mut self, identity: &ProcessIdentity) -> Result<(), QemuMediaError>;
    /// Close all QMP/media effects before stopping the Process.
    fn close_media_effects(&mut self) -> Result<(), QemuMediaError> {
        Ok(())
    }
    /// Stop exactly one verified Process.
    fn stop(&mut self, identity: &ProcessIdentity) -> Result<(), QemuMediaError>;
    /// Release the retained Host-global Device authority.
    fn release_device_authority(&mut self) -> Result<(), QemuMediaError> {
        Ok(())
    }
    /// Delete the controller-created runtime Volume after Process exit.
    fn delete_runtime_volume(&mut self) -> Result<(), QemuMediaError> {
        Ok(())
    }
}

/// Durable non-secret recovery state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuMediaRecoveryState {
    /// Current phase.
    pub phase: QemuMediaPhase,
    /// Whether the finalizer remains installed.
    pub finalizer_installed: bool,
    /// Expected process identity, if a prior launch committed it.
    pub expected_identity: Option<ProcessIdentity>,
}

/// QEMU media lifecycle controller.
pub struct QemuMediaController<E> {
    config: ProviderConfig,
    settings: GuestProviderSpecSettings,
    process: ProcessSpec,
    guest_ref: ResourceRef,
    phase: QemuMediaPhase,
    expected_identity: Option<ProcessIdentity>,
    finalizer_installed: bool,
    marker: PhantomData<E>,
}

impl<E> QemuMediaController<E> {
    /// Construct a controller with explicit Provider and Process contracts.
    pub fn new(
        config: ProviderConfig,
        settings: GuestProviderSpecSettings,
        process: ProcessSpec,
        guest_ref: ResourceRef,
    ) -> Result<Self, QemuMediaError> {
        config.validate()?;
        settings.validate()?;
        process.validate()?;
        if guest_ref.resource_type().as_str() != "Guest" {
            return Err(QemuMediaError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            settings,
            process,
            guest_ref,
            phase: QemuMediaPhase::Pending,
            expected_identity: None,
            finalizer_installed: true,
            marker: PhantomData,
        })
    }

    /// Return the current phase.
    pub const fn phase(&self) -> QemuMediaPhase {
        self.phase
    }

    /// Return whether the Guest finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer_installed
    }

    /// Set the durable expected identity used for restart adoption.
    pub fn set_expected_identity(&mut self, identity: ProcessIdentity) {
        self.expected_identity = Some(identity);
    }

    /// Export non-secret restart state.
    pub fn recovery_state(&self) -> QemuMediaRecoveryState {
        QemuMediaRecoveryState {
            phase: self.phase,
            finalizer_installed: self.finalizer_installed,
            expected_identity: self.expected_identity.clone(),
        }
    }

    /// Restore non-secret restart state.
    pub fn restore_recovery_state(
        mut self,
        recovery: QemuMediaRecoveryState,
    ) -> Result<Self, QemuMediaError> {
        if !recovery.finalizer_installed && recovery.phase != QemuMediaPhase::Finalized {
            return Err(QemuMediaError::InvalidConfiguration);
        }
        self.phase = recovery.phase;
        self.finalizer_installed = recovery.finalizer_installed;
        self.expected_identity = recovery.expected_identity;
        Ok(self)
    }

    /// Test-only state setup used by hermetic finalizer tests.
    #[doc(hidden)]
    pub fn mark_ready_for_test(&mut self) {
        self.phase = QemuMediaPhase::Ready;
        self.finalizer_installed = true;
    }
}

impl<E: QemuMediaEffectPort> QemuMediaController<E> {
    /// Reconcile dependencies, process identity, and QMP readiness.
    pub fn reconcile(
        &mut self,
        dependencies: &QemuMediaDependencies,
        effect: &mut E,
    ) -> Result<QemuMediaReconcileOutcome, QemuMediaError> {
        if !self.finalizer_installed {
            return Err(QemuMediaError::InvalidState);
        }
        let Some(device) = dependencies.device.as_ref() else {
            self.phase = QemuMediaPhase::Pending;
            return Ok(QemuMediaReconcileOutcome::Retry { after_ms: 500 });
        };
        if !dependencies.network_ready
            || !dependencies.media_ready
            || (self.settings.display_window && !dependencies.display_ready)
        {
            self.phase = QemuMediaPhase::Pending;
            return Ok(QemuMediaReconcileOutcome::Retry { after_ms: 500 });
        }
        let expected_process = device.process_identity.as_deref().unwrap_or("qemu-media");
        DeviceAdmission::validate(&self.guest_ref, device, expected_process, "qemu-media/v1")
            .map_err(QemuMediaError::Device)?;

        let observed = effect.observe()?;
        let identity = match observed {
            Some(candidate) => {
                let Some(expected) = self.expected_identity.as_ref() else {
                    self.phase = QemuMediaPhase::Degraded;
                    return Err(QemuMediaError::AdoptionAmbiguous);
                };
                if verify_identity(expected, &candidate) != AdoptionOutcome::Adopted {
                    self.phase = QemuMediaPhase::Degraded;
                    return Err(QemuMediaError::AdoptionAmbiguous);
                }
                self.phase = QemuMediaPhase::Starting;
                effect.open_pidfd(&candidate)?;
                candidate
            }
            None => {
                self.phase = QemuMediaPhase::Starting;
                let ticket =
                    LaunchTicket::new(self.process.clone(), Vec::<ResourceRef>::new(), None)?;
                let candidate = effect.launch(&ticket)?;
                self.expected_identity = Some(candidate.clone());
                effect.open_pidfd(&candidate)?;
                candidate
            }
        };

        if !dependencies.qmp_ready {
            self.phase = QemuMediaPhase::WaitingQmp;
            return Ok(QemuMediaReconcileOutcome::Retry { after_ms: 250 });
        }
        self.expected_identity = Some(identity);
        self.phase = if self.settings.pause_at_boot {
            QemuMediaPhase::PausedAtBoot
        } else {
            QemuMediaPhase::Ready
        };
        Ok(QemuMediaReconcileOutcome::Ready)
    }

    /// Finalize QMP/media effects, then stop the Process and release authority.
    pub fn finalize(&mut self, effect: &mut E) -> Result<(), QemuMediaError> {
        if !self.finalizer_installed {
            return Ok(());
        }
        self.phase = QemuMediaPhase::Finalizing;
        effect.close_media_effects()?;
        if let Some(identity) = self.expected_identity.as_ref() {
            effect.stop(identity)?;
            if effect.observe()?.is_some() {
                self.phase = QemuMediaPhase::Degraded;
                return Err(QemuMediaError::FinalizationIncomplete);
            }
        }
        effect.release_device_authority()?;
        effect.delete_runtime_volume()?;
        self.finalizer_installed = false;
        self.phase = QemuMediaPhase::Finalized;
        Ok(())
    }

    /// Borrow the controller's Provider configuration projection.
    pub const fn config(&self) -> &ProviderConfig {
        &self.config
    }
}
