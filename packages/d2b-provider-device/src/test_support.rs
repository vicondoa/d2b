//! Test-support recording doubles for the `Device` driver effect port.
//!
//! Gated behind the `test-support` Cargo feature (or `cfg(test)`) so
//! production consumers never pull this in; `d2bd`'s plane tests read the
//! [`RecordingEffects`] double through this module.

use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectOutcome, SharedProviderEffectPhase, SharedProviderEffectRequest,
    SharedProviderFinalize,
};

use crate::driver::{DeviceComponent, DeviceDriverEffects, DeviceResourceState};

/// A recording [`DeviceDriverEffects`] double.
///
/// `reconcile_device` logs the driven component into `reconciled` and both
/// effect calls append to `calls`; `call_order()` reads the calls in order.
#[derive(Default)]
pub struct RecordingEffects {
    /// The component each `reconcile_device` call drove, in call order.
    pub reconciled: parking_lot::Mutex<Vec<DeviceComponent>>,
    /// The effect-call order (`"reconcile"`, `"finalize"`).
    calls: parking_lot::Mutex<Vec<&'static str>>,
}

#[async_trait]
impl DeviceDriverEffects for RecordingEffects {
    async fn reconcile_device(
        &self,
        component: DeviceComponent,
        _request: &SharedProviderEffectRequest<'_>,
        _state: &DeviceResourceState,
    ) -> Result<
        SharedProviderEffectOutcome,
        d2b_provider_toolkit::SharedProviderEffectError,
    > {
        self.calls.lock().push("reconcile");
        self.reconciled.lock().push(component);
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Pending,
        ))
    }

    async fn finalize_device(
        &self,
        _component: DeviceComponent,
        _request: &SharedProviderEffectRequest<'_>,
        _state: &DeviceResourceState,
    ) -> Result<
        SharedProviderFinalize,
        d2b_provider_toolkit::SharedProviderEffectError,
    > {
        self.calls.lock().push("finalize");
        Ok(SharedProviderFinalize::Complete)
    }
}

impl RecordingEffects {
    /// The effect calls made so far, in call order.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}
