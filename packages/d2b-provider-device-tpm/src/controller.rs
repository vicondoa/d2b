//! TPM Device reconcile state machine.

use core::fmt;

use crate::{
    DEVICE_TPM_FINALIZER,
    runner::{
        FlushLaunchTicket, SignedBinaryRef, SwtpmArgvError, SwtpmSettings, SwtpmStartLaunchTicket,
    },
    state::{StateDirIntent, TpmStateObservation, TpmStateValidationError},
};

/// TPM controller lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmPhase {
    /// No state-directory effect has completed.
    Pending,
    /// State directory is being prepared or checked.
    PreparingState,
    /// The pre-start flush is running.
    Flushing,
    /// swtpm is being started.
    Starting,
    /// The long-lived swtpm process is Ready.
    Ready,
    /// The Device is safe but cannot currently serve a TPM.
    Degraded,
    /// The current generation failed closed.
    Failed,
    /// Finalizer teardown is stopping the worker.
    Finalizing,
    /// Finalizer was cleared without deleting the Volume.
    Finalized,
}

/// Closed Provider effect failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmEffectError {
    /// The state marker or owner identity failed.
    StateIntegrity,
    /// The flush process failed.
    FlushFailed,
    /// The swtpm worker was absent or failed to become Ready.
    SwtpmMissing,
    /// The Core adapter rejected a launch.
    SpawnRejected,
    /// The operation may be retried without losing authority.
    Transient,
}

impl TpmEffectError {
    /// Return the stable Device error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::StateIntegrity => "device-state-integrity-failure",
            Self::FlushFailed => "device-provision-failed",
            Self::SwtpmMissing => "device-worker-failed",
            Self::SpawnRejected => "device-broker-inaccessible",
            Self::Transient => "transient",
        }
    }
}

impl fmt::Display for TpmEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for TpmEffectError {}

/// Core effect port for TPM state hardening and worker launch.
pub trait TpmEffectPort {
    /// Harden/reconcile the state directory and return opaque launch tickets.
    fn prepare_state_dir(
        &mut self,
        intent: &StateDirIntent,
    ) -> Result<TpmStatePreparationResult, TpmEffectError>;
    /// Run the one-shot pre-start flush.
    fn flush(&mut self, ticket: &FlushLaunchTicket) -> Result<(), TpmEffectError>;
    /// Start the long-lived swtpm worker.
    fn start(
        &mut self,
        ticket: &SwtpmStartLaunchTicket,
        settings: SwtpmSettings,
        binary: &SignedBinaryRef,
    ) -> Result<(), TpmEffectError>;
    /// Stop the owned swtpm worker during finalization.
    fn stop(&mut self) -> Result<(), TpmEffectError>;
}

/// Opaque result of state-directory preparation.
#[derive(Clone, PartialEq, Eq)]
pub struct TpmStatePreparationResult {
    /// Validated state observation.
    pub observation: TpmStateObservation,
    /// Pre-start flush ticket.
    pub flush_ticket: FlushLaunchTicket,
    /// Long-lived swtpm ticket.
    pub swtpm_ticket: SwtpmStartLaunchTicket,
}

impl fmt::Debug for TpmStatePreparationResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TpmStatePreparationResult(<redacted>)")
    }
}

/// Controller-level failures, distinct from effect failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmControllerError {
    /// Device settings failed the signed Provider schema bounds.
    Settings(SwtpmArgvError),
    /// The state machine was called in a phase that cannot accept the action.
    InvalidState,
    /// The trusted state observation was rejected.
    StateValidation(TpmStateValidationError),
    /// A Core effect failed.
    Effect(TpmEffectError),
}

impl fmt::Display for TpmControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settings(error) => error.fmt(formatter),
            Self::InvalidState => formatter.write_str("tpm-invalid-state"),
            Self::StateValidation(error) => error.fmt(formatter),
            Self::Effect(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TpmControllerError {}

/// Reconcile result disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmReconcileDisposition {
    /// The Device and worker converged.
    Ready,
    /// The Device should be retried.
    Retry,
    /// The Device failed closed.
    Failed,
}

/// Closed reconcile outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmReconcileOutcome {
    /// State and worker are Ready.
    Converged,
    /// A transient effect should be retried.
    Transient,
    /// The state marker or worker failed.
    Failed,
}

/// TPM Device controller state.
pub struct TpmController {
    intent: StateDirIntent,
    settings: SwtpmSettings,
    binary: SignedBinaryRef,
    phase: TpmPhase,
    finalizer: bool,
    volume_preserved: bool,
    worker_started: bool,
}

impl TpmController {
    /// Construct a controller with the state-preserving finalizer installed.
    pub fn new(
        intent: StateDirIntent,
        settings: SwtpmSettings,
        binary: SignedBinaryRef,
    ) -> Result<Self, TpmControllerError> {
        settings.validate().map_err(TpmControllerError::Settings)?;
        Ok(Self {
            intent,
            settings,
            binary,
            phase: TpmPhase::Pending,
            finalizer: true,
            volume_preserved: true,
            worker_started: false,
        })
    }

    /// Return the current lifecycle phase.
    pub const fn phase(&self) -> TpmPhase {
        self.phase
    }

    /// Return whether the state-preserving finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Return whether the persistent Volume is protected from deletion.
    pub const fn volume_preserved(&self) -> bool {
        self.volume_preserved
    }

    /// Return the fixed finalizer ID.
    pub const fn finalizer_id(&self) -> &'static str {
        DEVICE_TPM_FINALIZER
    }

    /// Reconcile in the required prepare → flush → swtpm order.
    pub fn reconcile<P: TpmEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<TpmReconcileOutcome, TpmControllerError> {
        if !self.finalizer || matches!(self.phase, TpmPhase::Finalizing | TpmPhase::Finalized) {
            return Err(TpmControllerError::InvalidState);
        }
        self.phase = TpmPhase::PreparingState;
        let prepared = match port.prepare_state_dir(&self.intent) {
            Ok(prepared) => prepared,
            Err(error) => return self.effect_failed(error),
        };
        if let Err(error) = self.intent.validate(&prepared.observation) {
            self.phase = TpmPhase::Failed;
            return Err(TpmControllerError::StateValidation(error));
        }
        if self.settings.startup_clear {
            self.phase = TpmPhase::Flushing;
            if let Err(error) = port.flush(&prepared.flush_ticket) {
                return self.effect_failed(error);
            }
        }
        self.phase = TpmPhase::Starting;
        if let Err(error) = port.start(&prepared.swtpm_ticket, self.settings, &self.binary) {
            return self.effect_failed(error);
        }
        self.worker_started = true;
        self.phase = TpmPhase::Ready;
        Ok(TpmReconcileOutcome::Converged)
    }

    /// Stop the worker and clear the finalizer without deleting the Volume.
    pub fn finalize<P: TpmEffectPort>(&mut self, port: &mut P) -> Result<(), TpmControllerError> {
        if !self.finalizer {
            return Ok(());
        }
        self.phase = TpmPhase::Finalizing;
        if self.worker_started {
            port.stop().map_err(|error| {
                self.phase = TpmPhase::Degraded;
                TpmControllerError::Effect(error)
            })?;
        }
        self.worker_started = false;
        self.volume_preserved = true;
        self.finalizer = false;
        self.phase = TpmPhase::Finalized;
        Ok(())
    }

    fn effect_failed<T>(&mut self, error: TpmEffectError) -> Result<T, TpmControllerError> {
        self.phase = if error == TpmEffectError::Transient {
            TpmPhase::Degraded
        } else {
            TpmPhase::Failed
        };
        Err(TpmControllerError::Effect(error))
    }
}

impl fmt::Debug for TpmController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TpmController")
            .field("phase", &self.phase)
            .field("finalizer", &self.finalizer)
            .field("volume_preserved", &self.volume_preserved)
            .field("worker_started", &self.worker_started)
            .finish()
    }
}
