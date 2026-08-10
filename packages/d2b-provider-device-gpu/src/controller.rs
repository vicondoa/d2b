//! Combined GPU/video Device reconcile state machine.

use core::fmt;
use d2b_contracts::v3::{ResourceUid, device::DeviceArbitration};

use crate::{
    GpuEffectError, GpuEffectPort, GpuEffectTokenSet, GpuProcessRole, GpuProcessSelectionError,
    GpuSettings, process::select_processes,
};

/// GPU controller lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPhase {
    /// No worker effects have started.
    Pending,
    /// The GPU/render-node worker is starting.
    GpuStarting,
    /// The GPU/render-node worker is Ready.
    GpuReady,
    /// The video worker is starting after GPU readiness.
    VideoStarting,
    /// All requested workers are Ready.
    Ready,
    /// A worker can be retried.
    Degraded,
    /// The generation failed closed.
    Failed,
    /// Finalizer is stopping workers.
    Finalizing,
    /// Finalizer cleared.
    Finalized,
}

/// GPU controller failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuControllerError {
    /// Settings or process selection violated the Device contract.
    Selection(GpuProcessSelectionError),
    /// Core effect failed.
    Effect(GpuEffectError),
    /// A finalizer transition was invalid.
    InvalidState,
}

impl fmt::Display for GpuControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Selection(error) => return error.fmt(formatter),
            Self::Effect(error) => return error.fmt(formatter),
            Self::InvalidState => "gpu-invalid-state",
        })
    }
}

impl std::error::Error for GpuControllerError {}

/// Closed reconcile outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuReconcileOutcome {
    /// GPU and optional video workers converged.
    Converged,
    /// A transient effect should be retried.
    Retry,
}

/// Combined GPU/video controller.
pub struct GpuController {
    device_uid: ResourceUid,
    arbitration: DeviceArbitration,
    settings: GpuSettings,
    tokens: GpuEffectTokenSet,
    phase: GpuPhase,
    finalizer: bool,
    gpu_role: Option<GpuProcessRole>,
    video_started: bool,
}

impl GpuController {
    /// Construct a controller with a Core-resolved token set.
    pub fn new(
        device_uid: ResourceUid,
        arbitration: DeviceArbitration,
        settings: GpuSettings,
        tokens: GpuEffectTokenSet,
    ) -> Result<Self, GpuControllerError> {
        select_processes(&device_uid, arbitration, &settings)
            .map_err(GpuControllerError::Selection)?;
        Ok(Self {
            device_uid,
            arbitration,
            settings,
            tokens,
            phase: GpuPhase::Pending,
            finalizer: true,
            gpu_role: None,
            video_started: false,
        })
    }

    /// Return the current controller phase.
    pub const fn phase(&self) -> GpuPhase {
        self.phase
    }

    /// Return whether the Provider finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Start the GPU worker and only then the optional video worker.
    pub fn reconcile<P: GpuEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<GpuReconcileOutcome, GpuControllerError> {
        if !self.finalizer || matches!(self.phase, GpuPhase::Finalizing | GpuPhase::Finalized) {
            return Err(GpuControllerError::InvalidState);
        }
        let declarations = select_processes(&self.device_uid, self.arbitration, &self.settings)
            .map_err(GpuControllerError::Selection)?;
        let gpu = declarations
            .first()
            .ok_or(GpuControllerError::InvalidState)?
            .role();
        self.phase = GpuPhase::GpuStarting;
        let ticket = match port.open_devices(&self.device_uid, &self.tokens) {
            Ok(ticket) => ticket,
            Err(error) => return self.effect_failed(error),
        };
        if let Err(error) = port.start(gpu, &ticket) {
            return self.effect_failed(error);
        }
        self.gpu_role = Some(gpu);
        self.phase = GpuPhase::GpuReady;
        if self.settings.video_sidecar {
            self.phase = GpuPhase::VideoStarting;
            if let Err(error) = port.start(GpuProcessRole::Video, &ticket) {
                return self.effect_failed(error);
            }
            self.video_started = true;
        }
        self.phase = GpuPhase::Ready;
        Ok(GpuReconcileOutcome::Converged)
    }

    /// Stop video first and the GPU/render-node worker second.
    pub fn finalize<P: GpuEffectPort>(&mut self, port: &mut P) -> Result<(), GpuControllerError> {
        if !self.finalizer {
            return Ok(());
        }
        self.phase = GpuPhase::Finalizing;
        if self.video_started {
            port.stop(GpuProcessRole::Video)
                .map_err(GpuControllerError::Effect)?;
            self.video_started = false;
        }
        if let Some(role) = self.gpu_role.take() {
            port.stop(role).map_err(GpuControllerError::Effect)?;
        }
        self.finalizer = false;
        self.phase = GpuPhase::Finalized;
        Ok(())
    }

    fn effect_failed<T>(&mut self, error: GpuEffectError) -> Result<T, GpuControllerError> {
        self.phase = if error == GpuEffectError::Transient {
            GpuPhase::Degraded
        } else {
            GpuPhase::Failed
        };
        Err(GpuControllerError::Effect(error))
    }
}

impl fmt::Debug for GpuController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuController")
            .field("device_uid", &"<redacted>")
            .field("arbitration", &self.arbitration)
            .field("phase", &self.phase)
            .field("finalizer", &self.finalizer)
            .field("gpu_role", &self.gpu_role)
            .field("video_started", &self.video_started)
            .finish()
    }
}
