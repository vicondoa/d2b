//! Shared test doubles and fixtures for the volume-virtiofs conformance
//! suite.
//!
//! Every double is hermetic: the suite asserts the binding lifecycle,
//! sandbox, and privacy obligations without a virtiofsd binary, a socket,
//! a broker, or a guest.

use std::path::PathBuf;

use tokio::sync::Mutex;

use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceUid, ZoneRevision};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume_binding::VolumeBindingStatusResource;
use crate::error::VirtiofsBindingError;
use crate::bindings::StoredBinding;
use crate::port::{LaunchedWorker, MountObservation, ServingWorkerLaunch, VirtiofsBindingEffectPort};
use crate::worker::VirtiofsdWorkerPlan;

/// One recorded effect-port call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortCall {
    /// The store-view readiness marker was checked.
    ObserveStoreViewMarker,
    /// A worker was launched.
    LaunchWorker,
    /// The private socket was probed.
    ObserveSocket,
    /// The guest mount was probed.
    ObserveGuestMount,
    /// The worker was deleted.
    DeleteWorker,
    /// The fenced status projection was written (KTD3).
    WriteStatus,
}

/// A scripted, recording [`VirtiofsBindingEffectPort`].
///
/// The double plays the server side of the fenced status projection: a
/// write is rejected unless the writer is the virtiofs controller
/// identity and the fence still matches the identity the server holds
/// for the binding (KTD3).
#[derive(Debug)]
pub struct ScriptedPort {
    store_view_marker: bool,
    socket_ready: bool,
    consumer_mount_slot: Mutex<MountObservation>,
    mount_after_delete: MountObservation,
    launched_plans: Mutex<Vec<VirtiofsdWorkerPlan>>,
    launched_sockets: Mutex<Vec<PathBuf>>,
    current_fence: Mutex<Option<(ResourceUid, ResourceGeneration, ZoneRevision)>>,
    calls: Mutex<Vec<PortCall>>,
    status_writes: Mutex<Vec<VolumeBindingStatusResource>>,
}

impl ScriptedPort {
    /// A port whose worker serves and whose consumer has mounted.
    ///
    /// The server side opens holding the canonical fixture binding
    /// identity, so writes under the fixture fence are current.
    pub fn serving() -> Self {
        let fixture = fixtures::binding("read-only");
        Self {
            store_view_marker: true,
            socket_ready: true,
            consumer_mount_slot: Mutex::new(MountObservation::Present),
            mount_after_delete: MountObservation::Absent,
            launched_plans: Mutex::new(Vec::new()),
            launched_sockets: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            current_fence: Mutex::new(Some((
                fixture.uid().clone(),
                fixture.generation(),
                fixture.revision(),
            ))),
            status_writes: Mutex::new(Vec::new()),
        }
    }

    /// A port whose consumer has not started yet.
    ///
    /// The source still serves; only the consumer is absent. This is the
    /// pre-boot steady state a Guest's prepared export is in.
    pub fn consumer_not_running(mut self) -> Self {
        self.consumer_mount_slot = Mutex::new(MountObservation::ConsumerNotRunning);
        self
    }

    /// A port whose running consumer reports no mount.
    pub fn consumer_mount_absent(mut self) -> Self {
        self.consumer_mount_slot = Mutex::new(MountObservation::Absent);
        self
    }

    /// A port whose socket never comes up.
    pub const fn socket_never_ready(mut self) -> Self {
        self.socket_ready = false;
        self
    }

    /// A port whose consumer never reports the mount.
    pub fn guest_never_mounts(mut self) -> Self {
        self.consumer_mount_slot = Mutex::new(MountObservation::Absent);
        self
    }

    /// Move the consumer to a new observation.
    ///
    /// The consumer is the outside world to this double: the same port
    /// that reported "not running" before a boot reports "present" after
    /// one, so one controller instance can be observed across the boot
    /// the two completion conditions are separated for.
    pub async fn set_consumer_mount(&self, observation: MountObservation) {
        *self.consumer_mount_slot.lock().await = observation;
    }

    /// A store-view marker that has not been published yet.
    pub const fn store_view_marker_missing(mut self) -> Self {
        self.store_view_marker = false;
        self
    }

    /// A port whose guest mount survives worker deletion.
    pub const fn mount_survives_delete(mut self) -> Self {
        self.mount_after_delete = MountObservation::Present;
        self
    }

    /// Advance the server-side binding generation: every fence observed
    /// before this call is stale (AE1).
    ///
    /// Synchronous surface (consumed from plain `#[test]` fns without a
    /// runtime): non-blocking `try_lock` per plan U4, failing closed on a
    /// collision instead of parking the caller's thread.
    pub fn advance_generation(&self) {
        if let Ok(mut fence) = self.current_fence.try_lock()
            && let Some((_, generation, revision)) = fence.as_ref()
        {
            *fence = Some((
                fixtures::binding("read-only").uid().clone(),
                ResourceGeneration::new(generation.get() + 1)
                    .expect("fixture generation stays in range"),
                *revision,
            ));
        }
    }

    /// Return every accepted status projection, in write order.
    ///
    /// Synchronous surface (consumed from plain `#[test]` fns without a
    /// runtime): non-blocking `try_lock` per plan U4, failing closed on
    /// a collision instead of parking the caller's thread.
    pub fn status_writes(&self) -> Vec<VolumeBindingStatusResource> {
        self.status_writes
            .try_lock()
            .map(|writes| writes.clone())
            .unwrap_or_default()
    }

    /// Return every recorded call in order.
    ///
    /// Synchronous surface (consumed from plain `#[test]` fns without a
    /// runtime): non-blocking `try_lock` per plan U4, failing closed on
    /// a collision instead of parking the caller's thread.
    pub fn calls(&self) -> Vec<PortCall> {
        self.calls
            .try_lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    /// Return every worker plan the controller asked to launch.
    ///
    /// Synchronous surface (consumed from plain `#[test]` fns without a
    /// runtime): non-blocking `try_lock` per plan U4, failing closed on
    /// a collision instead of parking the caller's thread.
    pub fn launched_plans(&self) -> Vec<VirtiofsdWorkerPlan> {
        self.launched_plans
            .try_lock()
            .map(|plans| plans.clone())
            .unwrap_or_default()
    }

    /// Return every private socket path the controller asked to bind, in
    /// launch order.
    ///
    /// Synchronous surface (consumed from plain `#[test]` fns without a
    /// runtime): non-blocking `try_lock` per plan U4, failing closed on a
    /// collision instead of parking the caller's thread.
    pub fn launched_sockets(&self) -> Vec<PathBuf> {
        self.launched_sockets
            .try_lock()
            .map(|sockets| sockets.clone())
            .unwrap_or_default()
    }

    async fn record(&self, call: PortCall) {
        self.calls.lock().await.push(call);
    }

    fn deleted(&self) -> bool {
        self.calls().contains(&PortCall::DeleteWorker)
    }
}

impl VirtiofsBindingEffectPort for &ScriptedPort {
    async fn launch_worker(
        &self,
        binding: &StoredBinding,
        launch: &ServingWorkerLaunch,
    ) -> Result<LaunchedWorker, VirtiofsBindingError> {
        self.record(PortCall::LaunchWorker).await;
        self.launched_plans.lock().await.push(launch.plan.clone());
        self.launched_sockets.lock().await.push(launch.socket_path.clone());
        Ok(LaunchedWorker {
            process_ref: ResourceRef::parse("Process/vol-work-state-virtiofsd-work-vm")
                .expect("valid fixture ref"),
            socket: binding.socket_identity(&fixtures::zone()),
        })
    }

    async fn observe_socket(
        &self,
        _binding: &StoredBinding,
        _worker: &LaunchedWorker,
    ) -> Result<bool, VirtiofsBindingError> {
        self.record(PortCall::ObserveSocket).await;
        Ok(self.socket_ready)
    }

    async fn observe_guest_mount(
        &self,
        _binding: &StoredBinding,
    ) -> Result<MountObservation, VirtiofsBindingError> {
        self.record(PortCall::ObserveGuestMount).await;
        if self.deleted() {
            return Ok(self.mount_after_delete);
        }
        Ok(*self.consumer_mount_slot.lock().await)
    }

    async fn observe_store_view_marker(
        &self,
        _binding: &StoredBinding,
    ) -> Result<bool, VirtiofsBindingError> {
        self.record(PortCall::ObserveStoreViewMarker).await;
        Ok(self.store_view_marker)
    }

    async fn delete_worker(
        &self,
        _binding: &StoredBinding,
        _worker: &LaunchedWorker,
    ) -> Result<(), VirtiofsBindingError> {
        self.record(PortCall::DeleteWorker).await;
        Ok(())
    }

    async fn write_binding_status(
        &self,
        writer: &BoundedToken,
        binding: &StoredBinding,
        projection: &VolumeBindingStatusResource,
    ) -> Result<(), VirtiofsBindingError> {
        self.record(PortCall::WriteStatus).await;
        if writer.as_str() != "volume-virtiofs" {
            return Err(VirtiofsBindingError::UnauthorizedWriter);
        }
        let current = self.current_fence.lock().await.clone();
        let Some((uid, generation, revision)) = current else {
            return Err(VirtiofsBindingError::StaleFence);
        };
        if !binding.fence().matches(&uid, generation, revision) {
            return Err(VirtiofsBindingError::StaleFence);
        }
        self.status_writes.lock().await.push(projection.clone());
        Ok(())
    }
}

/// Canonical binding and Volume fixtures.
pub mod fixtures {
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::volume::{ViewSpec, VolumeSpec};
    use serde_json::{Value, json};

    use crate::bindings::StoredBinding;

    /// The Zone every fixture lives in.
    pub fn zone() -> BoundedToken {
        BoundedToken::parse("dev").expect("valid fixture token")
    }

    /// The dedicated per-Volume worker principal.
    pub fn principal() -> BoundedToken {
        BoundedToken::parse("vol-work-state-vfd").expect("valid fixture token")
    }

    /// A read-only view granting only read and traverse.
    pub fn read_only_view() -> ViewSpec {
        serde_json::from_value(json!({ "path": "live", "rights": ["read", "traverse"] }))
            .expect("conformant fixture view")
    }

    /// The canonical stored binding at the requested access level.
    pub fn binding(access: &str) -> StoredBinding {
        binding_for(access, "work-vm", "ro-store")
    }

    /// One stored binding for the requested guest and named view.
    pub fn binding_for(access: &str, guest: &str, view: &str) -> StoredBinding {
        let value = binding_envelope(access, guest, view);
        StoredBinding::from_resource_spec(&value).expect("conformant fixture binding")
    }

    /// One stored binding envelope at the requested access level.
    pub fn binding_envelope(access: &str, guest: &str, view: &str) -> Value {
        json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "VolumeBinding",
            "metadata": {
                "name": format!("vol-work-state-{view}-{guest}"),
                "zone": "dev",
                "ownerRef": "Volume/work-state",
                "uid": "123e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 7,
            },
            "spec": {
                "providerRef": "Provider/volume-virtiofs",
                "volumeRef": "Volume/work-state",
                "executionRef": format!("Guest/{guest}"),
                "view": view,
                "access": access,
                "presentation": {
                    "presentation": "filesystem",
                    "destination": "/nix/.ro-store",
                },
                "slot": format!("{view}-slot"),
                "source": {
                    "admittedRights": ["consume"],
                    "arbitration": "shared",
                    "realizedFacets": ["filesystem-presentation"],
                },
            },
        })
    }

    /// The store-view Volume a binding serves read-only.
    pub fn store_view_volume() -> VolumeSpec {
        serde_json::from_value(json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "local-path", "sourcePolicyId": "state-root" },
            },
            "kind": "durable",
            "layout": [],
            "views": {
                "ro-store": { "path": "live", "rights": ["read", "traverse"] },
                "controller": { "path": "", "rights": ["read", "write", "traverse"] },
            },
        }))
        .expect("conformant fixture Volume spec")
    }

    /// A closure-sourced Volume served read-only out of the broker-managed
    /// store-view farm rather than out of the shared content store.
    pub fn closure_store_view_volume() -> VolumeSpec {
        serde_json::from_value(json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "nix-closure", "systemArtifactId": "base-system" },
            },
            "kind": "durable",
            "layout": [],
            "views": {
                "ro-store": { "path": "live", "rights": ["read", "traverse"] },
            },
        }))
        .expect("conformant closure fixture Volume spec")
    }

    /// The closure fixture's named view.
    pub fn closure_view() -> ViewSpec {
        serde_json::from_value(json!({ "path": "live", "rights": ["read", "traverse"] }))
            .expect("conformant closure fixture view")
    }

    /// A Volume whose source kind admits no serving view at all.
    ///
    /// A tmpfs is a private per-consumer scratch source: it names no
    /// declared storage row and no store-view generation, so there is
    /// nothing to compose a view root from and the serving side refuses it
    /// rather than inventing one.
    pub fn unservable_source_volume() -> VolumeSpec {
        serde_json::from_value(json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "tmpfs" },
            },
            "kind": "ephemeral",
            "layout": [],
            "quota": { "maxBytes": 1048576, "maxInodes": 16, "enforcement": "hard" },
            "views": {
                "ro-store": { "path": "live", "rights": ["read", "traverse"] },
            },
        }))
        .expect("conformant tmpfs fixture Volume spec")
    }
}
