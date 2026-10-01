//! The declared facets the `NetworkBinding` family's driver effects reach
//! host state through.
//!
//! The shared fabric of one Network is realized by the Network provider, once
//! per `(Zone, Network, execution target)`, and every consumer holding a
//! membership on it contributes one interface keyed by its own consumer
//! identity. This crate owns no host state: the join, the drain, the release,
//! and the observation that tells the driver whether a membership is already
//! held all cross the provider boundary as the four declared facets below, so
//! the composition root supplies the objects and the driver holds no daemon
//! type (R2).
//!
//! Each facet answers for the exact [`FabricMembership`] the driver derived
//! from the committed row, so no effect can act on a relationship the row does
//! not name, and releasing one membership can never reach another member's
//! interface or the fabric they still share.

use std::sync::Arc;

use async_trait::async_trait;

use crate::driver::{FabricDrain, FabricMembership, FabricMembershipState, FabricRelease};

/// The daemon-supplied facet set the `NetworkBinding` driver's effects are
/// built from.
///
/// The composition root supplies the objects; the driver builds its effect port
/// from them at construction (R2) and never receives an externally built port.
#[derive(Clone)]
pub struct NetworkBindingEffectFacets {
    /// What the shared fabric currently holds for this consumer's membership.
    pub observe: Arc<dyn FabricObserveSource>,
    /// Joining one consumer's membership on the shared fabric.
    pub join: Arc<dyn FabricJoinSource>,
    /// Blocking new use and driving outstanding use to the safe state.
    pub drain: Arc<dyn FabricDrainSource>,
    /// Removing one membership, retaining the fabric another member still uses.
    pub release: Arc<dyn FabricReleaseSource>,
}

/// The daemon-supplied membership observation: whether the shared fabric
/// currently holds this consumer's membership, which Network generation it was
/// realized under, and whether the presented interface exists yet.
///
/// The observation fails closed: a fabric that cannot answer reports no
/// membership, and the driver then joins rather than assuming one.
#[async_trait]
pub trait FabricObserveSource: Send + Sync + 'static {
    /// What the shared fabric holds for `membership` right now.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter cannot complete the
    /// observation; the pass defers retryably rather than reporting absence.
    async fn observe(&self, membership: &FabricMembership) -> Result<FabricMembershipState, String>;
}

/// The daemon-supplied fabric join (U6): one consumer's membership on the
/// provider-owned shared fabric.
///
/// The join is idempotent under retry: a membership the fabric already holds
/// under the same Network generation keeps its single interface, and the
/// returned state says so. A second join never produces a second interface.
#[async_trait]
pub trait FabricJoinSource: Send + Sync + 'static {
    /// Join `membership` on the shared fabric and report the resulting state.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter fails to realize the
    /// membership; the pass defers retryably and re-joins.
    async fn join(&self, membership: &FabricMembership) -> Result<FabricMembershipState, String>;
}

/// The daemon-supplied drain (U6): block new use of a membership, then drive
/// outstanding use to the safe state before the membership is removed.
#[async_trait]
pub trait FabricDrainSource: Send + Sync + 'static {
    /// Fence and drain `membership`.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter cannot complete the
    /// drain; the teardown retries.
    async fn drain(&self, membership: &FabricMembership) -> Result<FabricDrain, String>;
}

/// The daemon-supplied release (U6): remove one consumer's membership.
///
/// The release is idempotent under retry: a membership that was never joined,
/// or was already released, answers `Ok` with the fabric retained. A fabric
/// another member still uses is never removed (R36).
#[async_trait]
pub trait FabricReleaseSource: Send + Sync + 'static {
    /// Remove `membership` from the shared fabric.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter fails to remove the
    /// membership; the teardown retries.
    async fn release(&self, membership: &FabricMembership) -> Result<FabricRelease, String>;
}
