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
use crate::facets::{
    DeviceWorkerEvidenceSource, EndpointAccessSource, EndpointBindingEffectFacets,
    EndpointEffectFacets, EndpointGrantSource, EndpointLocatorSource, EndpointRevokeSource,
    EndpointSocketSource, GuestVmmEvidenceSource,
};
use crate::binding::EndpointAccessObservation;

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

/// Scripted `EndpointBinding` delivery port: records every live-host call in
/// order and answers the endpoint owner's private resolution, the effective
/// access, the grant, and the revoke from the script a test sets.
///
/// The daemon's plane tests and this crate's own driver tests share one
/// implementation, so a delivery sequence cannot drift between the owner
/// crate and the composition root.
pub struct FakeBindingEffects {
    calls: parking_lot::Mutex<Vec<&'static str>>,
    realized: AtomicBool,
    observed: parking_lot::Mutex<Option<EndpointAccessObservation>>,
    granted: parking_lot::Mutex<Option<EndpointAccessObservation>>,
    grant_bits: parking_lot::Mutex<u32>,
    observe_error: parking_lot::Mutex<Option<String>>,
}

impl FakeBindingEffects {
    /// A fresh scripted double: the endpoint owner has realized nothing, and
    /// no observation or grant is scripted.
    #[allow(clippy::disallowed_methods, reason = "synchronous construction")]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: parking_lot::Mutex::new(Vec::new()),
            realized: AtomicBool::new(false),
            observed: parking_lot::Mutex::new(None),
            granted: parking_lot::Mutex::new(None),
            grant_bits: parking_lot::Mutex::new(0),
            observe_error: parking_lot::Mutex::new(None),
        })
    }

    /// A double whose endpoint owner has privately realized the endpoint, so
    /// a driver will go on to observe and grant.
    #[allow(clippy::disallowed_methods, reason = "synchronous construction")]
    pub fn realized() -> Arc<Self> {
        let host = Self::new();
        host.set_realized(true);
        host
    }

    /// Script the endpoint owner's private resolution verdict.
    pub fn set_realized(&self, realized: bool) {
        self.realized.store(realized, Ordering::SeqCst);
    }

    /// Script what the kernel applies to the consumer principal.
    #[allow(clippy::disallowed_methods, reason = "synchronous setter")]
    pub fn set_observed(&self, observation: Option<EndpointAccessObservation>) {
        *self.observed.lock() = observation;
    }

    /// Script what a grant leaves effective. `None` makes the grant fail, the
    /// way a socket that is not there fails to be granted.
    #[allow(clippy::disallowed_methods, reason = "synchronous setter")]
    pub fn set_granted(&self, observation: Option<EndpointAccessObservation>) {
        *self.granted.lock() = observation;
    }

    /// Script the observation failing outright.
    #[allow(clippy::disallowed_methods, reason = "synchronous setter")]
    pub fn set_observe_error(&self, error: Option<String>) {
        *self.observe_error.lock() = error;
    }

    /// The POSIX bits the last grant requested.
    #[allow(clippy::disallowed_methods, reason = "synchronous read")]
    pub fn last_grant_bits(&self) -> u32 {
        *self.grant_bits.lock()
    }

    /// How many times the grant was attempted.
    #[allow(clippy::disallowed_methods, reason = "synchronous read")]
    pub fn grant_attempts(&self) -> usize {
        self.calls.lock().iter().filter(|call| **call == "grant").count()
    }

    /// The live-host calls recorded so far, in order.
    #[allow(clippy::disallowed_methods, reason = "synchronous read")]
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }

    /// The facet set the plane and this crate's tests build the binding
    /// driver's effects from: this one double behind all four declared
    /// facets, so a delivery sequence reads as one order.
    #[allow(clippy::disallowed_methods, reason = "synchronous construction")]
    pub fn facet_set(self: &Arc<Self>) -> EndpointBindingEffectFacets {
        EndpointBindingEffectFacets {
            locator: Arc::new(ScriptedBindingFacet(Arc::clone(self))),
            access: Arc::new(ScriptedBindingFacet(Arc::clone(self))),
            grant: Arc::new(ScriptedBindingFacet(Arc::clone(self))),
            revoke: Arc::new(ScriptedBindingFacet(Arc::clone(self))),
        }
    }
}

/// The four declared binding facets, all reading the one shared double.
struct ScriptedBindingFacet(Arc<FakeBindingEffects>);

#[async_trait::async_trait]
#[allow(clippy::disallowed_methods, reason = "test-support recorder lock")]
impl EndpointLocatorSource for ScriptedBindingFacet {
    async fn realized(&self, _endpoint: &ResourceRef, _purpose: &str) -> Result<bool, String> {
        self.0.calls.lock().push("locator"); // async-gate-allow: test-support recorder lock
        Ok(self.0.realized.load(Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
#[allow(clippy::disallowed_methods, reason = "test-support recorder lock")]
impl EndpointAccessSource for ScriptedBindingFacet {
    async fn observe(
        &self,
        _endpoint: &ResourceRef,
        _purpose: &str,
        _consumer: &ResourceRef,
    ) -> Result<Option<EndpointAccessObservation>, String> {
        if let Some(error) = self.0.observe_error.lock().clone() {
            return Err(error);
        }
        self.0.calls.lock().push("observe"); // async-gate-allow: test-support recorder lock
        Ok(*self.0.observed.lock())
    }
}

#[async_trait::async_trait]
#[allow(clippy::disallowed_methods, reason = "test-support recorder lock")]
impl EndpointGrantSource for ScriptedBindingFacet {
    async fn grant(
        &self,
        _endpoint: &ResourceRef,
        _purpose: &str,
        _consumer: &ResourceRef,
        socket_right_bits: u32,
    ) -> Result<EndpointAccessObservation, String> {
        *self.0.grant_bits.lock() = socket_right_bits;
        self.0.calls.lock().push("grant"); // async-gate-allow: test-support recorder lock
        self.0
            .granted
            .lock()
            .ok_or_else(|| "the exact endpoint is not a socket".to_owned())
    }
}

#[async_trait::async_trait]
#[allow(clippy::disallowed_methods, reason = "test-support recorder lock")]
impl EndpointRevokeSource for ScriptedBindingFacet {
    async fn revoke(
        &self,
        _endpoint: &ResourceRef,
        _purpose: &str,
        _consumer: &ResourceRef,
    ) -> Result<(), String> {
        self.0.calls.lock().push("revoke"); // async-gate-allow: test-support recorder lock
        Ok(())
    }
}