//! The declared facets the provider-owned `EndpointBinding` effects service
//! reaches daemon state through (U6).
//!
//! The `EndpointBinding` family's driver effects are served by this crate's
//! own implementation (see [`crate::effects_service`]) over the Endpoint
//! family's exact-endpoint realization. The daemon-owned half crosses the
//! provider boundary as declared facets rather than daemon calls: the exact
//! endpoint verification (resolve the named `Endpoint` row's locator
//! privately and report what the kernel applies for this consumer), the
//! delivery of that exact endpoint in the form the row's attachment kind
//! declares, the fence that blocks new use ahead of release, the attachment
//! observation the teardown gate reads, and the release itself.
//!
//! Every facet takes a [`EndpointDeliveryTarget`], which carries committed-row
//! facts only. No facet accepts a host path, a socket name, or a locator:
//! the endpoint's locator stays inside the adapter that resolves it, which is
//! what keeps a consumer from being handed anything but the one inode its
//! relationship admitted.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_endpoint::binding::{
    EndpointAccessObservation, EndpointSocketIdentity,
};

use crate::driver::{EndpointBindingDelivery, EndpointDeliveryTarget};

/// The daemon-supplied facet set the provider-owned `EndpointBinding`
/// effects are built from (U6).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2). The five facets are the exact endpoint
/// verification, the delivery, the pre-drain fence, the attachment
/// observation, and the release.
#[derive(Clone)]
pub struct EndpointBindingEffectFacets {
    /// The exact endpoint verification: what the kernel applies for this
    /// consumer on the one endpoint this relationship names.
    pub verify: Arc<dyn EndpointVerifySource>,
    /// The exact endpoint delivery in the form the row's attachment kind
    /// declares.
    pub deliver: Arc<dyn EndpointDeliverSource>,
    /// The pre-drain fence: block new use for this relationship.
    pub fence: Arc<dyn EndpointFenceSource>,
    /// The attachment observation the teardown gate reads.
    pub attached: Arc<dyn EndpointAttachmentSource>,
    /// The release of one relationship's delivery.
    pub release: Arc<dyn EndpointReleaseSource>,
}

/// The daemon-supplied exact endpoint verification (U6): what the kernel
/// actually applies for one consumer principal on one exact endpoint.
///
/// The evidence is the inode's, never a second channel (U13): the named
/// entry already ANDed with the ACL mask (or the inode's mode class), the
/// AND across every ancestor's effective traverse bit, the pinned `(dev,
/// ino)` the endpoint owner resolved, and whether the endpoint is
/// accepting.
#[async_trait]
pub trait EndpointVerifySource: Send + Sync + 'static {
    /// Observe the exact endpoint for this consumer.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter cannot complete the
    /// observation. The observation itself never fails open: an endpoint the
    /// adapter cannot resolve reports the shortest effective access the
    /// observation contract allows, so the driver refuses it.
    async fn verify(
        &self,
        target: &EndpointDeliveryTarget,
    ) -> Result<EndpointAccessObservation, String>;
}

/// The daemon-supplied exact endpoint delivery (U6): the consumer receives
/// the one inode its relationship admitted, in the declared form.
#[async_trait]
pub trait EndpointDeliverSource: Send + Sync + 'static {
    /// Deliver the exact endpoint and answer the identity the delivery
    /// actually names.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the delivery could not be established. The call is
    /// idempotent under retry: a delivery already in place for this target
    /// answers with the same pinned identity and re-issues nothing.
    async fn deliver(
        &self,
        target: &EndpointDeliveryTarget,
        delivery: &EndpointBindingDelivery,
    ) -> Result<EndpointSocketIdentity, String>;
}

/// The daemon-supplied pre-drain fence (U6): block new use for one
/// relationship ahead of its typed release.
#[async_trait]
pub trait EndpointFenceSource: Send + Sync + 'static {
    /// Block new use of this relationship.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter cannot fence the relationship;
    /// the teardown does not proceed on an unfenced row. Idempotent under
    /// retry.
    async fn fence(&self, target: &EndpointDeliveryTarget) -> Result<(), String>;
}

/// The daemon-supplied attachment observation (U6): whether the consumer
/// still holds the delivered endpoint.
#[async_trait]
pub trait EndpointAttachmentSource: Send + Sync + 'static {
    /// Whether the consumer is still attached to the exact endpoint.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter cannot complete the
    /// observation. The observation fails closed: an answer the adapter
    /// cannot establish is `true` (still attached), never `false`.
    async fn attached(&self, target: &EndpointDeliveryTarget) -> Result<bool, String>;
}

/// The daemon-supplied release (U6): drop one relationship's delivery.
#[async_trait]
pub trait EndpointReleaseSource: Send + Sync + 'static {
    /// Release the delivery.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter fails to release the delivery.
    /// The release is idempotent under retry: a delivery that was never
    /// established, or whose endpoint is already gone, answers `Ok(())`.
    async fn release(&self, target: &EndpointDeliveryTarget) -> Result<(), String>;
}
