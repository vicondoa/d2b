//! Combined GPU/video Device reconcile state machine.

use core::fmt;
use d2b_contracts_resource::v3::{ResourceUid, device::DeviceArbitration};

use crate::{
    GpuAuthorityAdmission, GpuAuthorityError, GpuAuthorityLease, GpuClosureProof, GpuEffectError,
    GpuEffectPort, GpuEffectTokenSet, GpuLaunchTicket, GpuLifecycleEffectPort, GpuProcessIdentity,
    GpuProcessObservation, GpuProcessRole, GpuProcessSelectionError, GpuSettings, GpuWorkerSpec,
    VideoWorkerSpec, process::select_processes,
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
    /// Restart identity was ambiguous and is quarantined.
    Quarantined,
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
    /// Core authority admission failed before an effect.
    Authority(GpuAuthorityError),
    /// Restart observation was ambiguous.
    Quarantined,
}

impl fmt::Display for GpuControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Selection(error) => return error.fmt(formatter),
            Self::Effect(error) => return error.fmt(formatter),
            Self::InvalidState => "gpu-invalid-state",
            Self::Authority(error) => return error.fmt(formatter),
            Self::Quarantined => "gpu-authority-quarantined",
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
    admission: Option<GpuAuthorityAdmission>,
    authority_lease: Option<GpuAuthorityLease>,
    ticket: Option<GpuLaunchTicket>,
    gpu_identity: Option<GpuProcessIdentity>,
    video_identity: Option<GpuProcessIdentity>,
    gpu_closure: Option<GpuClosureProof>,
    video_closure: Option<GpuClosureProof>,
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
            admission: None,
            authority_lease: None,
            ticket: None,
            gpu_identity: None,
            video_identity: None,
            gpu_closure: None,
            video_closure: None,
        })
    }

    /// Construct an authority-bound controller from Core admission evidence.
    pub fn new_authorized(
        admission: GpuAuthorityAdmission,
        settings: GpuSettings,
        tokens: GpuEffectTokenSet,
    ) -> Result<Self, GpuControllerError> {
        let device_uid = admission.owner().device_uid().clone();
        let mut controller = Self::new(device_uid, admission.arbitration(), settings, tokens)?;
        controller.admission = Some(admission);
        Ok(controller)
    }

    /// Return the current controller phase.
    pub const fn phase(&self) -> GpuPhase {
        self.phase
    }

    /// Return whether the Provider finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Whether this controller owns a Core admission.
    pub const fn authority_reserved(&self) -> bool {
        self.authority_lease.is_some()
    }

    /// Return the current GPU process identity, if started or adopted.
    pub const fn gpu_identity(&self) -> Option<&GpuProcessIdentity> {
        self.gpu_identity.as_ref()
    }

    /// Return the current video process identity, if started or adopted.
    pub const fn video_identity(&self) -> Option<&GpuProcessIdentity> {
        self.video_identity.as_ref()
    }

    /// Start the GPU worker and only then the optional video worker.
    pub fn reconcile<P: GpuEffectPort>(
        &mut self,
        _port: &mut P,
    ) -> Result<GpuReconcileOutcome, GpuControllerError> {
        if self.admission.is_none() || self.authority_lease.is_none() {
            return Err(GpuControllerError::Authority(
                GpuAuthorityError::StartupRehydrationRequired,
            ));
        }
        if !self.finalizer || matches!(self.phase, GpuPhase::Finalizing | GpuPhase::Finalized) {
            return Err(GpuControllerError::InvalidState);
        }
        Err(GpuControllerError::InvalidState)
    }

    /// Stop video first and the GPU/render-node worker second.
    pub fn finalize<P: GpuEffectPort>(&mut self, port: &mut P) -> Result<(), GpuControllerError> {
        if !self.finalizer {
            return Ok(());
        }
        let _ = port;
        Err(GpuControllerError::InvalidState)
    }

    /// Reconcile through the authority-aware production effect boundary.
    ///
    /// The Host-global reservation is acquired before the first open or
    /// spawn and remains retained until [`Self::finalize_lifecycle`] confirms
    /// every worker closure.
    pub fn reconcile_lifecycle<P: GpuLifecycleEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<GpuReconcileOutcome, GpuControllerError> {
        if !self.finalizer
            || matches!(
                self.phase,
                GpuPhase::Failed
                    | GpuPhase::Finalizing
                    | GpuPhase::Finalized
                    | GpuPhase::Quarantined
            )
        {
            return Err(GpuControllerError::InvalidState);
        }
        let admission = self
            .admission
            .as_ref()
            .ok_or(GpuControllerError::InvalidState)?;
        if self.settings.video_sidecar && admission.video_principal().is_none() {
            return Err(GpuControllerError::Authority(
                GpuAuthorityError::PrincipalNotSeparated,
            ));
        }
        if self.authority_lease.is_none() {
            self.authority_lease = Some(
                port.reserve_authority(admission)
                    .map_err(GpuControllerError::Effect)?,
            );
        }
        if self.phase == GpuPhase::Ready {
            return Ok(GpuReconcileOutcome::Converged);
        }
        if self.ticket.is_none() {
            self.ticket = Some(
                port.open_authorized_devices(admission, &self.tokens)
                    .map_err(GpuControllerError::Effect)?,
            );
        }
        let ticket = self
            .ticket
            .as_ref()
            .ok_or(GpuControllerError::InvalidState)?;
        let generation = admission.owner().generation();
        if self.gpu_identity.is_none() {
            let spec = GpuWorkerSpec::gpu(&self.device_uid, &self.settings)
                .map_err(GpuControllerError::Selection)?;
            let identity = port
                .start_gpu_worker(
                    &spec,
                    ticket,
                    admission.gpu_principal(),
                    admission.platform(),
                    generation,
                )
                .map_err(GpuControllerError::Effect)?;
            self.gpu_role = Some(spec.process().role());
            self.gpu_identity = Some(identity.clone());
            if let Err(error) = validate_started_identity(
                &identity,
                spec.process().role(),
                admission.gpu_principal(),
                admission.platform(),
                generation,
            ) {
                self.phase = GpuPhase::Failed;
                return Err(GpuControllerError::Effect(error));
            }
        }
        self.phase = GpuPhase::GpuReady;
        if self.settings.video_sidecar && self.video_identity.is_none() {
            let principal = admission
                .video_principal()
                .ok_or(GpuControllerError::Authority(
                    GpuAuthorityError::PrincipalNotSeparated,
                ))?;
            let spec = VideoWorkerSpec::new(&self.device_uid, &self.settings)
                .map_err(GpuControllerError::Selection)?;
            let identity = port
                .start_video_worker(&spec, ticket, principal, admission.platform(), generation)
                .map_err(GpuControllerError::Effect)?;
            self.video_identity = Some(identity.clone());
            self.video_started = true;
            if let Err(error) = validate_started_identity(
                &identity,
                GpuProcessRole::Video,
                principal,
                admission.platform(),
                generation,
            ) {
                self.phase = GpuPhase::Failed;
                return Err(GpuControllerError::Effect(error));
            }
        }
        self.phase = GpuPhase::Ready;
        Ok(GpuReconcileOutcome::Converged)
    }

    /// Adopt matching GPU/video workers after a daemon restart.
    pub fn adopt_lifecycle<P: GpuLifecycleEffectPort>(
        &mut self,
        lease: GpuAuthorityLease,
        expected: &[GpuProcessIdentity],
        port: &mut P,
    ) -> Result<GpuReconcileOutcome, GpuControllerError> {
        if !self.finalizer
            || matches!(
                self.phase,
                GpuPhase::Failed
                    | GpuPhase::Finalizing
                    | GpuPhase::Finalized
                    | GpuPhase::Quarantined
            )
        {
            return Err(GpuControllerError::InvalidState);
        }
        let admission = self
            .admission
            .as_ref()
            .ok_or(GpuControllerError::InvalidState)?;
        if self.settings.video_sidecar && admission.video_principal().is_none() {
            return Err(GpuControllerError::Authority(
                GpuAuthorityError::PrincipalNotSeparated,
            ));
        }
        self.authority_lease = Some(lease);
        let mut matched = Vec::new();
        let mut missing = false;
        for identity in expected {
            match port
                .observe_worker(identity)
                .map_err(GpuControllerError::Effect)?
            {
                GpuProcessObservation::Matching(observed) => {
                    if observed != *identity {
                        self.phase = GpuPhase::Quarantined;
                        return Err(GpuControllerError::Quarantined);
                    }
                    matched.push(observed);
                }
                GpuProcessObservation::Ambiguous => {
                    self.phase = GpuPhase::Quarantined;
                    return Err(GpuControllerError::Quarantined);
                }
                GpuProcessObservation::StaleIdentity => {
                    self.phase = GpuPhase::Failed;
                    return Err(GpuControllerError::Effect(
                        GpuEffectError::StaleDeviceIdentity,
                    ));
                }
                GpuProcessObservation::Missing => {
                    missing = true;
                }
            }
        }
        for identity in matched {
            let expected_role = if identity.role() == GpuProcessRole::Video {
                GpuProcessRole::Video
            } else if self.settings.render_node_only {
                GpuProcessRole::RenderNode
            } else {
                GpuProcessRole::FullGpu
            };
            let expected_principal = match identity.role() {
                GpuProcessRole::Video => admission.video_principal(),
                GpuProcessRole::FullGpu | GpuProcessRole::RenderNode => {
                    Some(admission.gpu_principal())
                }
            };
            let Some(expected_principal) = expected_principal else {
                self.phase = GpuPhase::Failed;
                return Err(GpuControllerError::Effect(GpuEffectError::WrongPrincipal));
            };
            if identity.role() != expected_role
                || identity.principal() != expected_principal
                || (identity.role() == GpuProcessRole::Video && !self.settings.video_sidecar)
            {
                self.phase = GpuPhase::Failed;
                return Err(GpuControllerError::Effect(GpuEffectError::WrongPrincipal));
            }
            if identity.platform() != admission.platform() {
                self.phase = GpuPhase::Failed;
                return Err(GpuControllerError::Effect(GpuEffectError::PlatformMismatch));
            }
            if identity.generation() != admission.owner().generation() {
                self.phase = GpuPhase::Failed;
                return Err(GpuControllerError::Effect(
                    GpuEffectError::StaleDeviceIdentity,
                ));
            }
            match identity.role() {
                GpuProcessRole::Video => {
                    if self.video_identity.is_some() {
                        self.phase = GpuPhase::Quarantined;
                        return Err(GpuControllerError::Quarantined);
                    }
                    self.video_started = true;
                    self.video_identity = Some(identity);
                }
                role => {
                    if self.gpu_identity.is_some() {
                        self.phase = GpuPhase::Quarantined;
                        return Err(GpuControllerError::Quarantined);
                    }
                    self.gpu_role = Some(role);
                    self.gpu_identity = Some(identity);
                }
            }
        }
        if missing
            || self.gpu_identity.is_none()
            || (self.settings.video_sidecar && self.video_identity.is_none())
        {
            self.phase = GpuPhase::Pending;
            return Ok(GpuReconcileOutcome::Retry);
        }
        self.phase = GpuPhase::Ready;
        Ok(GpuReconcileOutcome::Converged)
    }

    /// Close workers and release Host-global authority after exact proofs.
    pub fn finalize_lifecycle<P: GpuLifecycleEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), GpuControllerError> {
        if !self.finalizer {
            return Ok(());
        }
        self.phase = GpuPhase::Finalizing;
        if self.video_closure.is_none()
            && let Some(identity) = self.video_identity.as_ref()
        {
            let closure = port
                .stop_worker(identity)
                .map_err(GpuControllerError::Effect)?;
            if closure.identity() != identity {
                self.phase = GpuPhase::Failed;
                return Err(GpuControllerError::Effect(GpuEffectError::CloseUnconfirmed));
            }
            self.video_closure = Some(closure);
        }
        if self.gpu_closure.is_none()
            && let Some(identity) = self.gpu_identity.as_ref()
        {
            let closure = port
                .stop_worker(identity)
                .map_err(GpuControllerError::Effect)?;
            if closure.identity() != identity {
                self.phase = GpuPhase::Failed;
                return Err(GpuControllerError::Effect(GpuEffectError::CloseUnconfirmed));
            }
            self.gpu_closure = Some(closure);
        }
        let closures = self
            .video_closure
            .iter()
            .chain(self.gpu_closure.iter())
            .cloned()
            .collect::<Vec<_>>();
        if let Some(lease) = self.authority_lease.take()
            && let Err(error) = port.release_authority(lease.clone(), &closures)
        {
            self.authority_lease = Some(lease);
            return Err(GpuControllerError::Effect(error));
        }
        self.video_identity = None;
        self.gpu_identity = None;
        self.ticket = None;
        self.gpu_role = None;
        self.video_started = false;
        self.gpu_closure = None;
        self.video_closure = None;
        self.finalizer = false;
        self.phase = GpuPhase::Finalized;
        Ok(())
    }
}

fn validate_started_identity(
    identity: &GpuProcessIdentity,
    expected_role: GpuProcessRole,
    expected_principal: &crate::GpuPrincipalToken,
    expected_platform: &crate::GpuPlatformToken,
    expected_generation: d2b_contracts_resource::v3::ResourceGeneration,
) -> Result<(), GpuEffectError> {
    if identity.role() != expected_role || identity.principal() != expected_principal {
        return Err(GpuEffectError::WrongPrincipal);
    }
    if identity.platform() != expected_platform {
        return Err(GpuEffectError::PlatformMismatch);
    }
    if identity.generation() != expected_generation {
        return Err(GpuEffectError::StaleDeviceIdentity);
    }
    Ok(())
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
            .field("has_authority", &self.authority_lease.is_some())
            .field("has_gpu_identity", &self.gpu_identity.is_some())
            .field("has_video_identity", &self.video_identity.is_some())
            .field("has_gpu_closure", &self.gpu_closure.is_some())
            .field("has_video_closure", &self.video_closure.is_some())
            .finish()
    }
}
