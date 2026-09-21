//! The declared facets the provider-owned Activation effects service reaches
//! daemon state through.
//!
//! The Activation family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The one daemon-structural
//! capability the effects need - the dispatch of the preserved
//! host-generation handoff over the daemon's authenticated origination
//! socket, presented as the daemon's admin-uid caller authority - crosses
//! the provider boundary as a declared facet rather than a daemon call: the
//! facet is a type this crate declares, it names exactly the operation the
//! family performs (the typed handoff, never the whole request envelope),
//! the daemon host supplies its implementation through the composition root
//! (never derived from caller input), and the family crate holds no daemon
//! state type.

use std::sync::Arc;

use d2b_contracts_broker::broker_wire::ApplyHostGenerationHandoffResponse;
use d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff;

/// The daemon-supplied facet set the provider-owned Activation effects are
/// built from.
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2). The one facet is the handoff dispatch source: the
/// daemon's own dispatch of one preserved host-generation handoff over its
/// authenticated origination socket, presented as the daemon's admin-uid
/// caller authority - the same authority the retired daemon adapter
/// presented.
#[derive(Clone)]
pub struct ActivationEffectFacets {
    /// The handoff dispatch source: one preserved host-generation handoff
    /// dispatched as the daemon's admin-uid caller authority.
    pub broker: Arc<dyn ActivationBrokerDispatch>,
}

/// The daemon-supplied handoff dispatch source.
///
/// The daemon implements this trait in its composition root over its own
/// dispatch machinery, with the daemon's admin-uid caller role; the family
/// crate receives the typed handoff result, never a daemon function, socket
/// path, or the whole broker request envelope.
pub trait ActivationBrokerDispatch: Send + Sync + 'static {
    /// Dispatch one preserved host-generation handoff as the daemon's
    /// admin-uid caller authority. A failed dispatch is a string error the
    /// family reduces to its closed `Incomplete` result.
    fn dispatch_handoff(
        &self,
        request: ApplyHostGenerationHandoff,
    ) -> Result<ApplyHostGenerationHandoffResponse, String>;
}