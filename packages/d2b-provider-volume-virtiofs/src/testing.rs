//! Shared test doubles and fixtures for the volume-virtiofs conformance
//! suite.
//!
//! Every double is hermetic: the suite asserts the Export lifecycle,
//! sandbox, and privacy obligations without a virtiofsd binary, a socket,
//! a broker, or a guest.

use std::future::Future;
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use d2b_contracts::v3::ResourceRef;

use crate::error::VirtiofsExportError;
use crate::export::ExportSpec;
use crate::port::{LaunchedWorker, VirtiofsExportEffectPort};
use crate::worker::VirtiofsdWorkerPlan;

/// Drive a future to completion on the calling thread.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::hint::spin_loop(),
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
}

/// A scripted, recording [`VirtiofsExportEffectPort`].
#[derive(Debug)]
pub struct ScriptedPort {
    store_view_marker: bool,
    socket_ready: bool,
    guest_mount_ready: bool,
    guest_mount_after_delete: bool,
    launch_error: Option<VirtiofsExportError>,
    launched_plans: Mutex<Vec<VirtiofsdWorkerPlan>>,
    calls: Mutex<Vec<PortCall>>,
}

impl ScriptedPort {
    /// A port whose worker serves and whose guest mounts.
    pub fn serving() -> Self {
        Self {
            store_view_marker: true,
            socket_ready: true,
            guest_mount_ready: true,
            guest_mount_after_delete: false,
            launch_error: None,
            launched_plans: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
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
    pub const fn failing_launch(mut self, error: VirtiofsExportError) -> Self {
        self.launch_error = Some(error);
        self
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

impl VirtiofsExportEffectPort for &ScriptedPort {
    async fn launch_worker(
        &self,
        export: &ExportSpec,
        plan: &VirtiofsdWorkerPlan,
    ) -> Result<LaunchedWorker, VirtiofsExportError> {
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
            socket: export.socket_identity(&fixtures::zone()),
        })
    }

    async fn observe_socket(&self, _worker: &LaunchedWorker) -> Result<bool, VirtiofsExportError> {
        self.record(PortCall::ObserveSocket);
        Ok(self.socket_ready)
    }

    async fn observe_guest_mount(&self, _export: &ExportSpec) -> Result<bool, VirtiofsExportError> {
        self.record(PortCall::ObserveGuestMount);
        if self.deleted() {
            return Ok(self.guest_mount_after_delete);
        }
        Ok(self.guest_mount_ready)
    }

    async fn observe_store_view_marker(
        &self,
        _export: &ExportSpec,
    ) -> Result<bool, VirtiofsExportError> {
        self.record(PortCall::ObserveStoreViewMarker);
        Ok(self.store_view_marker)
    }

    async fn delete_worker(&self, _worker: &LaunchedWorker) -> Result<(), VirtiofsExportError> {
        self.record(PortCall::DeleteWorker);
        Ok(())
    }
}

/// Canonical Export and Volume fixtures.
pub mod fixtures {
    use d2b_contracts::v3::ResourceRef;
    use d2b_contracts::v3::execution_policy::BoundedToken;
    use d2b_contracts::v3::volume::{AttachmentSettings, ViewSpec, VolumeSpec};
    use serde_json::{Value, json};

    use crate::export::ExportSpec;

    /// The Zone every fixture lives in.
    pub fn zone() -> BoundedToken {
        BoundedToken::parse("dev").expect("valid fixture token")
    }

    /// The dedicated per-Volume worker principal.
    pub fn principal() -> BoundedToken {
        BoundedToken::parse("vol-work-state-vfd").expect("valid fixture token")
    }

    /// The Volume every fixture Export references.
    pub fn volume_ref() -> ResourceRef {
        ResourceRef::parse("Volume/work-state").expect("valid fixture ref")
    }

    /// A read-only view granting only read and traverse.
    pub fn read_only_view() -> ViewSpec {
        serde_json::from_value(json!({ "path": "live", "rights": ["read", "traverse"] }))
            .expect("conformant fixture view")
    }

    /// One Export at the requested access level.
    pub fn export(access: &str) -> ExportSpec {
        export_with_settings(access, json!({}))
    }

    /// One Export with overridden attachment settings.
    pub fn export_with_settings(access: &str, settings: Value) -> ExportSpec {
        let settings: AttachmentSettings =
            serde_json::from_value(settings).expect("conformant fixture settings");
        let access = serde_json::from_value(Value::String(access.to_owned()))
            .expect("conformant fixture access");
        ExportSpec::new(
            volume_ref(),
            ResourceRef::parse("Guest/work-vm").expect("valid fixture ref"),
            BoundedToken::parse("ro-store").expect("valid fixture token"),
            access,
            settings,
        )
        .expect("conformant fixture Export")
    }

    /// The store-view Volume an Export serves read-only.
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
