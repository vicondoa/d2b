//! Test-support recording doubles for the `Device` driver effect port.
//!
//! Gated behind the `test-support` Cargo feature (or `cfg(test)`) so
//! production consumers never pull this in; `d2bd`'s plane tests read the
//! recording doubles through this module.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize,
};

use crate::driver::{DeviceComponent, DeviceResourceState};
use crate::facets::{DeviceEffectFacets, DeviceRuntime};

/// Recording [`DeviceRuntime`] double: answers Pending/Complete for every
/// effect call and records the driven components, so `d2bd`'s plane tests
/// can build a facet set without a daemon.
#[derive(Default)]
pub struct RecordingRuntime {
    /// Components reconciled, in call order.
    pub reconciled: parking_lot::Mutex<Vec<DeviceComponent>>,
    /// Components finalized, in call order.
    pub finalized: parking_lot::Mutex<Vec<DeviceComponent>>,
}

#[async_trait]
impl DeviceRuntime for RecordingRuntime {
    async fn reconcile_device(
        &self,
        component: DeviceComponent,
        _request: &SharedProviderEffectRequest<'_>,
        _state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.reconciled.lock().push(component); // async-gate-allow: test-support recorder lock
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Pending,
        ))
    }

    async fn finalize_device(
        &self,
        component: DeviceComponent,
        _request: &SharedProviderEffectRequest<'_>,
        _state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.finalized.lock().push(component); // async-gate-allow: test-support recorder lock
        Ok(SharedProviderFinalize::Complete)
    }
}

/// Build a Device facet set from a recording runtime double.
pub fn recording_facets(runtime: Arc<RecordingRuntime>) -> DeviceEffectFacets {
    DeviceEffectFacets { runtime }
}
