//! The volume-virtiofs VolumeBinding controller.
//!
//! It reconciles `VolumeBinding` resources and never writes a Volume
//! row: it reads the referenced Volume only to resolve the named view
//! and the target Guest's vcpu count. It is the sole author of the
//! binding status projection (KTD3) and writes the fenced projection on
//! every reconcile.

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::resource_status::ResourcePhase;
use d2b_contracts_resource::v3::volume::{AttachmentAccess, ViewSpec, VolumeSpec};
use d2b_contracts_resource::v3::volume_binding::VolumeBindingStatusResource;

use crate::error::VirtiofsBindingError;
use crate::bindings::{VOLUME_BINDING_FINALIZER, VOLUME_BINDING_RESOURCE_TYPE, StoredBinding};
use crate::port::{BindingPhase, BindingStatusReport, LaunchedWorker, VirtiofsBindingEffectPort};
use crate::worker::{VirtiofsdWorkerPlan, WorkerSandbox};

/// The exact shared-Runner contract for `volume-virtiofs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtiofsRunnerContract {
    /// The standard ResourceType served by this Provider.
    pub resource_type: &'static str,
    /// The finalizer installed on VolumeBinding resources.
    pub finalizer: &'static str,
    /// Bounded repair interval in seconds.
    pub repair_interval_secs: u64,
    /// Whether configuration is dependency-only.
    pub watched_configuration_is_dependency: bool,
}

/// Return the production volume-virtiofs Runner contract.
pub const fn virtiofs_runner_contract() -> VirtiofsRunnerContract {
    VirtiofsRunnerContract {
        resource_type: VOLUME_BINDING_RESOURCE_TYPE,
        finalizer: VOLUME_BINDING_FINALIZER,
        repair_interval_secs: 30,
        watched_configuration_is_dependency: true,
    }
}

/// Resolve the named view a binding selects, read-only.
pub fn resolve_view<'spec>(
    volume: &'spec VolumeSpec,
    binding: &StoredBinding,
) -> Result<&'spec ViewSpec, VirtiofsBindingError> {
    volume
        .views()
        .get(binding.spec().view().as_str())
        .ok_or(VirtiofsBindingError::ViewNotFound)
}

/// Map a coarse serving phase onto the universal lifecycle phase.
pub const fn binding_phase(phase: BindingPhase) -> ResourcePhase {
    match phase {
        BindingPhase::Pending => ResourcePhase::Pending,
        BindingPhase::Ready => ResourcePhase::Ready,
        BindingPhase::Degraded => ResourcePhase::Degraded,
        BindingPhase::Failed => ResourcePhase::Failed,
    }
}

/// The volume-virtiofs controller over its injected effect port.
#[derive(Debug)]
pub struct VirtiofsBindingController<P> {
    provider: BoundedToken,
    port: P,
}

impl<P: VirtiofsBindingEffectPort> VirtiofsBindingController<P> {
    /// Build a controller over the injected port.
    pub fn new(port: P) -> Self {
        Self {
            provider: BoundedToken::parse("volume-virtiofs").expect("frozen provider name"),
            port,
        }
    }

    /// Borrow the Provider name.
    pub const fn provider(&self) -> &BoundedToken {
        &self.provider
    }

    /// The finalizer this controller adds, and only to a VolumeBinding.
    pub const fn finalizer(&self) -> &'static str {
        VOLUME_BINDING_FINALIZER
    }

    /// Reconcile one binding to a serving worker and report its status.
    ///
    /// Terminal failures surface a Failed phase with a stable reason
    /// instead of collapsing to Pending (KTD5). Every reconcile whose
    /// verdict could be computed writes the fenced public status
    /// projection through the effect port under the virtiofs controller
    /// identity (KTD3); a rejected write fails the reconcile closed.
    pub async fn reconcile(
        &self,
        binding: &StoredBinding,
        volume: &VolumeSpec,
        vcpu_count: u32,
        principal: BoundedToken,
    ) -> Result<BindingStatusReport, VirtiofsBindingError> {
        let report = self
            .compute_report(binding, volume, vcpu_count, principal)
            .await?;
        self.port
            .write_binding_status(&self.provider, binding, &report.projection)
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = error.code(),
                    "binding status write rejected; reconcile failed closed",
                );
            })?;
        Ok(report)
    }

    /// Compute one reconcile verdict without writing status.
    async fn compute_report(
        &self,
        binding: &StoredBinding,
        volume: &VolumeSpec,
        vcpu_count: u32,
        principal: BoundedToken,
    ) -> Result<BindingStatusReport, VirtiofsBindingError> {
        let failed = |reason: VirtiofsBindingError| BindingStatusReport {
            provider: self.provider.clone(),
            phase: BindingPhase::Failed,
            binding_ready: false,
            guest_mount_ready: false,
            worker_process_ref: None,
            socket: None,
            reason: Some(reason),
            projection: Self::projection(binding, false, Some(reason)),
        };

        if binding.spec().access() == AttachmentAccess::SharedWrite {
            tracing::warn!(
                binding = %binding.uid().to_canonical_string(),
                provider = %self.provider.as_str(),
                reason = VirtiofsBindingError::SharedWriteUnsupported.code(),
                "binding rejected: shared-write attachment unsupported",
            );
            return Ok(failed(VirtiofsBindingError::SharedWriteUnsupported));
        }
        if let Err(error) = WorkerSandbox::conformant().assert_conformant() {
            tracing::warn!(
                binding = %binding.uid().to_canonical_string(),
                provider = %self.provider.as_str(),
                reason = error.code(),
                "virtiofsd sandbox posture assertion failed; worker launch refused",
            );
            return Err(error);
        }
        let view = match resolve_view(volume, binding) {
            Ok(view) => view,
            Err(reason) => {
                tracing::warn!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = reason.code(),
                    "binding view resolution failed",
                );
                return Ok(failed(reason));
            }
        };
        let plan = match VirtiofsdWorkerPlan::for_binding(binding, view, vcpu_count, principal) {
            Ok(plan) => plan,
            Err(error) => {
                tracing::warn!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = error.code(),
                    "virtiofsd worker plan rejected for binding",
                );
                return Ok(failed(error));
            }
        };
        if binding.spec().view().as_str() == "ro-store"
            && !self
                .port
                .observe_store_view_marker(binding)
                .await
                .inspect_err(|error| {
                    tracing::warn!(
                        binding = %binding.uid().to_canonical_string(),
                        provider = %self.provider.as_str(),
                        reason = error.code(),
                        "store-view marker probe failed for binding",
                    );
                })?
        {
            // Cardinality: dependency wait that can recur every pass;
            // keep it at debug level.
            tracing::debug!(
                binding = %binding.uid().to_canonical_string(),
                provider = %self.provider.as_str(),
                reason = VirtiofsBindingError::StoreViewMarkerMissing.code(),
                "binding pending: store-view marker missing",
            );
            return Ok(BindingStatusReport {
                provider: self.provider.clone(),
                phase: BindingPhase::Pending,
                binding_ready: false,
                guest_mount_ready: false,
                worker_process_ref: None,
                socket: None,
                reason: Some(VirtiofsBindingError::StoreViewMarkerMissing),
                projection: Self::projection(
                    binding,
                    false,
                    Some(VirtiofsBindingError::StoreViewMarkerMissing),
                ),
            });
        }

        let worker = match self.port.launch_worker(binding, &plan).await {
            Ok(worker) => worker,
            Err(error) => {
                tracing::warn!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = error.code(),
                    "virtiofsd worker launch failed for binding",
                );
                return Ok(failed(error));
            }
        };
        let binding_ready = self
            .port
            .observe_socket(&worker)
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    worker = %worker.process_ref.to_canonical_string(),
                    reason = error.code(),
                    "worker socket probe failed for binding",
                );
            })?;
        let guest_mount_ready = if binding_ready {
            self.port
                .observe_guest_mount(binding)
                .await
                .inspect_err(|error| {
                    tracing::warn!(
                        binding = %binding.uid().to_canonical_string(),
                        provider = %self.provider.as_str(),
                        reason = error.code(),
                        "guest mount probe failed for binding",
                    );
                })?
        } else {
            false
        };

        let (phase, reason) = match (binding_ready, guest_mount_ready) {
            (true, true) => (BindingPhase::Ready, None),
            (true, false) => {
                // Cardinality: degraded steady state can recur every pass;
                // keep it at debug level.
                tracing::debug!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = VirtiofsBindingError::GuestMountNotReady.code(),
                    "binding degraded: guest mount not ready",
                );
                (
                    BindingPhase::Degraded,
                    Some(VirtiofsBindingError::GuestMountNotReady),
                )
            }
            (false, _) => {
                // Cardinality: pending steady state can recur every pass;
                // keep it at debug level.
                tracing::debug!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = VirtiofsBindingError::BindingNotReady.code(),
                    "binding pending: worker socket not ready",
                );
                (
                    BindingPhase::Pending,
                    Some(VirtiofsBindingError::BindingNotReady),
                )
            }
        };
        Ok(BindingStatusReport {
            provider: self.provider.clone(),
            phase,
            binding_ready,
            guest_mount_ready,
            worker_process_ref: Some(worker.process_ref),
            socket: Some(worker.socket),
            reason,
            projection: Self::projection(binding, phase == BindingPhase::Ready, reason),
        })
    }

    /// Build the fenced public projection for one reconcile.
    ///
    /// The projection shape and its fence live in the binding contract
    /// ([`StoredBinding::status_projection`]); the controller only supplies
    /// the observation (its Ready phase and the frozen failure reason).
    fn projection(
        binding: &StoredBinding,
        ready: bool,
        reason: Option<VirtiofsBindingError>,
    ) -> VolumeBindingStatusResource {
        binding.status_projection(ready, reason)
    }

    /// Drain one binding before its finalizer is cleared.
    ///
    /// The owned worker and Endpoint are deleted first, then the guest
    /// mount is confirmed absent. A mount that is still present blocks
    /// the drain rather than being force-cleared (KTD6).
    pub async fn drain(
        &self,
        binding: &StoredBinding,
        worker: &LaunchedWorker,
    ) -> Result<(), VirtiofsBindingError> {
        self.port
            .delete_worker(worker)
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = error.code(),
                    "virtiofsd worker delete failed during drain",
                );
            })?;
        if self
            .port
            .observe_guest_mount(binding)
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = error.code(),
                    "guest mount probe failed during drain",
                );
            })?
        {
            // Cardinality: once per blocked finalization pass.
            tracing::warn!(
                binding = %binding.uid().to_canonical_string(),
                provider = %self.provider.as_str(),
                reason = VirtiofsBindingError::DrainIncomplete.code(),
                "binding drain blocked: guest mount still present",
            );
            return Err(VirtiofsBindingError::DrainIncomplete);
        }
        Ok(())
    }
}
