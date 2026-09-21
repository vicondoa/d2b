//! Test-support doubles for the Host family's effects surfaces.
//!
//! Three doubles share this module:
//!
//! - [`RecordingEffects`], the scripted [`HostDriverEffects`] seam: records
//!   every `observe_host` call order-preservingly and can script the
//!   reported phase or a refusal, so the driver's behavior tests run over
//!   scripted observations;
//! - [`RecordingProbe`], the scripted [`HostProbeEffectPort`] surface the
//!   family's effects run over: the same surface the crate's production
//!   probe implements, so the hosted and driver observation paths are
//!   testable hermetically;
//! - [`RecordingMinijailGate`], the scripted
//!   [`MinijailPlatformGateSource`] facet the production probe reads its
//!   platform gate through.
//!
//! Gated behind the `test-support` Cargo feature (available automatically
//! under `cargo test`), so production consumers never pull it in. The plane
//! tests in `d2bd` reach the same public surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use d2b_contracts_resource::v3::{ResourcePhase, ResourceRef};
use d2b_contracts_resource::v3::host::HostSpec;
use d2b_provider_system_core::{
    HostCapabilityClass, HostObservationReport, HostProbeEffectPort, HostProbeMetadata,
    HostReconciler, MinijailPlatformGate, SystemCoreError,
};

use crate::{HostDriverEffects, HostEffectFacets, MinijailPlatformGateSource};

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

/// Scripted `HostProbeEffectPort` double: the probe surface the family's
/// effects run over, scripted hermetically (no host state read). Records
/// the probed capability classes order-preservingly and can script a probe
/// refusal. The scripted state lives behind an `Arc`, so a test can keep a
/// handle and script the service-held probe.
pub struct RecordingProbe {
    core: Arc<RecordingProbeCore>,
}

struct RecordingProbeCore {
    calls: tokio::sync::Mutex<Vec<HostCapabilityClass>>,
    capabilities: Vec<HostCapabilityClass>,
    fail: AtomicBool,
    gate: MinijailPlatformGate,
    metadata: HostProbeMetadata,
}

impl RecordingProbe {
    /// Construct the double over one scripted capability set, the default
    /// Ready gate (6.9 with cgroup.kill), and the default bounded metadata.
    pub fn new(capabilities: Vec<HostCapabilityClass>) -> Arc<Self> {
        Arc::new(Self {
            core: Arc::new(RecordingProbeCore {
                calls: tokio::sync::Mutex::new(Vec::new()),
                capabilities,
                fail: AtomicBool::new(false),
                gate: MinijailPlatformGate::new(6, 9, true),
                metadata: HostProbeMetadata {
                    kernel_release: "6.9.0-test".to_owned(),
                    os_name: "Linux".to_owned(),
                    user_manager_available: true,
                    active_process_count: 3,
                },
            }),
        })
    }

    /// The probed capability classes in arrival order.
    pub fn probed_classes(&self) -> Vec<HostCapabilityClass> {
        self.core
            .calls
            .try_lock()
            .expect("uncontended test mutex")
            .clone()
    }

    /// Script whether the next probe refuses.
    pub fn set_failing(&self, failing: bool) {
        self.core.fail.store(failing, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl HostProbeEffectPort for RecordingProbe {
    async fn probe(&self, capability: HostCapabilityClass) -> Result<bool, SystemCoreError> {
        self.core.calls.lock().await.push(capability);
        if self.core.fail.load(Ordering::SeqCst) {
            return Err(SystemCoreError::HostProbeFailed);
        }
        Ok(self.core.capabilities.contains(&capability))
    }

    async fn platform(&self) -> Result<MinijailPlatformGate, SystemCoreError> {
        if self.core.fail.load(Ordering::SeqCst) {
            return Err(SystemCoreError::HostProbeFailed);
        }
        Ok(self.core.gate)
    }

    async fn metadata(&self) -> Result<HostProbeMetadata, SystemCoreError> {
        if self.core.fail.load(Ordering::SeqCst) {
            return Err(SystemCoreError::HostProbeFailed);
        }
        Ok(self.core.metadata.clone())
    }
}

/// Scripted minijail platform gate source: the daemon-supplied
/// [`MinijailPlatformGateSource`] facet double the production probe is
/// built from in tests.
pub struct RecordingMinijailGate {
    gate: tokio::sync::Mutex<MinijailPlatformGate>,
}

impl RecordingMinijailGate {
    /// Construct the double over one scripted gate snapshot.
    pub fn new(gate: MinijailPlatformGate) -> Arc<Self> {
        Arc::new(Self {
            gate: tokio::sync::Mutex::new(gate),
        })
    }

    /// Script the gate the next probes observe.
    pub fn set(&self, gate: MinijailPlatformGate) {
        *self.gate.try_lock().expect("uncontended test mutex") = gate;
    }
}

impl MinijailPlatformGateSource for RecordingMinijailGate {
    fn platform_gate(&self) -> MinijailPlatformGate {
        *self.gate.try_lock().expect("uncontended test mutex")
    }
}

/// Build the host family's declared facet set over the crate's production
/// probe on a scripted (or recording) minijail gate source, exactly as the
/// production composition root builds it from the daemon's gate probe.
pub fn recording_facets(gate: Arc<RecordingMinijailGate>) -> HostEffectFacets {
    HostEffectFacets {
        probe: crate::production_probe(gate),
    }
}

/// Build the host family's declared facet set over a scripted probe double:
/// the same [`HostProbeEffectPort`] surface the crate's production probe
/// implements, so the observation paths are testable hermetically.
pub fn scripted_facets(probe: Arc<RecordingProbe>) -> HostEffectFacets {
    HostEffectFacets { probe }
}
