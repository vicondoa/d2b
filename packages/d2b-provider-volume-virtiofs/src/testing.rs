//! Shared test doubles and fixtures for the volume-virtiofs conformance
//! suite.
//!
//! Every double is hermetic: the suite asserts the binding lifecycle,
//! sandbox, and privacy obligations without a virtiofsd binary, a socket,
//! a broker, or a guest.

use std::future::Future;
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceUid, ZoneRevision};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume_binding::VolumeBindingStatusResource;
use crate::error::VirtiofsBindingError;
use crate::export::StoredBinding;
use crate::port::{LaunchedWorker, VirtiofsBindingEffectPort};
use crate::worker::VirtiofsdWorkerPlan;

/// Drive a future to completion on the calling thread.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => continue,
        }
    }
}

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
    guest_mount_ready: bool,
    guest_mount_after_delete: bool,
    launch_error: Option<VirtiofsBindingError>,
    launched_plans: Mutex<Vec<VirtiofsdWorkerPlan>>,
    current_fence: Mutex<Option<(ResourceUid, ResourceGeneration, ZoneRevision)>>,
    calls: Mutex<Vec<PortCall>>,
    status_writes: Mutex<Vec<VolumeBindingStatusResource>>,
}

impl ScriptedPort {
    /// A port whose worker serves and whose guest mounts.
    ///
    /// The server side opens holding the canonical fixture binding
    /// identity, so writes under the fixture fence are current.
    pub fn serving() -> Self {
        let fixture = fixtures::binding("read-only");
        Self {
            store_view_marker: true,
            socket_ready: true,
            guest_mount_ready: true,
            guest_mount_after_delete: false,
            launch_error: None,
            launched_plans: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            current_fence: Mutex::new(Some((
                fixture.uid().clone(),
                fixture.generation(),
                fixture.revision(),
            ))),
            status_writes: Mutex::new(Vec::new()),
        }
    }

    /// A port whose socket never comes up.
    pub const fn socket_never_ready(mut self) -> Self {
        self.socket_ready = false;
        self
    }

    /// A port whose guest never reports the mount.
    pub const fn guest_never_mounts(mut self) -> Self {
        self.guest_mount_ready = false;
        self
    }

    /// A store-view marker that has not been published yet.
    pub const fn store_view_marker_missing(mut self) -> Self {
        self.store_view_marker = false;
        self
    }

    /// A port whose guest mount survives worker deletion.
    pub const fn mount_survives_delete(mut self) -> Self {
        self.guest_mount_after_delete = true;
        self
    }

    /// A port whose launch fails.
    pub const fn failing_launch(mut self, error: VirtiofsBindingError) -> Self {
        self.launch_error = Some(error);
        self
    }

    /// Advance the server-side binding generation: every fence observed
    /// before this call is stale (AE1).
    pub fn advance_generation(&self) {
        if let Ok(mut fence) = self.current_fence.lock()
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
    pub fn status_writes(&self) -> Vec<VolumeBindingStatusResource> {
        self.status_writes
            .lock()
            .map(|writes| writes.clone())
            .unwrap_or_default()
    }

    /// Return every recorded call in order.
    pub fn calls(&self) -> Vec<PortCall> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    /// Return every worker plan the controller asked to launch.
    pub fn launched_plans(&self) -> Vec<VirtiofsdWorkerPlan> {
        self.launched_plans
            .lock()
            .map(|plans| plans.clone())
            .unwrap_or_default()
    }

    fn record(&self, call: PortCall) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    fn deleted(&self) -> bool {
        self.calls().contains(&PortCall::DeleteWorker)
    }
}

impl VirtiofsBindingEffectPort for &ScriptedPort {
    async fn launch_worker(
        &self,
        binding: &StoredBinding,
        plan: &VirtiofsdWorkerPlan,
    ) -> Result<LaunchedWorker, VirtiofsBindingError> {
        self.record(PortCall::LaunchWorker);
        if let Ok(mut plans) = self.launched_plans.lock() {
            plans.push(plan.clone());
        }
        if let Some(error) = self.launch_error {
            return Err(error);
        }
        Ok(LaunchedWorker {
            process_ref: ResourceRef::parse("Process/vol-work-state-virtiofsd-work-vm")
                .expect("valid fixture ref"),
            socket: binding.socket_identity(&fixtures::zone()),
        })
    }

    async fn observe_socket(&self, _worker: &LaunchedWorker) -> Result<bool, VirtiofsBindingError> {
        self.record(PortCall::ObserveSocket);
        Ok(self.socket_ready)
    }

    async fn observe_guest_mount(
        &self,
        _binding: &StoredBinding,
    ) -> Result<bool, VirtiofsBindingError> {
        self.record(PortCall::ObserveGuestMount);
        if self.deleted() {
            return Ok(self.guest_mount_after_delete);
        }
        Ok(self.guest_mount_ready)
    }

    async fn observe_store_view_marker(
        &self,
        _binding: &StoredBinding,
    ) -> Result<bool, VirtiofsBindingError> {
        self.record(PortCall::ObserveStoreViewMarker);
        Ok(self.store_view_marker)
    }

    async fn delete_worker(&self, _worker: &LaunchedWorker) -> Result<(), VirtiofsBindingError> {
        self.record(PortCall::DeleteWorker);
        Ok(())
    }

    async fn write_binding_status(
        &self,
        writer: &BoundedToken,
        binding: &StoredBinding,
        projection: &VolumeBindingStatusResource,
    ) -> Result<(), VirtiofsBindingError> {
        self.record(PortCall::WriteStatus);
        if writer.as_str() != "volume-virtiofs" {
            return Err(VirtiofsBindingError::UnauthorizedWriter);
        }
        let current = self
            .current_fence
            .lock()
            .map(|fence| fence.clone())
            .unwrap_or_default();
        let Some((uid, generation, revision)) = current else {
            return Err(VirtiofsBindingError::StaleFence);
        };
        if !binding.fence().matches(&uid, generation, revision) {
            return Err(VirtiofsBindingError::StaleFence);
        }
        if let Ok(mut writes) = self.status_writes.lock() {
            writes.push(projection.clone());
        }
        Ok(())
    }
}

/// Canonical binding and Volume fixtures.
pub mod fixtures {
    use d2b_contracts_resource::v3::ResourceRef;
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::volume::{ViewSpec, VolumeSpec};
    use serde_json::{Value, json};

    use crate::export::StoredBinding;

    /// The Zone every fixture lives in.
    pub fn zone() -> BoundedToken {
        BoundedToken::parse("dev").expect("valid fixture token")
    }

    /// The dedicated per-Volume worker principal.
    pub fn principal() -> BoundedToken {
        BoundedToken::parse("vol-work-state-vfd").expect("valid fixture token")
    }

    /// The Volume every fixture binding references.
    pub fn volume_ref() -> ResourceRef {
        ResourceRef::parse("Volume/work-state").expect("valid fixture ref")
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
                "mountPath": "/nix/.ro-store",
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
}
