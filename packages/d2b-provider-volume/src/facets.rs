//! The declared facets the provider-owned Volume effects service reaches
//! daemon state through (U7).
//!
//! The Volume family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the trusted root resolver that anchors one
//! Volume's layout root through the daemon's bundle and state roots, and
//! the durable layout probe recover reads - crosses the provider boundary
//! as declared facets rather than as a daemon handle: every facet here is a
//! type the provider crate declares, an implementation of it is supplied by
//! the daemon host through the composition root (never derived from caller
//! input), and the family crate holds no daemon state type.
//!
//! One daemon-supplied runtime value backs both surfaces:
//!
//! - [`VolumeRuntime`] is the orchestration facade the driver effects
//!   delegate to: the daemon implements the reconcile and cleanup machinery
//!   over the anchored adapters ([`d2b_provider_volume_local::adapter`])
//!   and its own trusted root resolver, and the durable layout probe
//!   (`has_layout`) the driver's recover reads.
//! - the anchored-fd filesystem implementation itself lives in
//!   `d2b-provider-volume-local` beside the effect ports it implements, so
//!   the daemon holds no volume-local mutation code.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, volume::VolumeSpec};

/// The daemon-supplied facet set the provider-owned Volume effects are
/// built from (U7).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct VolumeEffectFacets {
    /// The daemon's Volume runtime: the orchestration the driver effects
    /// delegate to, over the daemon's own trusted root resolver and durable
    /// layout state.
    pub runtime: Arc<dyn VolumeRuntime>,
}

/// The daemon-hosted Volume runtime one zone's effects run over (U7).
///
/// The daemon implements this trait in its composition root (the same
/// controller construction the retired daemon adapter served), supplying
/// the trusted root resolver and the durable layout probe. The family
/// crate's effects service delegates the driver seam to it; the anchored
/// filesystem mutations themselves run through
/// [`d2b_provider_volume_local::adapter::AnchoredVolumeEffectAdapter`],
/// which the runtime constructs over its resolver.
#[async_trait]
pub trait VolumeRuntime: Send + Sync + 'static {
    /// Run the preserved volume-local layout reconcile and report whether
    /// the layout phase reached `Ready`.
    async fn reconcile_volume(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
        provider: Option<&serde_json::Value>,
        owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String>;

    /// Remove the Volume's own layout state (drain finalizer preserved);
    /// idempotent under retry (R10).
    async fn cleanup_volume(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
    ) -> Result<(), String>;

    /// Discover existing volume-local layout state for this exact uid
    /// (recover probe).
    fn has_layout(&self, volume_uid: &ResourceUid) -> bool;
}