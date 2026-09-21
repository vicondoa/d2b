//! Scripted [`VolumeRuntime`](crate::facets::VolumeRuntime) recording
//! double for downstream crates' unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production consumers
//! never pull this in. The double records every layout-probe call in order
//! and reports a scripted Ready/Degraded layout, so tests can assert exactly
//! which layout effects the Volume driver ran.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::volume::VolumeSpec;
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};

use crate::facets::{VolumeEffectFacets, VolumeRuntime};

/// A runtime double that refuses every call: the registration boundary never
/// runs an effect, so a test that accidentally drives one fails loudly
/// instead of passing silently.
#[derive(Default)]
pub struct RefusingRuntime;

#[async_trait]
impl VolumeRuntime for RefusingRuntime {
    async fn reconcile_volume(
        &self,
        _volume_uid: &ResourceUid,
        _spec: &VolumeSpec,
        _provider: Option<&serde_json::Value>,
        _owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String> {
        Err("refused: the registration boundary must never run an effect".to_owned())
    }

    async fn cleanup_volume(
        &self,
        _volume_uid: &ResourceUid,
        _spec: &VolumeSpec,
    ) -> Result<(), String> {
        Err("refused: the registration boundary must never run an effect".to_owned())
    }

    fn has_layout(&self, _volume_uid: &ResourceUid) -> bool {
        panic!("refused: the registration boundary must never run an effect")
    }
}

/// Scripted layout runtime: records every call in order.
pub struct RecordingRuntime {
    calls: parking_lot::Mutex<Vec<&'static str>>,
    /// Whether the runtime currently reports a Ready layout
    /// (`has_layout` returns this).
    pub ready: AtomicBool,
    /// Report a Degraded/Pending layout instead of a Ready one
    /// (`reconcile_volume` returns `Ok(false)`).
    pub degraded: AtomicBool,
}

impl RecordingRuntime {
    /// A fresh runtime: no layout yet, every call recorded.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: parking_lot::Mutex::new(Vec::new()),
            ready: AtomicBool::new(false),
            degraded: AtomicBool::new(false),
        })
    }

    /// A runtime whose layout report stays Degraded/Pending (`Ok(false)`).
    pub fn degraded() -> Arc<Self> {
        let fake = Self::new();
        fake.degraded.store(true, std::sync::atomic::Ordering::SeqCst);
        fake
    }

    /// The ordered log of layout-probe calls made through this runtime.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}

/// The facet set the plane tests build the Volume family's effects from,
/// exactly as the production composition root builds it from the daemon's
/// runtime.
pub fn recording_facets(runtime: Arc<RecordingRuntime>) -> VolumeEffectFacets {
    VolumeEffectFacets { runtime }
}

#[async_trait]
impl VolumeRuntime for RecordingRuntime {
    async fn reconcile_volume(
        &self,
        _volume_uid: &ResourceUid,
        _spec: &VolumeSpec,
        _provider: Option<&serde_json::Value>,
        _owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String> {
        self.calls.lock().push("ensure-layout");
        if self.degraded.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(false);
        }
        self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(true)
    }

    async fn cleanup_volume(
        &self,
        _volume_uid: &ResourceUid,
        _spec: &VolumeSpec,
    ) -> Result<(), String> {
        self.calls.lock().push("remove-layout");
        Ok(())
    }

    fn has_layout(&self, _volume_uid: &ResourceUid) -> bool {
        self.calls.lock().push("has-layout");
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }
}