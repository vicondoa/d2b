//! The declared facets the provider-owned Network effects service reaches
//! daemon state through (U14).
//!
//! The Network family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the trusted bundle's resolved Network intents,
//! the installed generation identity, the authenticated origination socket
//! and caller authority, and the reconcile orchestration (admission,
//! content fence, child rows, readiness) - crosses the provider boundary as
//! declared facets rather than as a daemon handle: every facet here is a
//! type the provider crate declares, an implementation of it is supplied by
//! the daemon host through the composition root (never derived from caller
//! input), and the family crate holds no daemon state type.
//!
//! Two facet surfaces share one daemon-supplied runtime value:
//!
//! - [`NetworkRuntime`] is the orchestration facade the driver effects
//!   delegate to: the daemon implements the reconcile and finalize
//!   machinery (admission, content fence, child rows, the reconciler) and
//!   this crate's kernel broker ([`crate::broker::KernelNetworkBroker`])
//!   runs the privileged cores over the broker-generic network kernels.
//! - [`crate::broker::NetworkIntentSource`] is the intent-resolution facet
//!   the kernel broker resolves its trusted bundle inputs through: the
//!   daemon supplies a loader over its own trusted bundle that yields a
//!   fresh resolver per invocation (the retired adapter's per-call
//!   reload),and every intent the crate resolves through it is resolved
//!   against that per-call resolver - never from caller input.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_core::bundle_resolver::BundleResolver;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};

/// The daemon-supplied facet set the provider-owned Network effects are
/// built from (U14).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct NetworkEffectFacets {
    /// The daemon's Network runtime: the orchestration the driver effects
    /// delegate to, over the daemon's own admission, child rows, and
    /// readiness state.
    pub runtime: Arc<dyn NetworkRuntime>,
}

/// The daemon-hosted Network runtime one zone's effects run over (U14).
///
/// The daemon implements this trait in its composition root (the same
/// shared-provider effects adapter that serves the other shared families),
/// supplying the trusted bundle, the authenticated origination socket and
/// caller authority, and the reconcile/finalize orchestration. The family
/// crate's effects service delegates the driver seam to it and reads the
/// bundle facts its declared service reports through it; the kernel
/// invocations themselves run through this crate's
/// [`crate::broker::KernelNetworkBroker`], which the runtime constructs
/// from the broker facets ([`crate::broker::NetworkBrokerFacets`]).
#[async_trait]
pub trait NetworkRuntime: Send + Sync + 'static {
    /// The trusted bundle the family's effects resolve their intents from.
    ///
    /// The daemon implementation loads a fresh, fully re-verified resolver
    /// per invocation (the retired adapter's per-call reload), so an
    /// on-disk bundle replacement is observed without a daemon restart; the
    /// owned `Arc` keeps the served resolver valid for the caller's read.
    /// The seat is async because the reload re-reads and re-verifies the
    /// on-disk bundle, which must not park an executor worker.
    async fn bundle(&self) -> Arc<BundleResolver>;

    /// The authenticated daemon-to-broker origination socket one kernel
    /// invocation goes over.
    fn broker_socket_path(&self) -> &Path;

    /// The caller authority one kernel invocation presents: the same
    /// `AdminUid` authority the retired daemon adapter presented.
    fn caller_role(&self) -> BrokerCallerRole;

    /// Reconcile one Network row through the family's reconciler.
    async fn reconcile_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one Network row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}