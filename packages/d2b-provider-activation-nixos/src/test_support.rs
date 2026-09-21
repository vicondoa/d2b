//! Scripted driver-effect test doubles for the activation-nixos provider.
//!
//! Gated behind the `test-support` Cargo feature so production
//! consumers never pull this in.

use std::sync::Arc;

use d2b_contracts_broker::broker_wire::{BrokerRequest, BrokerResponse};
use d2b_contracts_broker::host_generation::HostGenerationHandoffIntent;
use d2b_contracts_resource::v3::ResourceRef;

use crate::driver::{ActivationDriverEffects, HostHandoffResult};
use crate::facets::{ActivationBrokerDispatch, ActivationEffectFacets};

// ── FakeActivationEffects ───────────────────────────────────────────────────

/// Scripted host-handoff port: records each dispatch and returns the
/// next scripted result.
pub struct FakeActivationEffects {
    dispatches: parking_lot::Mutex<Vec<(ResourceRef, HostGenerationHandoffIntent)>>,
    results: parking_lot::Mutex<Vec<HostHandoffResult>>,
}

impl FakeActivationEffects {
    /// Create a double whose first handoff dispatch returns the given
    /// [`HostHandoffResult`]; subsequent dispatches answer `Incomplete`
    /// once the scripted queue is exhausted.
    pub fn new(result: HostHandoffResult) -> Arc<Self> {
        Arc::new(Self {
            dispatches: parking_lot::Mutex::new(Vec::new()),
            results: parking_lot::Mutex::new(vec![result]),
        })
    }

    /// The host-generation handoff dispatches recorded so far, in call order.
    pub fn dispatches(&self) -> Vec<(ResourceRef, HostGenerationHandoffIntent)> {
        self.dispatches.lock().clone()
    }
}

#[async_trait::async_trait]
impl ActivationDriverEffects for FakeActivationEffects {
    async fn apply_host_generation_handoff(
        &self,
        target: ResourceRef,
        intent: HostGenerationHandoffIntent,
    ) -> HostHandoffResult {
        self.dispatches.lock().push((target, intent));
        self.results
            .lock()
            .pop()
            .unwrap_or(HostHandoffResult::Incomplete)
    }
}

// ── RecordingBrokerDispatch ─────────────────────────────────────────────────

/// Scripted broker dispatch double: records each dispatched request and
/// returns the next scripted response.
pub struct RecordingBrokerDispatch {
    requests: parking_lot::Mutex<Vec<BrokerRequest>>,
    results: parking_lot::Mutex<Vec<Result<BrokerResponse, String>>>,
}

impl RecordingBrokerDispatch {
    /// Create a double with no scripted responses: every dispatch fails, so
    /// the effects reduce it to `Incomplete` (the driver tests never rely on
    /// a dispatch outcome through this double).
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            requests: parking_lot::Mutex::new(Vec::new()),
            results: parking_lot::Mutex::new(Vec::new()),
        })
    }

    /// Create a double whose dispatches return the given scripted responses
    /// in order; once the queue is exhausted, a dispatch fails.
    pub fn with_responses(responses: Vec<Result<BrokerResponse, String>>) -> Arc<Self> {
        Arc::new(Self {
            requests: parking_lot::Mutex::new(Vec::new()),
            results: parking_lot::Mutex::new(responses),
        })
    }

    /// The broker requests dispatched so far, in call order.
    pub fn requests(&self) -> Vec<BrokerRequest> {
        self.requests.lock().clone()
    }
}

impl ActivationBrokerDispatch for RecordingBrokerDispatch {
    fn dispatch(&self, request: BrokerRequest) -> Result<BrokerResponse, String> {
        self.requests.lock().push(request);
        self.results
            .lock()
            .pop()
            .unwrap_or_else(|| Err("scripted-dispatch-exhausted".to_owned()))
    }
}

/// The facet set the factory and declaration tests build over: the
/// daemon-supplied broker dispatch source double (the plane supplies the
/// dispatch through the composition root).
pub fn recording_facets(broker: Arc<dyn ActivationBrokerDispatch>) -> ActivationEffectFacets {
    ActivationEffectFacets { broker }
}