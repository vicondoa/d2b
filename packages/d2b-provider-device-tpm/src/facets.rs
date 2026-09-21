//! The declared facets the provider-owned TPM effects service reaches
//! daemon state through (U12 tpm step).
//!
//! The TPM family's resource effect port ([`crate::resource_effect::
//! TpmResourceEffectPort`]) is implemented by this crate's own effects
//! module (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the trusted bundle the state-directory intent is
//! resolved from, the authenticated origination socket and caller authority
//! one kernel invocation goes over, and the owning Guest's lifecycle
//! admission - crosses the provider boundary as a declared facet rather
//! than as a daemon handle: every facet here is a type the provider crate
//! declares, an implementation of it is supplied by the daemon host through
//! the composition root (never derived from caller input), and the family
//! crate holds no daemon state type.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_core::bundle_resolver::BundleResolver;

use crate::resource_effect::TpmResourceEffectError;

/// The daemon-supplied facet set the provider-owned TPM effects are built
/// from (U12 tpm step).
///
/// The composition root supplies the object; the port never holds a daemon
/// state type (R2).
#[derive(Clone)]
pub struct TpmEffectFacets {
    /// The daemon's TPM runtime: the trusted bundle, the authenticated
    /// broker leg, and the owning Guest's lifecycle admission, supplied
    /// through the composition root.
    pub runtime: Arc<dyn TpmRuntime>,
}

/// The daemon-hosted TPM runtime one Device's effects run over (U12 tpm
/// step).
///
/// The daemon implements this trait in its composition root. The family
/// crate's effects module builds its port from it; the manager-routed child
/// rows the port ensures and reads cross as the toolkit's
/// [`d2b_provider_toolkit::SharedProviderChildSurface`], never as daemon
/// state.
#[async_trait::async_trait]
pub trait TpmRuntime: Send + Sync + 'static {
    /// The authenticated daemon-to-broker origination socket one kernel
    /// invocation goes over.
    fn broker_socket_path(&self) -> &Path;

    /// The caller authority one kernel invocation presents: the same
    /// `AdminUid` authority the retired daemon adapter presented.
    fn caller_role(&self) -> BrokerCallerRole;

    /// The io budget one kernel invocation serves under.
    fn kernel_io_timeout(&self) -> Duration;

    /// The trusted bundle the family's effects resolve their intents from.
    ///
    /// The daemon implementation loads a fresh, fully re-verified resolver
    /// per invocation (the retired adapter's per-call reload), so an
    /// on-disk bundle replacement is observed without a daemon restart; the
    /// owned `Arc` keeps the served resolver valid for the caller's
    /// synchronous read. A bundle that fails verification is refused
    /// closed.
    async fn load_bundle(&self) -> Result<Arc<BundleResolver>, TpmResourceEffectError>;

    /// Resolve and consume the owning Guest's lifecycle admission for one
    /// Device start, exactly once per pass.
    ///
    /// The retired adapter resolved the admission lazily (the plane's
    /// internal guest-lifecycle admission for the Device's owning Guest)
    /// and consumed the lease at most once per pass, when the pass reached
    /// the launchable swtpm row; the daemon implementation preserves that
    /// behavior.
    async fn consume_lifecycle_lease(
        &self,
        vm_id: &str,
        operation_id: &str,
    ) -> Result<(), TpmResourceEffectError>;
}