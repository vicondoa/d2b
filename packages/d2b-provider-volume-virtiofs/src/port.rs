//! The volume-virtiofs effect-port seam and public binding status.
//!
//! The controller validates semantics and calls this injected typed
//! port. It never imports the broker crate, spawns a process, binds a
//! socket, or resolves a host path. ProviderSupervisor alone maps a call
//! onto the broker, and the broker stays the sole privileged executor
//! and audit owner.

use std::future::Future;

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

/// The typed async effect port for the volume-virtiofs binding domain.
pub trait VirtiofsBindingEffectPort: Send + Sync {
    /// Launch the binding-owned virtiofsd worker.
    fn launch_worker(
        &self,
        binding: &StoredBinding,
        plan: &VirtiofsdWorkerPlan,
    ) -> impl Future<Output = Result<LaunchedWorker, VirtiofsBindingError>> + Send;

    /// Report whether the worker's private socket is listening.
    fn observe_socket(
        &self,
        worker: &LaunchedWorker,
    ) -> impl Future<Output = Result<bool, VirtiofsBindingError>> + Send;

    /// Report whether the guest observes the mount present.
    fn observe_guest_mount(
        &self,
        binding: &StoredBinding,
    ) -> impl Future<Output = Result<bool, VirtiofsBindingError>> + Send;

    /// Check the zero-length store-view marker before a ro-store launch.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BindingPhase {
    /// The worker exists but the share is not serving yet.
    Pending,
    /// The socket is listening and the guest mount is present.
    Ready,
    /// The socket is listening but the guest mount is not observed.
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
    /// Whether the worker reports itself serving.
    pub binding_ready: bool,
    /// Whether the guest reports the mount present.
    pub guest_mount_ready: bool,
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

fn serialize_reason<S: serde::Serializer>(
    reason: &Option<VirtiofsBindingError>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match reason {
        Some(reason) => serializer.serialize_str(reason.code()),
        None => serializer.serialize_none(),
    }
}
