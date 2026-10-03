//! Scripted [`VolumeRuntime`](crate::facets::VolumeRuntime) recording
//! double for downstream crates' unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production consumers
//! never pull this in. The double records every layout-probe call in order
//! and reports a scripted Ready/Degraded layout, so tests can assert exactly
//! which layout effects the Volume driver ran.
//!
//! It also scripts the canonical binding admission (U14): a test either
//! hands the seam a set of admitted relationships or withholds the evidence
//! entirely, and observes what the driver's pass commits in each case.

use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts_resource::v3::volume::VolumeSpec;
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_provider_volume_local::AdmittedVolumeBinding;

use crate::facets::{
    BindingAdmissionEvidence, BindingEvidenceAbsent, VolumeBindingAdmission, VolumeEffectFacets,
    VolumeRuntime, VolumeServingComposition,
};

/// The refusal the boundary records when it runs an effect: it never
/// passes silently.
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

    /// The boundary carries no authority path, so it admits nothing: the
    /// named refusal the facet's own default answers with, which the driver
    /// reads as "commit no canonical row".
    async fn admit_bindings(
        &self,
        _source: &VolumeBindingAdmission<'_>,
    ) -> Result<Vec<AdmittedVolumeBinding>, BindingEvidenceAbsent> {
        Err(BindingEvidenceAbsent::new(vec![
            BindingAdmissionEvidence::Authorization,
            BindingAdmissionEvidence::FreshnessFence,
        ]))
    }
}

/// Scripted layout runtime: records every call in order.
///
/// The call log is the blocking lock: [`VolumeRuntime::has_layout`] is a
/// synchronous trait method and [`RecordingRuntime::call_order`] a
/// synchronous accessor, so neither can await an async lock. Both
/// acquisitions sit behind one sync appender and one sync snapshot, and
/// each carries its own recorded exception.
pub struct RecordingRuntime {
    calls: Mutex<Vec<&'static str>>,
    /// Whether the runtime currently reports a Ready layout
    /// (`has_layout` returns this).
    pub ready: AtomicBool,
    /// Report a Degraded/Pending layout instead of a Ready one
    /// (`reconcile_volume` returns `Ok(false)`).
    pub degraded: AtomicBool,
    /// The admitted canonical relationships `admit_bindings` answers with.
    admitted: tokio::sync::Mutex<Vec<AdmittedVolumeBinding>>,
    /// Whether the seam carries admission evidence at all. Off, the seam
    /// refuses and names the absent facts; on, it answers with `admitted`,
    /// which may legitimately be empty.
    evidence: AtomicBool,
    /// How many passes asked the canonical admission seam (U14).
    admission_passes: AtomicUsize,
    /// The Zone's privileged virtiofs serving composition (U15), and how
    /// many passes reached its presence probe.
    serving_composition: std::sync::Mutex<Option<VolumeServingComposition>>,
    serving_probes: AtomicUsize,
}

impl RecordingRuntime {
    /// A fresh runtime: no layout yet, no admission evidence, every layout
    /// call recorded.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            ready: AtomicBool::new(false),
            degraded: AtomicBool::new(false),
            admitted: tokio::sync::Mutex::new(Vec::new()),
            evidence: AtomicBool::new(false),
            admission_passes: AtomicUsize::new(0),
            serving_composition: std::sync::Mutex::new(None),
            serving_probes: AtomicUsize::new(0),
        })
    }

    /// A runtime whose layout report stays Degraded/Pending (`Ok(false)`).
    pub fn degraded() -> Arc<Self> {
        let fake = Self::new();
        fake.degraded.store(true, std::sync::atomic::Ordering::SeqCst);
        fake
    }

    /// Answer the canonical admission with exactly `admitted` relationships
    /// (U14). The seam now carries the evidence the driver's pass needs, and
    /// an empty set is a real admission of nothing - not an absence.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    pub async fn set_admitted(&self, admitted: Vec<AdmittedVolumeBinding>) {
        *self.admitted.lock().await = admitted;
        self.evidence.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Withdraw the admission evidence: the seam now refuses and names the
    /// facts it does not carry, so the driver commits no canonical row and
    /// retires none.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    pub async fn withdraw_evidence(&self) {
        *self.admitted.lock().await = Vec::new();
        self.evidence.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// How many passes reached the canonical admission seam.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    pub fn admission_passes(&self) -> usize {
        self.admission_passes.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The ordered log of layout-probe calls made through this runtime.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .expect("a test-support recorder lock is never poisoned")
            .clone()
    }

    /// Append one layout-probe call to the ordered log.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn record(&self, call: &'static str) {
        self.calls
            .lock()
            .expect("a test-support recorder lock is never poisoned")
            .push(call);
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
        self.record("ensure-layout");
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
        self.record("remove-layout");
        Ok(())
    }

    fn has_layout(&self, _volume_uid: &ResourceUid) -> bool {
        self.record("has-layout");
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The scripted canonical admission (U14): evidence on, the admitted set
    /// is returned; evidence off, the seam refuses and names what is
    /// missing, so the driver's pass commits nothing.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn admit_bindings(
        &self,
        _source: &VolumeBindingAdmission<'_>,
    ) -> Result<Vec<AdmittedVolumeBinding>, BindingEvidenceAbsent> {
        self.admission_passes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if !self.evidence.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(BindingEvidenceAbsent::new(vec![
                BindingAdmissionEvidence::Authorization,
                BindingAdmissionEvidence::FreshnessFence,
            ]));
        }
        Ok(self.admitted.lock().await.clone())
    }

    /// The scripted virtiofs serving composition (U15): the double hands
    /// the family exactly what the daemon would, and withholds it - rather
    /// than inventing one - when a test withdrew it.
    fn virtiofs_serving(&self) -> Option<VolumeServingComposition> {
        self.serving_probes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.serving_composition
            .lock()
            .expect("a test-support recorder lock is never poisoned")
            .clone()
    }
}