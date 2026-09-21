//! The declared facets the provider-owned GPU effects service reaches
//! daemon state through (U12 gpu step).
//!
//! The GPU family's lifecycle port ([`crate::effects::GpuLifecycleEffectPort`])
//! is implemented by this crate's own effects module (see
//! [`crate::effects_service`]). The daemon state that implementation holds -
//! the Host-global GPU authority index the leases are admitted to and
//! released from - crosses the provider boundary as a declared facet rather
//! than as a daemon handle: every facet here is a type the provider crate
//! declares, an implementation of it is supplied by the daemon host through
//! the composition root (never derived from caller input), and the family
//! crate holds no daemon state type.

use std::sync::Arc;

use d2b_core_controller::authority::{AuthorityLease, AuthorityRequest};

use crate::effects::GpuEffectError;

/// The daemon-supplied facet set the provider-owned GPU effects are built
/// from (U12 gpu step).
///
/// The composition root supplies the object; the port never holds a daemon
/// state type (R2).
#[derive(Clone)]
pub struct GpuEffectFacets {
    /// The daemon's GPU runtime: the Host-global authority index one
    /// Device's leases are admitted to and released from.
    pub runtime: Arc<dyn GpuRuntime>,
}

/// The daemon-hosted GPU runtime one Device's effects run over (U12 gpu
/// step).
///
/// The daemon implements this trait in its composition root. The family
/// crate's effects module builds its port from it; the manager-routed child
/// rows the port reads and retires cross as the toolkit's
/// [`d2b_provider_toolkit::SharedProviderChildSurface`], never as daemon
/// state. The trait is synchronous because the family's controller is
/// synchronous (the daemon implementation drives its async authority index
/// on the runtime captured at construction).
pub trait GpuRuntime: Send + Sync + 'static {
    /// Admit one authority request to the Host-global index and return the
    /// issued lease.
    fn admit_authority(&self, request: AuthorityRequest) -> Result<AuthorityLease, GpuEffectError>;

    /// Release one lease back to the Host-global index.
    fn release_authority(&self, lease: &AuthorityLease) -> Result<(), GpuEffectError>;
}