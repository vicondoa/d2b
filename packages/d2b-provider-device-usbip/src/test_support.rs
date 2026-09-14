//! Uniform recording [`UsbipDriverEffects`] double, shared by this crate's own
//! tests and by `d2bd`'s plane tests (which opt in via the `test-support`
//! feature).

use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize,
};

use crate::driver::{UsbipComponent, UsbipDriverEffects};

/// Recording [`UsbipDriverEffects`] double.
///
/// Records one invariant literal per effect method into [`RecordingEffects::calls`]
/// (readable via [`RecordingEffects::call_order`]) and preserves the typed
/// component logs for each reconcile/finalize pass so the driver's own tests
/// can assert on the values the composite rows carry.
#[derive(Default)]
pub struct RecordingEffects {
    /// Order of every effect-method invocation, one `&'static str` per method.
    pub calls: parking_lot::Mutex<Vec<&'static str>>,
    /// Components reconciled, in call order.
    pub reconciled: parking_lot::Mutex<Vec<UsbipComponent>>,
    /// Components finalized, in call order.
    pub finalized: parking_lot::Mutex<Vec<UsbipComponent>>,
}

impl RecordingEffects {
    /// Snapshot of the effect-method invocation order.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl UsbipDriverEffects for RecordingEffects {
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.calls.lock().push("reconcile_usbip");
        self.reconciled.lock().push(component);
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Ready,
        ))
    }

    async fn finalize(
        &self,
        component: UsbipComponent,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.calls.lock().push("finalize");
        self.finalized.lock().push(component);
        Ok(SharedProviderFinalize::Complete)
    }
}
