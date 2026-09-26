//! Uniform recording [`UsbipDriverEffects`] double, shared by this crate's own
//! tests and by `d2bd`'s plane tests (which opt in via the `test-support`
//! feature).

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize,
};

use crate::driver::{UsbipComponent, UsbipDriverEffects};
use crate::facets::{UsbipBrokerFacets, UsbipEffectFacets, UsbipRuntime};

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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl UsbipDriverEffects for RecordingEffects {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.calls.lock().push("reconcile_usbip"); // async-gate-allow: test-support recorder lock
        self.reconciled.lock().push(component); // async-gate-allow: test-support recorder lock
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Ready,
        ))
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn finalize(
        &self,
        component: UsbipComponent,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.calls.lock().push("finalize"); // async-gate-allow: test-support recorder lock
        self.finalized.lock().push(component); // async-gate-allow: test-support recorder lock
        Ok(SharedProviderFinalize::Complete)
    }
}

/// Recording [`UsbipRuntime`] double: answers Ready/Complete for every
/// effect call and records the driven components, so `d2bd`'s plane tests
/// can build a facet set without a daemon.
#[derive(Default)]
pub struct RecordingRuntime {
    /// Components reconciled, in call order.
    pub reconciled: parking_lot::Mutex<Vec<UsbipComponent>>,
    /// Components finalized, in call order.
    pub finalized: parking_lot::Mutex<Vec<UsbipComponent>>,
}

#[async_trait]
impl UsbipRuntime for RecordingRuntime {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.reconciled.lock().push(component); // async-gate-allow: test-support recorder lock
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Ready,
        ))
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn finalize(
        &self,
        component: UsbipComponent,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.finalized.lock().push(component); // async-gate-allow: test-support recorder lock
        Ok(SharedProviderFinalize::Complete)
    }
}

/// Build a USBIP facet set from a recording runtime double. The broker
/// dispatch facet is a fail-closed double: the plane tests never invoke a
/// bind/unbind, so a call is a test bug rather than a silent success.
pub fn recording_facets(runtime: Arc<RecordingRuntime>) -> UsbipEffectFacets {
    UsbipEffectFacets {
        runtime,
        broker: UsbipBrokerFacets {
            dispatch: Arc::new(FailClosedDispatch),
        },
    }
}

/// A broker-dispatch double that refuses every call by name.
struct FailClosedDispatch;

impl crate::facets::UsbipBrokerDispatch for FailClosedDispatch {
    fn ack(
        &self,
        _request: d2b_contracts_broker::broker_wire::BrokerRequest,
    ) -> Result<(), crate::lifecycle::ServiceLifecycleError> {
        Err(crate::lifecycle::ServiceLifecycleError::Transient)
    }
}
