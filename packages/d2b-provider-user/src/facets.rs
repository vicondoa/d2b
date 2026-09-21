//! The declared facets the provider-owned User effects service reaches
//! daemon state through (U5).
//!
//! The User family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]) over the bounded local
//! account probe this crate owns (see [`crate::probe`]). The facet set
//! carries the probe the effects reconcile over: the composition root
//! builds it from the crate's own probe ([`UserEffectFacets::production`]),
//! and tests build it from the scripted probe ([`crate::test_support`]),
//! so composition and scripting cross the same
//! [`UserDiscoveryEffectPort`] boundary. Every probe input is host state
//! the crate reads itself; there is no daemon-structural read, so the
//! probe is the whole facet set (R2).

use std::sync::Arc;

use d2b_provider_system_core::UserDiscoveryEffectPort;

use crate::probe::UserProbe;

/// The daemon-supplied facet set the provider-owned User effects are built
/// from (U5).
///
/// The composition root supplies the set; the driver never holds a daemon
/// state type (R2). The set carries the bounded local-account probe the
/// family's effects reconcile over: production builds it from the crate's
/// own probe ([`UserEffectFacets::production`]), and tests build it from
/// the scripted probe, so nobody constructs a probe at the composition
/// site (the family reads host state itself).
#[derive(Clone)]
pub struct UserEffectFacets {
    /// The bounded local-account probe the family's effects reconcile over.
    pub probe: Arc<dyn UserDiscoveryEffectPort>,
}

impl UserEffectFacets {
    /// Build the production facet set: the crate's own bounded NSS probe.
    ///
    /// Every probe input is host state this crate reads itself, so the
    /// construction site supplies no externally built port (R2).
    pub fn production() -> Self {
        Self {
            probe: Arc::new(UserProbe),
        }
    }
}
