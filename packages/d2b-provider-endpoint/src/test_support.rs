//! Re-usable recording test double for the Endpoint driver effect port.
//!
//! The daemon (`d2bd`) and this crate's own tests share one canonical
//! recording implementation of [`EndpointDriverEffects`] and
//! [`EndpointPurposeVocabulary`], so the closed admission vocabulary and the
//! socket effect script cannot drift between the owner crate and the plane.
//! The vocabulary mapping equals the daemon's `endpoint_effects.rs` output
//! (both derive from the same provider constants), which preserves the
//! plane tests' vocabulary behavior.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::endpoint::EndpointClass;
use d2b_contracts_resource::v3::ResourceRef;

use crate::driver::{
    EndpointDriverEffects, EndpointPurposeVocabulary, GuestControlProducer,
};

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
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }

    /// Script the socket as present, as the recovery-adoption setup does.
    pub fn make_present(&self) {
        self.present.store(true, Ordering::SeqCst);
    }
}

impl EndpointPurposeVocabulary for FakeSocketEffects {
    fn guest_control_producer(&self, purpose: &str) -> Option<GuestControlProducer> {
        match purpose {
            "ch-api" => Some(GuestControlProducer::VmmProcess),
            "guest-control" => Some(GuestControlProducer::Guest),
            _ => None,
        }
    }

    fn device_worker_endpoint_class(&self, purpose: &str) -> Option<EndpointClass> {
        match purpose {
            "swtpm-tpm-socket" => Some(EndpointClass::Device),
            "swtpm-control-socket" => Some(EndpointClass::Control),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl EndpointDriverEffects for FakeSocketEffects {
    async fn socket_present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        self.calls.lock().push("socket-present");
        self.present.load(Ordering::SeqCst)
    }

    async fn ensure_socket(
        &self,
        _producer_ref: &ResourceRef,
        _purpose: &str,
    ) -> Result<(), String> {
        self.calls.lock().push("ensure-socket");
        self.make_present();
        Ok(())
    }

    async fn remove_socket(
        &self,
        _producer_ref: &ResourceRef,
        _purpose: &str,
    ) -> Result<(), String> {
        self.calls.lock().push("remove-socket");
        self.present.store(false, Ordering::SeqCst);
        Ok(())
    }
}
