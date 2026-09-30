//! The volume-virtiofs effect-port seam and public binding status.
//!
//! The controller validates semantics and calls this injected typed
//! port. It never imports the broker crate, spawns a process, binds a
//! socket, or resolves a host path. ProviderSupervisor alone maps a call
//! onto the broker, and the broker stays the sole privileged executor
//! and audit owner.

use std::future::Future;
use std::path::PathBuf;

use serde::Serialize;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume_binding::VolumeBindingStatusResource;

use crate::error::VirtiofsBindingError;
use crate::bindings::{SocketIdentity, StoredBinding};
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
