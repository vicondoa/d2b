//! The volume-virtiofs VolumeBinding controller.
//!
//! It reconciles `VolumeBinding` resources and never writes a Volume
//! row: it reads the referenced Volume only to resolve the named view
//! and the source that view lives in. It is the sole author of the
//! binding status projection (KTD3) and writes the fenced projection on
//! every reconcile.
//!
//! # The serving source and socket are derived, not discovered
//!
//! Everything one launch needs is derived here, from the admitted binding
//! and the Volume it names: the [`ServingSource`] locator and the private
//! socket path. The controller reads no Guest row, no Device row, and no
//! launch argument, so a Guest with no Device children prepares and serves
//! its export exactly like any other Guest (AE6). A per-Guest runtime
//! directory shared with the Device workers was the one input that made
//! export preparation depend on Device ownership, and it is gone.
//!
//! # Two completion conditions, never one
//!
//! The source is PREPARED before the consumer starts, and the consumer's
//! mount is observed after it boots. [`BindingPhase::Prepared`] is the
//! first condition and [`BindingPhase::Ready`] is the second; the fenced
//! status projection reports the pre-start condition, so the consumer's
//! start gate and its mount observation can never wait on each other
//! (R39-R40, AE21).

use std::path::{Path, PathBuf};

use d2b_contracts_resource::v3::execution_policy::BoundedToken;

use d2b_contracts_resource::v3::volume::{AttachmentAccess, ViewSpec, VolumeSpec};
use d2b_contracts_resource::v3::volume_binding::VolumeBindingStatusResource;

use crate::error::VirtiofsBindingError;
use crate::bindings::{ServingSource, VOLUME_BINDING_FINALIZER, StoredBinding};
use crate::port::{
    BindingPhase, BindingStatusReport, LaunchedWorker, MountObservation, ServingWorkerLaunch,
    VirtiofsBindingEffectPort,
};
use crate::worker::{VirtiofsdWorkerPlan, WorkerSandbox};

/// The bounded repair interval, in seconds, for virtiofs workers.
pub const VIRTIOFS_REPAIR_INTERVAL_SECS: u64 = 30;

/// The Provider component identity that owns a `VolumeBinding` row, and
/// the only identity allowed to publish that row's fenced status (KTD3).
///
/// The port checks the writer against this value before it crosses the
/// provider boundary, so a status write can never be minted under another
/// component's name.
pub const CONTROLLER_IDENTITY: &str = "volume-virtiofs";

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

/// The volume-virtiofs controller over its injected effect port.
#[derive(Debug)]
pub struct VirtiofsBindingController<P> {
    provider: BoundedToken,
    zone: BoundedToken,
    port: P,
    runtime_root: PathBuf,
}

impl<P: VirtiofsBindingEffectPort> VirtiofsBindingController<P> {
    /// Build a controller over the injected port.
    ///
    /// `zone` and `runtime_root` are the only two host inputs this crate
    /// takes, and both are properties of the controller instance, not of
    /// any resource. The controller is a Zone singleton, so the socket
    /// identity every binding derives is bound to the Zone this instance
    /// owns, and the runtime root is the broker's own directory. Neither
    /// is reached through a Guest row, so a Guest with Device children and
    /// one without derive identical deliveries.
    pub fn new(port: P, zone: BoundedToken, runtime_root: impl Into<PathBuf>) -> Self {
        Self {
            provider: BoundedToken::parse(CONTROLLER_IDENTITY)
                .expect("the frozen provider name is a bounded token"),
            zone,
            port,
            runtime_root: runtime_root.into(),
        }
    }

    /// Borrow the Zone this controller instance owns.
    pub const fn zone(&self) -> &BoundedToken {
        &self.zone
    }

    /// Borrow the Provider name.
    pub const fn provider(&self) -> &BoundedToken {
        &self.provider
    }

    /// The broker-owned runtime root serving sockets are bound in.
    pub fn runtime_root(&self) -> &Path {
        &self.runtime_root
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
            source_prepared: false,
            consumer_mount: MountObservation::ConsumerNotRunning,
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
        let plan = match VirtiofsdWorkerPlan::for_binding(
            binding,
            volume,
            view,
            vcpu_count,
            principal,
            &self.zone,
        ) {
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
        if matches!(plan.source, ServingSource::ClosureStoreView { .. })
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
                source_prepared: false,
                consumer_mount: MountObservation::ConsumerNotRunning,
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
        let launch = match self.launch_request(binding, &plan) {
            Ok(launch) => launch,
            Err(error) => {
                tracing::warn!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %self.provider.as_str(),
                    reason = error.code(),
                    "serving socket path could not be derived for binding",
                );
                return Ok(failed(error));
            }
        };
        let worker = match self.port.launch_worker(binding, &launch).await {
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
        let source_prepared = self
            .port
            .observe_socket(binding, &worker)
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
        // The consumer's mount is observed only once the source is
        // prepared, and its ABSENCE never retracts the pre-start
        // condition: a Guest that has not booted yet cannot have mounted,
        // and treating that as a failure would make the Guest's start gate
        // wait on the Guest itself.
        let consumer_mount = if source_prepared {
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
            MountObservation::ConsumerNotRunning
        };

        let (phase, reason) = Self::verdict(source_prepared, consumer_mount, &self.provider, binding);
        Ok(BindingStatusReport {
            provider: self.provider.clone(),
            phase,
            source_prepared,
            consumer_mount,
            worker_process_ref: Some(worker.process_ref),
            socket: Some(worker.socket),
            reason,
            projection: Self::projection(binding, source_prepared, reason),
        })
    }


    /// The delivery verdict for one committed row, WITHOUT publishing
    /// status (U15).
    ///
    /// This is the whole of the serving pass minus the row's fenced status
    /// publication, and it is the entry point a source-side pass uses after
    /// it commits a canonical row: the source owns the commit, and the
    /// `VolumeBinding` row's own actor owns the status projection that
    /// row publishes. Keeping the two apart is deliberate - a fenced
    /// projection is written by exactly one actor, so a second writer
    /// cannot publish a stale verdict under the row's fence.
    ///
    /// Everything else is unchanged from [`Self::reconcile`]: the worker
    /// plan, the private socket path, the closure store-view marker gate,
    /// the socket probe, and the consumer's three-state mount observation
    /// are all derived from the admitted row and answered by the privileged
    /// leg.
    pub async fn observe(
        &self,
        binding: &StoredBinding,
        volume: &VolumeSpec,
        vcpu_count: u32,
        principal: BoundedToken,
    ) -> Result<BindingStatusReport, VirtiofsBindingError> {
        self.compute_report(binding, volume, vcpu_count, principal)
            .await
    }

    /// Join the path-free plan to the private socket path the binding's
    /// own identity stands for.
    fn launch_request(
        &self,
        binding: &StoredBinding,
        plan: &VirtiofsdWorkerPlan,
    ) -> Result<ServingWorkerLaunch, VirtiofsBindingError> {
        let socket_path = binding
            .serving_socket_path(&self.zone, &self.runtime_root)
            .map_err(|error| {
                tracing::warn!(reason = error.code(), "private socket path refused");
                VirtiofsBindingError::ServingSocketPathUnresolved
            })?;
        Ok(ServingWorkerLaunch {
            plan: plan.clone(),
            socket_path,
        })
    }

    /// The pair of completion conditions, resolved to one phase.
    ///
    /// Source preparation and consumer completion are decided
    /// independently, so the phase names which of the two is outstanding
    /// rather than presenting one "ready" that both sides wait on.
    fn verdict(
        source_prepared: bool,
        consumer_mount: MountObservation,
        provider: &BoundedToken,
        binding: &StoredBinding,
    ) -> (BindingPhase, Option<VirtiofsBindingError>) {
        if !source_prepared {
            // Cardinality: pending steady state can recur every pass;
            // keep it at debug level.
            tracing::debug!(
                binding = %binding.uid().to_canonical_string(),
                provider = %provider.as_str(),
                reason = VirtiofsBindingError::BindingNotReady.code(),
                "binding pending: worker socket not ready",
            );
            return (
                BindingPhase::Pending,
                Some(VirtiofsBindingError::BindingNotReady),
            );
        }
        match consumer_mount {
            MountObservation::Present => (BindingPhase::Ready, None),
            MountObservation::ConsumerNotRunning => (BindingPhase::Prepared, None),
            MountObservation::Absent => {
                // Cardinality: degraded steady state can recur every pass;
                // keep it at debug level.
                tracing::debug!(
                    binding = %binding.uid().to_canonical_string(),
                    provider = %provider.as_str(),
                    reason = VirtiofsBindingError::GuestMountNotReady.code(),
                    "binding degraded: consumer reports no mount",
                );
                (
                    BindingPhase::Degraded,
                    Some(VirtiofsBindingError::GuestMountNotReady),
                )
            }
        }
    }

    /// Build the fenced public projection for one reconcile.
    ///
    /// The projection shape and its fence live in the binding contract
    /// ([`StoredBinding::status_projection`]); the controller only supplies
    /// the observation. What it reports is the PRE-START condition: the
    /// consumer's start gate reads this, so it is the source preparation
    /// verdict and never the consumer's mount.
    fn projection(
        binding: &StoredBinding,
        source_prepared: bool,
        reason: Option<VirtiofsBindingError>,
    ) -> VolumeBindingStatusResource {
        binding.status_projection(source_prepared, reason)
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
            .delete_worker(binding, worker)
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
            .is_mounted()
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
