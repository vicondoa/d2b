//! Production Volume effects: the layout effect port the Volume driver runs
//! behind, over the preserved `VolumeLocalController`.
//!
//! The controller closure rebuilds the preserved controller over the anchored
//! adapters per call (exactly the old `reconcile_volume` construction) over
//! the per-zone resolver; the durable layout probe recover reads is the
//! volume-local marker an initialized layout left behind. The plane assembles
//! both closures from `ServerState`, the bundle resolver, and the per-zone
//! registry; this module only adapts them onto the family's port.

use std::sync::Arc;

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, volume::VolumeSpec};
use d2b_provider_volume::VolumeDriverEffects;
use d2b_provider_volume_local::{LayoutPhase, VolumeLocalController};

/// Production effects over the preserved `VolumeLocalController`.
pub(crate) struct ProductionVolumeDriverEffects<S, L> {
    controller: Arc<dyn Fn() -> VolumeLocalController<S, L> + Send + Sync>,
    state: Arc<dyn Fn(&ResourceUid) -> bool + Send + Sync>,
}

impl<S, L> ProductionVolumeDriverEffects<S, L> {
    pub(crate) fn new(
        controller: Arc<dyn Fn() -> VolumeLocalController<S, L> + Send + Sync>,
        state: Arc<dyn Fn(&ResourceUid) -> bool + Send + Sync>,
    ) -> Self {
        Self { controller, state }
    }
}

#[async_trait::async_trait]
impl<
    S: d2b_provider_volume_local::VolumeSourceEffectPort + 'static,
    L: d2b_provider_volume_local::VolumeLayoutEffectPort + 'static,
> VolumeDriverEffects for ProductionVolumeDriverEffects<S, L>
{
    async fn ensure_layout(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
        provider: Option<&serde_json::Value>,
        owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String> {
        let report = (self.controller)()
            .reconcile(volume_uid, spec, provider, owner_ref)
            .await
            .map_err(|error| error.to_string())?;
        Ok(report.layout_phase == LayoutPhase::Ready)
    }

    async fn remove_layout(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
    ) -> Result<(), String> {
        (self.controller)()
            .cleanup(volume_uid, spec)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn has_layout(&self, volume_uid: &ResourceUid) -> bool {
        (self.state)(volume_uid)
    }
}
