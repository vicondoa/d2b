//! Test-support recording double for the [`HostDriverEffects`] port.
//!
//! The scripted effect fake for the Host family: records every probe call
//! order-preservingly and can script the reported phase or a probe refusal.
//! Gated behind the `test-support` Cargo feature (available automatically
//! under `cargo test`), so production consumers never pull it in. The plane
//! tests in `d2bd` reach it through the same public surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use d2b_contracts_resource::v3::{ResourcePhase, ResourceRef};
use d2b_provider_system_core::host_spec::HostSpec;
use d2b_provider_system_core::{HostCapabilityClass, HostObservationReport, HostReconciler};

use crate::HostDriverEffects;

/// Scripted observation port: records every call order-preservingly and
/// can fail the probe.
pub struct RecordingEffects {
    calls: tokio::sync::Mutex<Vec<String>>,
    phase: tokio::sync::Mutex<ResourcePhase>,
    /// Script whether the next probe refuses.
    pub fail: AtomicBool,
}

impl RecordingEffects {
    /// Construct a fresh double with the default Ready phase and no recorded
    /// calls.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: tokio::sync::Mutex::new(Vec::new()),
            phase: tokio::sync::Mutex::new(ResourcePhase::Ready),
            fail: AtomicBool::new(false),
        })
    }

    /// The observed call labels in arrival order.
    pub fn call_order(&self) -> Vec<String> {
        self.calls.try_lock().expect("uncontended test mutex").clone()
    }

    /// Script the phase the next probe reports.
    pub fn set_phase(&self, phase: ResourcePhase) {
        *self.phase.try_lock().expect("uncontended test mutex") = phase;
    }
}

#[async_trait::async_trait]
impl HostDriverEffects for RecordingEffects {
    async fn observe_host(
        &self,
        host_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        spec: &HostSpec,
    ) -> Result<HostObservationReport, String> {
        self.calls.lock().await.push("observe-host".to_owned());
        if self.fail.load(Ordering::SeqCst) {
            return Err("the scripted probe refused".to_owned());
        }
        let mut status = HostReconciler::new()
            .reconcile(host_ref, provider_ref, spec)
            .expect("the scripted host spec is admitted");
        status.phase = *self.phase.lock().await;
        Ok(HostObservationReport {
            status,
            capabilities: vec![HostCapabilityClass::Kvm],
            kernel_release: "6.9.0-test".to_owned(),
            os_name: "Linux".to_owned(),
            user_manager_available: true,
            active_process_count: 3,
            minijail_ready: true,
        })
    }
}