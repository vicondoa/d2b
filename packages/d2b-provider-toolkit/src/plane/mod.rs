//! The zone-plane integration surface a provider attaches through.
//!
//! A provider declares its own plane adapters, storage roots, principals,
//! and services ([`crate::declaration`]); the plane calls
//! [`crate::ProviderBase::attach`] with a [`ZonePlaneHandle`] that
//! carries those declared facts and the plane's own port. The toolkit owns
//! the order - adapters in declared dependency order - and never performs an
//! effect itself: every host-side action goes through the port, which the
//! composition root implements.
//!
//! The port is deliberately small. A provider can only ask its zone plane to
//! claim a declared storage root, deploy a declared adapter, or publish a
//! declared service, so a provider cannot reach any host state its own
//! declaration did not name.

mod creations;
mod handle;
mod reconcile;

pub use creations::{ChildCreationFailure, ChildCreationFence, CreationRefusal, CreationTable};
pub use handle::{
    DrainDeadline, MAX_DRAIN_BUDGET_MS, PlaneError, UnavailablePlanePort, ZonePlaneHandle,
    ZonePlanePort,
};
pub use reconcile::{
    CreateChild, MAX_REQUEUE_AFTER_MS, ReconcileCause, ReconcileCtx, ReconcileOutcome,
    ReconcileRefusal, ReconcileTarget,
};

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The toolkit's clock seam.
///
/// Every deadline the base hands a provider is measured against this clock,
/// so a test can drive drain and requeue behavior without waiting on wall
/// time. The production clock is [`SystemClock`].
pub trait Clock: Send + Sync + 'static {
    /// The current time as milliseconds since the Unix epoch.
    fn now_unix_ms(&self) -> u64;
}

/// The wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default()
    }
}

/// A shared clock handle.
pub type SharedClock = Arc<dyn Clock>;
