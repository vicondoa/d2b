//! The declared facets the provider-owned Activation effects service reaches
//! daemon state through.
//!
//! The Activation family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The one daemon-structural
//! capability the effects need - the dispatch of a preserved broker request
//! over the daemon's authenticated origination socket, presented as the
//! daemon's admin-uid caller authority - crosses the provider boundary as a
//! declared facet rather than a daemon call: the facet is a type this crate
//! declares, the daemon host supplies its implementation through the
//! composition root (never derived from caller input), and the family crate
//! holds no daemon state type.

use std::sync::Arc;

use d2b_contracts_broker::broker_wire::{BrokerRequest, BrokerResponse};

/// The daemon-supplied facet set the provider-owned Activation effects are
/// built from.
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2). The one facet is the broker dispatch source: the
/// daemon's own dispatch over its authenticated origination socket, presented
/// as the daemon's admin-uid caller authority - the same authority the
/// retired daemon adapter presented.
#[derive(Clone)]
pub struct ActivationEffectFacets {
    /// The broker dispatch source: one preserved broker request dispatched
    /// as the daemon's admin-uid caller authority.
    pub broker: Arc<dyn ActivationBrokerDispatch>,
}

/// The daemon-supplied broker dispatch source.
///
/// The daemon implements this trait in its composition root over its own
/// dispatch machinery, with the daemon's admin-uid caller role; the family
/// crate receives the dispatch result, never a daemon function or socket
/// path.
pub trait ActivationBrokerDispatch: Send + Sync + 'static {
    /// Dispatch one broker request as the daemon's admin-uid caller
    /// authority. A failed dispatch is a string error the family reduces to
    /// its closed `Incomplete` result.
    fn dispatch(&self, request: BrokerRequest) -> Result<BrokerResponse, String>;
}