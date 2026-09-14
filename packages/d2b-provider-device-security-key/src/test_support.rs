//! The crate's canonical recording test double for the security-key driver
//! effect port.
//!
//! Exposed under `test-support` so the crate's own tests and `d2bd`'s plane
//! tests read through the same `SecurityKeyDriverEffects` recording double.

use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize,
};

use crate::driver::{SecurityKeyComponent, SecurityKeyDriverEffects};

/// The canonical recording double for [`SecurityKeyDriverEffects`].
///
/// Records the reconciled components and the effect-method call order, so
/// tests can assert both which rows were reconciled and the order effects
/// ran.
#[derive(Default)]
pub struct RecordingEffects {
    reconciled: parking_lot::Mutex<Vec<SecurityKeyComponent>>,
    calls: parking_lot::Mutex<Vec<&'static str>>,
}

impl RecordingEffects {
    /// The driver-effect calls so far, in invocation order.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl SecurityKeyDriverEffects for RecordingEffects {
    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.reconciled.lock().push(component);
        self.calls.lock().push("reconcile_security_key");
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Ready,
        ))
    }

    async fn finalize(
        &self,
        _component: SecurityKeyComponent,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.calls.lock().push("finalize");
        Ok(SharedProviderFinalize::Complete)
    }
}
