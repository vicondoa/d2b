//! The declared facets the provider-owned USBIP effects service reaches
//! daemon state through (U12 usbip step).
//!
//! The USBIP family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the reconcile and finalize orchestration over the
//! daemon's admission, child rows, and readiness state, and the privileged
//! typed USBIP bind/unbind dispatch - crosses the provider boundary as
//! declared facets rather than as a daemon handle: every facet here is a
//! type the provider crate declares, an implementation of it is supplied by
//! the daemon host through the composition root (never derived from caller
//! input), and the family crate holds no daemon state type.
//!
//! Two facet surfaces share one daemon-supplied runtime value:
//!
//! - [`UsbipRuntime`] is the orchestration facade the driver effects
//!   delegate to: the daemon implements the reconcile and finalize
//!   machinery (admission, dependency barriers, child rows, the typed
//!   lifecycle controllers) over its own plane state.
//! - [`UsbipBrokerDispatch`] is the privileged broker-dispatch facet the
//!   crate's kernel dispatcher ([`crate::broker::KernelUsbipDispatcher`])
//!   sends its typed `UsbipBind`/`UsbipUnbind` requests through: the daemon
//!   supplies the authenticated origination dispatch over its broker
//!   socket, so the privileged wire path stays daemon-hosted while the
//!   dispatcher logic (the authority ledger, the lease fences, the
//!   bind/unbind sequencing) lives in the declaring crate.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::BrokerRequest;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};

use crate::driver::UsbipComponent;
use crate::lifecycle::ServiceLifecycleError;

/// The daemon-supplied facet set the provider-owned USBIP effects are built
/// from (U12 usbip step).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct UsbipEffectFacets {
    /// The daemon's USBIP runtime: the orchestration the driver effects
    /// delegate to, over the daemon's own admission, child rows, and
    /// readiness state.
    pub runtime: Arc<dyn UsbipRuntime>,
    /// The daemon-supplied broker facets the crate's kernel dispatcher is
    /// built from: the authenticated dispatch leg and the caller authority
    /// one typed bind/unbind request presents.
    pub broker: UsbipBrokerFacets,
}

/// The daemon-supplied broker facets one USBIP kernel dispatcher is built
/// from (U12 usbip step).
#[derive(Clone)]
pub struct UsbipBrokerFacets {
    /// The authenticated daemon-to-broker dispatch leg one typed USBIP
    /// bind/unbind request goes over. The daemon implementation presents
    /// the same `AdminUid` authority the retired daemon dispatcher
    /// presented.
    pub dispatch: Arc<dyn UsbipBrokerDispatch>,
}

/// The daemon-hosted USBIP runtime one zone's effects run over (U12 usbip
/// step).
///
/// The daemon implements this trait in its composition root (the same
/// shared-provider effects adapter that serves the other shared families),
/// supplying the reconcile/finalize orchestration over the daemon's own
/// admission, child rows, and readiness state. The family crate's effects
/// service delegates the driver seam to it; the privileged bind/unbind
/// invocations themselves run through this crate's
/// [`crate::broker::KernelUsbipDispatcher`], which the runtime constructs
/// from the broker facets ([`UsbipBrokerFacets`]).
#[async_trait]
pub trait UsbipRuntime: Send + Sync + 'static {
    /// Reconcile one USB Service or Binding row through the family's typed
    /// lifecycle controller.
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one USB row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}

/// The privileged typed USBIP broker dispatch one kernel dispatcher sends
/// its bind/unbind requests through (U12 usbip step).
///
/// The daemon implements this facet over its authenticated broker socket;
/// the crate's dispatcher holds no socket, path, or caller authority.
pub trait UsbipBrokerDispatch: Send + Sync + 'static {
    /// Dispatch one typed USBIP broker request and confirm its acceptance.
    fn ack(&self, request: BrokerRequest) -> Result<(), ServiceLifecycleError>;
}