//! Re-usable recording test double for the Endpoint driver effect port.
//!
//! The daemon (`d2bd`) and this crate's own tests share one canonical
//! recording implementation of [`EndpointDriverEffects`] and
//! [`EndpointPurposeVocabulary`], so the closed admission vocabulary and the
//! socket effect script cannot drift between the owner crate and the plane.
//! The vocabulary mapping delegates to this crate's own derivations (both
//! derive from the same provider constants), which preserves the plane
//! tests' vocabulary behavior.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::endpoint::EndpointClass;
use d2b_contracts_resource::v3::ResourceRef;

use crate::driver::{
    EndpointDriverEffects, EndpointPurposeVocabulary, GuestControlProducer,
};
use crate::effects_service::{device_worker_endpoint_class, guest_control_producer};
use crate::facets::{DeviceWorkerEvidenceSource, EndpointEffectFacets, EndpointSocketSource, GuestVmmEvidenceSource};

/// Scripted socket port: records every call in order, and answers the
/// purpose derivations with the purposes the declaring providers commit
/// (the Cloud Hypervisor child roles and the Device TPM worker sockets).
pub struct FakeSocketEffects {
    calls: parking_lot::Mutex<Vec<&'static str>>,
    present: AtomicBool,
}

impl FakeSocketEffects {
    /// A fresh scripted double: no recorded calls, socket absent.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: parking_lot::Mutex::new(Vec::new()),
            present: AtomicBool::new(false),
        })
    }

    /// The socket effect calls recorded so far, in order.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }

    /// Script the socket as present, as the recovery-adoption setup does.
    pub fn make_present(&self) {
        self.present.store(true, Ordering::SeqCst);
    }

    /// The facet set the plane and this crate's tests build the driver and
    /// the effects service from: the scripted socket double behind the host
    /// socket facet, and the same scripted presence behind the two
    /// row-evidence facets (a realized evidence family answers the same
    /// scripted flag the socket family answers).
    pub fn facet_set(self: &Arc<Self>) -> EndpointEffectFacets {
        EndpointEffectFacets {
            socket: Arc::new(ScriptedSocketSource(Arc::clone(self))),
            guest_vmm: Arc::new(ScriptedEvidence(Arc::clone(self))),
            device_worker: Arc::new(ScriptedEvidence(Arc::clone(self))),
        }
    }
}

/// The scripted host socket facet: records the socket calls on the shared
/// double and answers its scripted presence.
struct ScriptedSocketSource(Arc<FakeSocketEffects>);

#[async_trait::async_trait]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
impl EndpointSocketSource for ScriptedSocketSource {
    async fn present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        self.0.calls.lock().push("socket-present"); // async-gate-allow: test-support recorder lock
        self.0.present.load(Ordering::SeqCst)
    }

    async fn ensure(&self, _producer_ref: &ResourceRef, _purpose: &str) -> Result<(), String> {
        self.0.calls.lock().push("ensure-socket"); // async-gate-allow: test-support recorder lock
        self.0.make_present();
        Ok(())
    }

    async fn remove(&self, _producer_ref: &ResourceRef, _purpose: &str) -> Result<(), String> {
        self.0.calls.lock().push("remove-socket"); // async-gate-allow: test-support recorder lock
        self.0.present.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// The scripted row-evidence double: both evidence families answer the
/// shared double's scripted presence, so a test scripts the evidence row
/// `Ready` with the same `make_present()` the socket family scripts.
struct ScriptedEvidence(Arc<FakeSocketEffects>);

#[async_trait::async_trait]
impl GuestVmmEvidenceSource for ScriptedEvidence {
    async fn present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        self.0.present.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl DeviceWorkerEvidenceSource for ScriptedEvidence {
    async fn present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        self.0.present.load(Ordering::SeqCst)
    }
}

impl EndpointPurposeVocabulary for FakeSocketEffects {
    fn guest_control_producer(&self, purpose: &str) -> Option<GuestControlProducer> {
        guest_control_producer(purpose)
    }

    fn device_worker_endpoint_class(&self, purpose: &str) -> Option<EndpointClass> {
        device_worker_endpoint_class(purpose)
    }
}

#[async_trait::async_trait]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
impl EndpointDriverEffects for FakeSocketEffects {
    async fn socket_present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        self.calls.lock().push("socket-present"); // async-gate-allow: test-support recorder lock
        self.present.load(Ordering::SeqCst)
    }

    async fn ensure_socket(
        &self,
        _producer_ref: &ResourceRef,
        _purpose: &str,
    ) -> Result<(), String> {
        self.calls.lock().push("ensure-socket"); // async-gate-allow: test-support recorder lock
        self.make_present();
        Ok(())
    }

    async fn remove_socket(
        &self,
        _producer_ref: &ResourceRef,
        _purpose: &str,
    ) -> Result<(), String> {
        self.calls.lock().push("remove-socket"); // async-gate-allow: test-support recorder lock
        self.present.store(false, Ordering::SeqCst);
        Ok(())
    }
}