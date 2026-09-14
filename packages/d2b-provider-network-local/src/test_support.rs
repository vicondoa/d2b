//! Test-support recording doubles for `d2b-provider-network-local`.
//!
//! The crate's canonical recording [`NetworkDriverEffects`] double lives here
//! so the crate's own unit tests and `d2bd`'s plane tests share one shape,
//! rather than each defining its own bespoke fake.

use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize,
};

use crate::driver::NetworkDriverEffects;

/// Recording [`NetworkDriverEffects`] double.
///
/// Every effect call is appended to an ordered [`Self::call_order`] log while
/// the per-verb counters (`reconciled`, `finalized`) keep counting, so the
/// plane can assert both event ordering and invocation counts.
#[derive(Default)]
pub struct RecordingEffects {
    /// Ordered log of every effect call, oldest first.
    calls: parking_lot::Mutex<Vec<&'static str>>,
    /// Number of [`NetworkDriverEffects::reconcile_network`] invocations.
    pub reconciled: parking_lot::Mutex<usize>,
    /// Number of [`NetworkDriverEffects::finalize`] invocations.
    pub finalized: parking_lot::Mutex<usize>,
}

impl RecordingEffects {
    /// The ordered effect calls, oldest first.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl NetworkDriverEffects for RecordingEffects {
    async fn reconcile_network(
        &self,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.calls.lock().push("reconcile");
        *self.reconciled.lock() += 1;
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Ready,
        ))
    }

    async fn finalize(
        &self,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.calls.lock().push("finalize");
        *self.finalized.lock() += 1;
        Ok(SharedProviderFinalize::Complete)
    }
}
