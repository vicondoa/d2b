//! Combined GPU/video Device reconcile state machine.
//!
//! # A device grant is a binding, not a template name
//!
//! The device nodes a worker reaches used to be a property of its template:
//! the closed `device_worker_posture` table answered "which nodes does
//! `video-worker-nvidia` get" and the launch inherited the answer, so a
//! decode *setting* selected a device *grant*.
//!
//! [`GpuDeviceGrants`] is the admitted side of that decision. Each entry is
//! one named capability the Device source admitted a `DeviceBinding` for, so
//! a worker starts only when every capability its declared shape needs is
//! present here, and a missing one is a refusal before any reservation, open,
//! or spawn. The mode still chooses which capabilities the worker asks for;
//! it no longer chooses which device it is handed.

use core::fmt;
use d2b_contracts_resource::v3::{DeviceFunction, ResourceUid, device::DeviceArbitration};

use crate::{
    GpuAuthorityAdmission, GpuAuthorityError, GpuAuthorityLease, GpuClosureProof, GpuEffectError,
    GpuEffectTokenSet, GpuLaunchTicket, GpuLifecycleEffectPort, GpuProcessIdentity,
    GpuProcessObservation, GpuProcessRole, GpuProcessSelectionError, GpuSettings, GpuWorkerSpec,
    VideoWorkerSpec, process::select_processes,
};

/// GPU controller lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPhase {
    /// No worker effects have started.
    Pending,
    /// The GPU/render-node worker is Ready.
    GpuReady,
    /// All requested workers are Ready.
    Ready,
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
    /// A worker's declared shape needs a device capability this Device has
    /// no admitted `DeviceBinding` for.
    ///
    /// The count is the number of missing capabilities; the names live in
    /// the admission, not in a refusal.
    DeviceCapabilityRefused {
        /// How many named capabilities the configured shape needs and this
        /// Device has no admitted binding for.
        missing: usize,
    },
}

impl fmt::Display for GpuControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Selection(error) => return error.fmt(formatter),
            Self::Effect(error) => return error.fmt(formatter),
            Self::InvalidState => "gpu-invalid-state",
            Self::Authority(error) => return error.fmt(formatter),
            Self::Quarantined => "gpu-authority-quarantined",
            Self::DeviceCapabilityRefused { missing } => return write!(
                formatter,
                "gpu-device-capability-refused:{missing}"
            ),
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

/// The named device capabilities this Device's workers may reach.
///
/// One entry per admitted `DeviceBinding` function. Nothing here is a node
/// path, a template name, or a posture row: the Device source resolved the
/// name to an opaque physical authority and admitted the claim, and the
/// controller only checks that every capability a worker's declared shape
/// needs is present. A capability that is absent refuses the worker before it
/// takes a reservation, opens a device, or spawns.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct GpuDeviceGrants {
    functions: Vec<DeviceFunction>,
}

impl GpuDeviceGrants {
    /// Bind one Device's admitted capabilities.
    ///
    /// # Errors
    ///
    /// Returns [`GpuControllerError::DeviceCapabilityRefused`] when one
    /// admitted name is bound twice, which would leave the set ambiguous
    /// about what it carries.
    pub fn new(functions: Vec<DeviceFunction>) -> Result<Self, GpuControllerError> {
        let mut sorted = functions.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != functions.len() {
            return Err(GpuControllerError::DeviceCapabilityRefused { missing: 1 });
        }
        Ok(Self { functions })
    }

    /// Whether one named capability is admitted.
    pub fn covers(&self, function: &DeviceFunction) -> bool {
        self.functions.contains(function)
    }

    /// Borrow the admitted capability names.
    pub fn functions(&self) -> &[DeviceFunction] {
        &self.functions
    }

    /// The capabilities the configured GPU worker shape needs.
    ///
    /// Both GPU shapes reach the DRM render node; the full shape adds the
    /// DRM device and the cross-domain buffer node its context types need.
    pub fn required_worker_functions(render_node_only: bool) -> Vec<DeviceFunction> {
        let names: &[&str] = if render_node_only {
            &["render-node"]
        } else {
            &["render-node", "dri", "udmabuf"]
        };
        names
            .iter()
            .filter_map(|name| DeviceFunction::parse(*name).ok())
            .collect()
    }

    /// The capabilities the configured video decode shape needs.
    ///
    /// The NVIDIA arm needs the three NVIDIA nodes in addition to DRM. Which
    /// arm runs is still a setting, but it now selects which *admitted*
    /// capability the worker asks for: a Device without an admitted
    /// `nvidia-uvm` claim cannot select the NVIDIA decode mode at all.
    pub fn required_video_functions(nvidia_decode: bool) -> Vec<DeviceFunction> {
        let names: &[&str] = if nvidia_decode {
            &["render-node", "nvidia-ctl", "nvidia-uvm", "nvidia-device"]
        } else {
            &["render-node"]
        };
        names
            .iter()
            .filter_map(|name| DeviceFunction::parse(*name).ok())
            .collect()
    }
}

impl core::fmt::Debug for GpuDeviceGrants {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GpuDeviceGrants")
            .field("functions", &self.functions.len())
            .finish_non_exhaustive()
    }
}

/// Combined GPU/video controller.
pub struct GpuController {
    device_uid: ResourceUid,
    arbitration: DeviceArbitration,
    settings: GpuSettings,
    grants: GpuDeviceGrants,
    tokens: GpuEffectTokenSet,
    phase: GpuPhase,
    finalizer: bool,
    gpu_role: Option<GpuProcessRole>,
    admission: Option<GpuAuthorityAdmission>,
    authority_lease: Option<GpuAuthorityLease>,
    ticket: Option<GpuLaunchTicket>,
    gpu_identity: Option<GpuProcessIdentity>,
    video_identity: Option<GpuProcessIdentity>,
    gpu_closure: Option<GpuClosureProof>,
    video_closure: Option<GpuClosureProof>,
}

impl GpuController {
    /// Construct an authority-bound controller from Core admission evidence.
    ///
    /// # Errors
    ///
    /// Returns [`GpuControllerError::Selection`] when the arbitration and
    /// settings do not admit a GPU worker process.
    pub fn new_authorized(
        admission: GpuAuthorityAdmission,
        settings: GpuSettings,
        grants: GpuDeviceGrants,
        tokens: GpuEffectTokenSet,
    ) -> Result<Self, GpuControllerError> {
        let device_uid = admission.owner().device_uid().clone();
        select_processes(&device_uid, admission.arbitration(), &settings).map_err(|error| {
            tracing::warn!(
                device = %device_uid.to_canonical_string(),
                error = %error,
                "gpu process selection failed during controller construction",
            );
            GpuControllerError::Selection(error)
        })?;
        Ok(Self {
            device_uid,
            arbitration: admission.arbitration(),
            settings,
            grants,
            tokens,
            phase: GpuPhase::Pending,
            finalizer: true,
            gpu_role: None,
            admission: Some(admission),
            authority_lease: None,
            ticket: None,
            gpu_identity: None,
            video_identity: None,
            gpu_closure: None,
            video_closure: None,
        })
    }

    /// Return the current controller phase.
    pub const fn phase(&self) -> GpuPhase {
        self.phase
    }

    /// Borrow the current desired settings.
    pub const fn settings(&self) -> &GpuSettings {
        &self.settings
    }

    /// Return whether the Provider finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Whether this controller owns a Core admission.
    pub const fn authority_reserved(&self) -> bool {
        self.authority_lease.is_some()
    }

    /// Borrow the Core admission bound to this controller.
    pub const fn admission(&self) -> Option<&GpuAuthorityAdmission> {
        self.admission.as_ref()
    }

    /// Borrow the admitted device capabilities this Device's workers reach.
    pub const fn grants(&self) -> &GpuDeviceGrants {
        &self.grants
    }

    /// The capabilities a configured worker cannot reach yet.
    ///
    /// One Device declares a GPU worker shape and, when a sidecar is
    /// configured, a video decode shape. Each needs its own admitted
    /// capabilities; a name missing here is a refusal, not a node the launch
    /// is quietly handed.
    pub fn missing_capabilities(&self) -> Vec<DeviceFunction> {
        let mut required =
            GpuDeviceGrants::required_worker_functions(self.settings.render_node_only);
        if self.settings.video_sidecar {
            required.extend(GpuDeviceGrants::required_video_functions(
                self.settings.video_nvidia_decode,
            ));
        }
        required
            .into_iter()
            .filter(|function| !self.grants.covers(function))
            .collect()
    }

    /// Borrow the opaque device grants bound to this controller.
    pub const fn tokens(&self) -> &GpuEffectTokenSet {
        &self.tokens
    }

    /// Return the current GPU process identity, if started or adopted.
    pub const fn gpu_identity(&self) -> Option<&GpuProcessIdentity> {
        self.gpu_identity.as_ref()
    }

    /// Return the current video process identity, if started or adopted.
    pub const fn video_identity(&self) -> Option<&GpuProcessIdentity> {
        self.video_identity.as_ref()
    }

    /// Reconcile through the authority-aware production effect boundary.
    ///
    /// The Host-global reservation is acquired before the first open or
    /// spawn and remains retained until [`Self::finalize_lifecycle`] confirms
    /// every worker closure.
    ///
    /// # Errors
    ///
    /// Returns [`GpuControllerError::InvalidState`] when the finalizer is
    /// missing or the controller is in a terminal phase,
    /// [`GpuControllerError::Authority`] when video is configured without a
    /// separated principal, [`GpuControllerError::Selection`] when a worker
    /// spec cannot be derived, and [`GpuControllerError::Effect`] when a
    /// reservation, open, or spawn effect fails or a started worker identity
    /// fails validation.
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
            tracing::debug!(
                device = %self.device_uid.to_canonical_string(),
                reason = "finalizer missing or terminal phase",
                "gpu lifecycle reconcile refused before any effect",
            );
            return Err(GpuControllerError::InvalidState);
        }
        let admission = self
            .admission
            .as_ref()
            .ok_or(GpuControllerError::InvalidState)
            .inspect_err(|_| {
                tracing::debug!(
                    device = %self.device_uid.to_canonical_string(),
                    reason = "Core admission missing",
                    "gpu lifecycle reconcile refused before any effect",
                );
            })?;
        if self.settings.video_sidecar && admission.video_principal().is_none() {
            tracing::warn!(
                device = %self.device_uid.to_canonical_string(),
                reason = "video sidecar configured without a separated video principal",
                "gpu authority admission rejected during reconcile",
            );
            return Err(GpuControllerError::Authority(
                GpuAuthorityError::PrincipalNotSeparated,
            ));
        }
        // A device grant comes from an admitted binding, not from the shape
        // of the row. Refuse before the reservation, the open, and the spawn
        // rather than starting a worker whose declared capabilities this
        // Device never claimed.
        let missing = self.missing_capabilities();
        if !missing.is_empty() {
            tracing::warn!(
                device = %self.device_uid.to_canonical_string(),
                missing = missing.len(),
                reason = "worker shape needs capabilities this device has no admitted binding for",
                "gpu device capability admission rejected during reconcile",
            );
            return Err(GpuControllerError::DeviceCapabilityRefused {
                missing: missing.len(),
            });
        }
        if self.authority_lease.is_none() {
            self.authority_lease = Some(
                port.reserve_authority(admission)
                    .map_err(|error| {
                        tracing::warn!(
                            device = %self.device_uid.to_canonical_string(),
                            error = %error,
                            "gpu authority reservation failed during reconcile",
                        );
                        GpuControllerError::Effect(error)
                    })?,
            );
        }
        if self.phase == GpuPhase::Ready {
            return Ok(GpuReconcileOutcome::Converged);
        }
        if self.ticket.is_none() {
            self.ticket = Some(
                port.open_authorized_devices(admission, &self.tokens)
                    .map_err(|error| {
                        tracing::warn!(
                            device = %self.device_uid.to_canonical_string(),
                            error = %error,
                            "gpu device open failed during lifecycle reconcile",
                        );
                        GpuControllerError::Effect(error)
                    })?,
            );
        }
        let ticket = self
            .ticket
            .as_ref()
            .ok_or(GpuControllerError::InvalidState)?;
        let generation = admission.owner().generation();
        if self.gpu_identity.is_none() {
            let spec = GpuWorkerSpec::gpu(&self.device_uid, &self.settings)
                .map_err(|error| {
                    tracing::warn!(
                        device = %self.device_uid.to_canonical_string(),
                        error = %error,
                        "gpu worker spec selection failed during lifecycle reconcile",
                    );
                    GpuControllerError::Selection(error)
                })?;
            let identity = port
                .start_gpu_worker(
                    &spec,
                    ticket,
                    admission.gpu_principal(),
                    admission.platform(),
                    generation,
                )
                .map_err(|error| {
                    tracing::warn!(
                        device = %self.device_uid.to_canonical_string(),
                        role = "gpu",
                        error = %error,
                        "gpu worker start failed during lifecycle reconcile",
                    );
                    GpuControllerError::Effect(error)
                })?;
            self.gpu_role = Some(spec.process().role());
            if let Err(error) = validate_started_identity(
                &identity,
                spec.process().role(),
                admission.gpu_principal(),
                admission.platform(),
                generation,
            ) {
                self.gpu_identity = Some(identity);
                self.phase = GpuPhase::Failed;
                tracing::warn!(
                    device = %self.device_uid.to_canonical_string(),
                    role = "gpu",
                    error = %error,
                    "started gpu worker identity failed validation",
                );
                return Err(GpuControllerError::Effect(error));
            }
            self.gpu_identity = Some(identity);
        }
        self.phase = GpuPhase::GpuReady;
        if self.settings.video_sidecar && self.video_identity.is_none() {
            let principal = admission
                .video_principal()
                .ok_or(GpuControllerError::Authority(
                    GpuAuthorityError::PrincipalNotSeparated,
                ))
                .inspect_err(|_| {
                    tracing::warn!(
                        device = %self.device_uid.to_canonical_string(),
                        reason = "video sidecar configured without a separated video principal",
                        "video worker start refused during lifecycle reconcile",
                    );
                })?;
            let spec = VideoWorkerSpec::new(&self.device_uid, &self.settings)
                .map_err(|error| {
                    tracing::warn!(
                        device = %self.device_uid.to_canonical_string(),
                        role = "video",
                        error = %error,
                        "video worker spec selection failed during lifecycle reconcile",
                    );
                    GpuControllerError::Selection(error)
                })?;
            let identity = port
                .start_video_worker(&spec, ticket, principal, admission.platform(), generation)
                .map_err(|error| {
                    tracing::warn!(
                        device = %self.device_uid.to_canonical_string(),
                        role = "video",
                        error = %error,
                        "video worker start failed during lifecycle reconcile",
                    );
                    GpuControllerError::Effect(error)
                })?;
            if let Err(error) = validate_started_identity(
                &identity,
                GpuProcessRole::Video,
                principal,
                admission.platform(),
                generation,
            ) {
                self.video_identity = Some(identity);
                self.phase = GpuPhase::Failed;
                tracing::warn!(
                    device = %self.device_uid.to_canonical_string(),
                    role = "video",
                    error = %error,
                    "started video worker identity failed validation",
                );
                return Err(GpuControllerError::Effect(error));
            }
            self.video_identity = Some(identity);
        }
        self.phase = GpuPhase::Ready;
        Ok(GpuReconcileOutcome::Converged)
    }

    /// Adopt matching GPU/video workers after a daemon restart.
    ///
    /// # Errors
    ///
    /// Returns [`GpuControllerError::InvalidState`] when the finalizer is
    /// missing, the controller is in a terminal phase, or admission is
    /// absent, [`GpuControllerError::Authority`] when video is configured
    /// without a separated principal,
    /// [`GpuControllerError::Selection`] when a worker spec cannot be
    /// derived, [`GpuControllerError::Quarantined`] when the restart
    /// observation is ambiguous, and [`GpuControllerError::Effect`] when a
    /// probe effect fails or an observed identity does not match.
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
        let missing = self.missing_capabilities();
        if !missing.is_empty() {
            return Err(GpuControllerError::DeviceCapabilityRefused {
                missing: missing.len(),
            });
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
    ///
    /// # Errors
    ///
    /// Returns [`GpuControllerError::Effect`] when a stop effect fails or a
    /// closure proof does not match the stopped worker identity.
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
            .field("granted_capabilities", &self.grants.functions().len())
            .field("phase", &self.phase)
            .field("finalizer", &self.finalizer)
            .field("gpu_role", &self.gpu_role)
            .field("video_started", &self.video_identity.is_some())
            .field("has_authority", &self.authority_lease.is_some())
            .field("has_gpu_identity", &self.gpu_identity.is_some())
            .field("has_video_identity", &self.video_identity.is_some())
            .field("has_gpu_closure", &self.gpu_closure.is_some())
            .field("has_video_closure", &self.video_closure.is_some())
            .finish()
    }
}
