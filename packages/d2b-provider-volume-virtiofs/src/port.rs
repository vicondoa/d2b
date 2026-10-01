//! The volume-virtiofs effect-port seam and public binding status.
//!
//! The controller validates semantics and calls this injected typed
//! port. It never imports the broker crate, spawns a process, binds a
//! socket, or resolves a host path. ProviderSupervisor alone maps a call
//! onto the broker, and the broker stays the sole privileged executor
//! and audit owner.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::fmt;

use serde::Serialize;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume_binding::VolumeBindingStatusResource;

use crate::controller::{CONTROLLER_IDENTITY, VirtiofsBindingController};
use crate::error::VirtiofsBindingError;
use crate::bindings::{SocketIdentity, StoredBinding};
use crate::facets::{
    VirtiofsServingAnswer, VirtiofsServingDispatch, VirtiofsServingError,
    VirtiofsServingObservation, VirtiofsServingRequest,
};
use crate::worker::VirtiofsdWorkerPlan;

/// The worker the effect adapter launched for one binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchedWorker {
    /// The binding-owned virtiofsd Process resource.
    pub process_ref: ResourceRef,
    /// The opaque identity of the private listening socket.
    pub socket: SocketIdentity,
}

/// Everything one binding's serving worker launch is authorized by.
///
/// Both halves are derived by the controller FROM THE ADMITTED BINDING: the
/// path-free plan, and the private socket path the plan's opaque socket
/// identity stands for. An adapter receives no argv, no view root, no
/// shared directory, and no Guest or Device row, so it cannot widen,
/// re-point, or re-interpret the launch it was handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServingWorkerLaunch {
    /// The path-free worker plan.
    pub plan: VirtiofsdWorkerPlan,
    /// The private listening socket path, derived from the plan's socket
    /// identity under the broker-owned runtime root.
    pub socket_path: PathBuf,
}

/// What the consumer side of one binding currently reports.
///
/// The three states are distinct because source preparation and consumer
/// completion are distinct conditions (R39). A consumer that has not run
/// yet cannot report an absent mount, so collapsing `ConsumerNotRunning`
/// into `Absent` would make a Guest that is still booting look degraded
/// and would fold the pre-boot condition into the post-boot one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MountObservation {
    /// The consumer is not running, so no mount can exist yet. The
    /// pre-boot steady state: the source is prepared and the consumer has
    /// simply not started.
    ConsumerNotRunning,
    /// The consumer is running and reports no mount.
    Absent,
    /// The consumer is running and reports the mount present.
    Present,
}

impl MountObservation {
    /// Whether the consumer reports the mount itself present.
    pub const fn is_mounted(self) -> bool {
        matches!(self, Self::Present)
    }
}

/// The typed async effect port for the volume-virtiofs binding domain.
///
/// Every verb carries the committed row it is about, so the privileged leg
/// receives the KTD3 fence on each request rather than only on the one that
/// launched the worker.
pub trait VirtiofsBindingEffectPort: Send + Sync {
    /// Launch the binding-owned virtiofsd worker from the launch the
    /// controller derived.
    fn launch_worker(
        &self,
        binding: &StoredBinding,
        launch: &ServingWorkerLaunch,
    ) -> impl Future<Output = Result<LaunchedWorker, VirtiofsBindingError>> + Send;

    /// Report whether the worker's private socket is listening.
    fn observe_socket(
        &self,
        binding: &StoredBinding,
        worker: &LaunchedWorker,
    ) -> impl Future<Output = Result<bool, VirtiofsBindingError>> + Send;

    /// Report what the consumer currently observes at the mount point.
    fn observe_guest_mount(
        &self,
        binding: &StoredBinding,
    ) -> impl Future<Output = Result<MountObservation, VirtiofsBindingError>> + Send;

    /// Check the zero-length store-view marker before a closure-view
    /// launch.
    ///
    /// Adapters must explicitly prove the marker. A missing implementation
    /// fails closed instead of permitting a store-view worker launch.
    fn observe_store_view_marker(
        &self,
        _binding: &StoredBinding,
    ) -> impl Future<Output = Result<bool, VirtiofsBindingError>> + Send {
        async { Ok(false) }
    }

    /// Delete the binding-owned worker and its Endpoint.
    fn delete_worker(
        &self,
        binding: &StoredBinding,
        worker: &LaunchedWorker,
    ) -> impl Future<Output = Result<(), VirtiofsBindingError>> + Send;

    /// Write the fenced binding status projection (KTD3).
    ///
    /// The server side must validate the writer identity and the fence:
    /// a write whose fence no longer matches the stored binding is
    /// rejected, and only the virtiofs controller identity may write.
    /// A missing implementation fails closed instead of permitting an
    /// unvalidated readiness write.
    fn write_binding_status(
        &self,
        _writer: &BoundedToken,
        _binding: &StoredBinding,
        _projection: &VolumeBindingStatusResource,
    ) -> impl Future<Output = Result<(), VirtiofsBindingError>> + Send {
        async { Err(VirtiofsBindingError::UnauthorizedWriter) }
    }
}

/// Coarse lifecycle phase of one binding.
///
/// The phases separate the source's PREPARATION from the consumer's
/// COMPLETION. A Guest's storage export is prepared before the Guest
/// starts and its mount is observed after the Guest boots, so folding the
/// two into one "ready" would make each wait on the other: the Guest could
/// not start until the binding was ready, and the binding could not be
/// ready until the Guest had mounted (R39-R40, AE21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BindingPhase {
    /// The source is not prepared yet: no serving worker, no listening
    /// socket, or an unverified closure-view marker. The consumer's
    /// pre-start condition does not hold, so it must not start.
    Pending,
    /// The source is prepared and the consumer has not reported mount
    /// completion.
    ///
    /// This is the pre-boot steady state of a Guest whose export was
    /// prepared before it started. The consumer MAY start: the storage it
    /// needs to boot is already being served.
    Prepared,
    /// The source is prepared and the consumer reports the mount present.
    Ready,
    /// The source is prepared, the consumer is running, and the consumer
    /// reports no mount. The mount was observed, so its absence is a
    /// regression rather than a pre-boot state.
    Degraded,
    /// A frozen invariant does not hold; nothing was launched.
    Failed,
}

/// The volume-virtiofs written binding status report.
///
/// It carries the opaque socket identity, never the socket path, and no
/// shared directory, argv, unit name, or numeric identity. The public
/// [`BindingStatusReport::projection`] is the fenced
/// `VolumeBindingStatusResource` the controller writes on every
/// reconcile (KTD3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingStatusReport {
    /// The Provider implementation that owns this binding.
    pub provider: BoundedToken,
    /// The coarse lifecycle phase.
    pub phase: BindingPhase,
    /// Whether the source is prepared: the serving worker is up, its
    /// socket is listening, and a closure view's marker verified.
    ///
    /// This is the consumer's pre-start condition, and it holds BEFORE
    /// the consumer runs. It is never a function of the consumer's mount.
    pub source_prepared: bool,
    /// What the consumer currently observes at its mount point.
    pub consumer_mount: MountObservation,
    /// The binding-owned worker Process, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_process_ref: Option<ResourceRef>,
    /// The opaque identity of the private listening socket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket: Option<SocketIdentity>,
    /// The condition code when the binding is not Ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_reason")]
    pub reason: Option<VirtiofsBindingError>,
    /// The fenced public status projection written on this reconcile.
    pub projection: VolumeBindingStatusResource,
}

impl BindingStatusReport {
    /// Whether the consumer may start under this report.
    ///
    /// Only the pre-start condition is consulted. A consumer that has not
    /// started cannot have mounted, so a mount observation is not part of
    /// the answer - which is precisely the condition that would otherwise
    /// form a startup cycle.
    pub const fn permits_consumer_start(&self) -> bool {
        self.source_prepared
    }
}

impl Serialize for MountObservation {
    /// Serializes the observation class, never a path or a Guest name.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::ConsumerNotRunning => "consumer-not-running",
            Self::Absent => "absent",
            Self::Present => "present",
        })
    }
}

fn serialize_reason<S: serde::Serializer>(
    reason: &Option<VirtiofsBindingError>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match reason {
        Some(reason) => serializer.serialize_str(reason.code()),
        None => serializer.serialize_none(),
    }
}

// ---------------------------------------------------------------------------
// Production port: the family seam over the daemon-supplied dispatch
// ---------------------------------------------------------------------------

/// The stable code an identity the privileged leg returned did not match is
/// refused under.
///
/// The launch a worker came back under, or the socket an observation
/// answered about, is the relationship's own derived identity and nothing
/// else. An answer that names another one is not evidence about this row,
/// so it is refused by name rather than folded into a verdict.
const WORKER_IDENTITY_MISMATCH: &str = "virtiofs-worker-identity-mismatch";

/// The production effect port: the family's [`VirtiofsBindingEffectPort`]
/// over the daemon-supplied privileged dispatch (U15).
///
/// The port holds the dispatch and the Zone's bounded token, and nothing
/// else: no socket, no host path, no numerical principal. Every verb builds
/// the typed request from the committed row and the controller's own
/// derivation, and every answer is reconciled against those derived facts
/// before the serving pass believes it. A privileged leg that refuses, or
/// that never answers, fails the verb closed - the pass never reports a
/// delivery it could not observe.
pub struct VirtiofsBindingPort {
    dispatch: Arc<dyn VirtiofsServingDispatch>,
    zone: BoundedToken,
}

impl fmt::Debug for VirtiofsBindingPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VirtiofsBindingPort")
            .field("zone", &self.zone.as_str())
            .finish_non_exhaustive()
    }
}

impl VirtiofsBindingPort {
    /// Build the port over the daemon-supplied dispatch.
    ///
    /// `zone` is the socket-identity namespace the whole relationship is
    /// derived in, so it is a property of this instance rather than of any
    /// resource: two bindings that share a source and a consumer still
    /// derive two distinct sockets inside it.
    pub fn new(dispatch: Arc<dyn VirtiofsServingDispatch>, zone: BoundedToken) -> Self {
        Self { dispatch, zone }
    }

    /// The privileged dispatch this port serves.
    pub const fn dispatch(&self) -> &Arc<dyn VirtiofsServingDispatch> {
        &self.dispatch
    }

    /// The production serving pass over this port.
    ///
    /// `runtime_root` is the broker-owned directory the private serving
    /// socket path is derived under. The controller owns the derivation;
    /// the port never resolves a path of its own.
    pub fn into_controller(
        self,
        runtime_root: impl Into<PathBuf>,
    ) -> VirtiofsBindingController<Self> {
        let zone = self.zone.clone();
        VirtiofsBindingController::new(self, zone, runtime_root)
    }
    /// The privileged leg's refusal class is a foreign vocabulary and is
    /// recorded at the refusal point, where it belongs; the family's status
    /// carries the closed code the rest of the pass speaks.
    async fn ask(
        &self,
        request: VirtiofsServingRequest,
    ) -> Result<VirtiofsServingAnswer, VirtiofsBindingError> {
        self.dispatch.dispatch(request).await.map_err(|error| {
            tracing::warn!(
                zone = %self.zone.as_str(),
                reason = error.code(),
                detail = %error,
                "virtiofs serving dispatch refused the privileged leg request",
            );
            match error {
                VirtiofsServingError::Refused(_) => VirtiofsBindingError::ServingRefused,
                VirtiofsServingError::Unavailable(_) => {
                    VirtiofsBindingError::ServingUnavailable
                }
            }
        })
    }

    /// Reconcile one observation against the socket the committed row
    /// derives.
    fn reconcile_observation(
        answer: VirtiofsServingAnswer,
        socket: SocketIdentity,
    ) -> Result<VirtiofsServingObservation, VirtiofsBindingError> {
        match answer {
            VirtiofsServingAnswer::Observed(observed)
                if observed.socket() == socket =>
            {
                Ok(observed)
            }
            _ => Err(VirtiofsBindingError::WorkerIdentityMismatch),
        }
    }
}

impl VirtiofsBindingEffectPort for VirtiofsBindingPort {
    /// The worker that came back is reconciled against the worker's own
    /// derived identity: the binding's worker `Process` reference and the
    /// socket identity its relationship stands for. A worker realized
    /// under another identity is refused, never adopted.
    async fn launch_worker(
        &self,
        binding: &StoredBinding,
        launch: &ServingWorkerLaunch,
    ) -> Result<LaunchedWorker, VirtiofsBindingError> {
        let expected = binding
            .worker_process_ref()
            .map_err(|_| VirtiofsBindingError::InvalidBinding)?;
        let request = VirtiofsServingRequest::for_launch(
            binding,
            self.zone.clone(),
            expected.clone(),
            launch.clone(),
        );
        let socket = request.socket();
        if request.launch().map(|launch| launch.plan.socket) != Some(socket) {
            // The plan the controller derived and the socket the request
            // carries are the same derivation; a disagreement here is a
            // composition fault, not a privileged-leg answer.
            return Err(VirtiofsBindingError::WorkerIdentityMismatch);
        }
        match self.ask(request).await? {
            VirtiofsServingAnswer::Launched(worker)
                if worker.process_ref == expected && worker.socket == socket =>
 {
                Ok(worker)
            }
            _ => {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    reason = WORKER_IDENTITY_MISMATCH,
                    "virtiofsd worker came back under another derived identity",
                );
                Err(VirtiofsBindingError::WorkerIdentityMismatch)
            }
        }
    }

    /// Source preparation is the privileged leg's own socket evidence, for
    /// the socket this relationship derives and no other.
    async fn observe_socket(
        &self,
        binding: &StoredBinding,
        worker: &LaunchedWorker,
    ) -> Result<bool, VirtiofsBindingError> {
        let request = VirtiofsServingRequest::for_observe(
            binding,
            self.zone.clone(),
            worker.process_ref.clone(),
            Some(worker.clone()),
        );
        let socket = request.socket();
        let observed = Self::reconcile_observation(self.ask(request).await?, socket)?;
        Ok(observed.socket_listening())
    }

    /// Delivery is what the CONSUMER reports at its mount point: the three
    /// observations stay distinct, and a consumer that has not started
    /// cannot report an absent mount.
    async fn observe_guest_mount(
        &self,
        binding: &StoredBinding,
    ) -> Result<MountObservation, VirtiofsBindingError> {
        let request = VirtiofsServingRequest::for_observe(
            binding,
            self.zone.clone(),
            binding
                .worker_process_ref()
                .map_err(|_| VirtiofsBindingError::InvalidBinding)?,
            None,
        );
        let socket = request.socket();
        let observed = Self::reconcile_observation(self.ask(request).await?, socket)?;
        Ok(observed.consumer_mount())
    }

    /// A closure view's worker is launched only over a proven zero-length
    /// readiness marker. An unanswered probe fails closed: a marker that
    /// could not be read is not a marker that is present.
    async fn observe_store_view_marker(
        &self,
        binding: &StoredBinding,
    ) -> Result<bool, VirtiofsBindingError> {
        let request = VirtiofsServingRequest::for_observe(
            binding,
            self.zone.clone(),
            binding
                .worker_process_ref()
                .map_err(|_| VirtiofsBindingError::InvalidBinding)?,
            None,
        );
        let socket = request.socket();
        let observed = Self::reconcile_observation(self.ask(request).await?, socket)?;
        Ok(observed.marker_present())
    }

    /// The teardown travels as its own verb, so the key a worker was
    /// launched under cannot reproduce its removal.
    async fn delete_worker(
        &self,
        binding: &StoredBinding,
        worker: &LaunchedWorker,
    ) -> Result<(), VirtiofsBindingError> {
        let request = VirtiofsServingRequest::for_remove(
            binding,
            self.zone.clone(),
            worker.process_ref.clone(),
            worker.clone(),
        );
        let socket = request.socket();
        match self.ask(request).await? {
            VirtiofsServingAnswer::Removed => Ok(()),
            // An answer about another socket is not an answer about this
            // relationship's teardown.
            VirtiofsServingAnswer::Observed(observed) if observed.socket() == socket => {
                Err(VirtiofsBindingError::WorkerIdentityMismatch)
            }
            _ => Err(VirtiofsBindingError::WorkerIdentityMismatch),
        }
    }

    /// Only the virtiofs controller identity may publish, and the fence
    /// travels with the projection so the writing side validates it
    /// against the row it names.
    async fn write_binding_status(
        &self,
        writer: &BoundedToken,
        binding: &StoredBinding,
        projection: &VolumeBindingStatusResource,
    ) -> Result<(), VirtiofsBindingError> {
        if writer.as_str() != CONTROLLER_IDENTITY {
            return Err(VirtiofsBindingError::UnauthorizedWriter);
        }
        let request = VirtiofsServingRequest::for_publish(
            binding,
            self.zone.clone(),
            binding
                .worker_process_ref()
                .map_err(|_| VirtiofsBindingError::InvalidBinding)?,
            writer.clone(),
            projection.clone(),
        );
        match self.ask(request).await? {
            VirtiofsServingAnswer::Published => Ok(()),
            _ => Err(VirtiofsBindingError::StaleFence),
        }
    }
}
