//! The declared facets the provider-owned security-key effects service
//! reaches daemon state through (U12 security-key step).
//!
//! The security-key family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the reconcile and finalize orchestration over the
//! daemon's admission, child rows, and readiness state - crosses the
//! provider boundary as a declared facet rather than as a daemon handle:
//! every facet here is a type the provider crate declares, an
//! implementation of it is supplied by the daemon host through the
//! composition root (never derived from caller input), and the family crate
//! holds no daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};

use crate::driver::SecurityKeyComponent;

/// The daemon-supplied facet set the provider-owned security-key effects are
/// built from (U12 security-key step).
///
/// The composition root supplies the object; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct SecurityKeyEffectFacets {
    /// The daemon's security-key runtime: the orchestration the driver
    /// effects delegate to, over the daemon's own admission, child rows,
    /// and readiness state.
    pub runtime: Arc<dyn SecurityKeyRuntime>,
}

/// The daemon-hosted security-key runtime one zone's effects run over (U12
/// security-key step).
///
/// The daemon implements this trait in its composition root (the same
/// shared-provider effects adapter that serves the other shared families),
/// supplying the reconcile/finalize orchestration over the daemon's own
/// admission, child rows, and readiness state. The family crate's effects
/// service delegates the driver seam to it.
#[async_trait]
pub trait SecurityKeyRuntime: Send + Sync + 'static {
    /// Reconcile one security-key Service or Binding row through the
    /// family's typed lifecycle controller.
    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one security-key row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}