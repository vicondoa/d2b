//! Scripted driver-effect test double for the activation-nixos provider.
//!
//! Gated behind the `test-support` Cargo feature so production
//! consumers never pull this in.

use std::sync::Arc;

use d2b_contracts_broker::host_generation::HostGenerationHandoffIntent;
use d2b_contracts_resource::v3::ResourceRef;

use crate::driver::{ActivationDriverEffects, HostHandoffResult};

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
