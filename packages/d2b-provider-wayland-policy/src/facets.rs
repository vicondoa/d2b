//! The declared facets the provider-owned interaction effects service reaches
//! daemon state through (U12).
//!
//! The interaction family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]) over the committed
//! interaction identity, the zone's resource plane, and the audio mediator
//! source. The reader plan: the effects read row state through the generic
//! plane read facet (the daemon's manager client), the committed identity
//! through the identity source (the daemon's authority), and the broker-backed
//! audio mediator through the mediator source (the daemon's mediator
//! construction) - every daemon-structural read crosses the provider boundary
//! as a declared facet, never a daemon handle, and never derived from caller
//! input.
//!
//! The per-zone audio controller registry is shared state this crate owns; the
//! facets constructor creates it once per Zone, so every effects value built
//! from the facet set - the six drivers' shared port and the hosted service -
//! reconciles the same registry.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_provider_audio_pipewire::AudioMediator;
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::manager::{ResourceSelector, ResourceView};

use crate::audio_registry::AudioEffectRegistry;

/// The committed interaction identity subset the family's display-session
/// admission reads.
///
/// The daemon resolves the full committed identity from its durable Zone
/// authority and hands the effects the bounded subset this crate declares:
/// the WaylandSession row identity and the subject / host / user references
/// the session spec must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionEffectIdentity {
    /// The WaylandSession row reference the family's effects admit.
    pub wayland_session_ref: ResourceRef,
    /// The WaylandSession row's durable uid.
    pub wayland_session_uid: ResourceUid,
    /// The Guest row the session spec must name.
    pub subject_ref: ResourceRef,
    /// The Host execution target the session spec must name.
    pub host_execution_ref: ResourceRef,
    /// The User row the session spec must name.
    pub user_ref: ResourceRef,
}

/// The daemon-supplied committed interaction identity source.
///
/// The daemon implements this over its durable Zone authority and resolves the
/// identity at call time, so a Zone whose authority has not committed an
/// interaction identity yet keeps the committed semantics the old effects had
/// (the admission defers, never guesses).
#[async_trait]
pub trait InteractionIdentitySource: Send + Sync + 'static {
    /// The current committed interaction identity for the Zone, when the
    /// authority retains one.
    async fn identity(&self) -> Option<InteractionEffectIdentity>;
}

/// The daemon-supplied plane read source: the bounded row reads the effects
/// make against the Zone's manager.
///
/// The daemon implements this over its per-Zone resource plane; the effects
/// reach row state through it, never through a daemon state handle. A read
/// the plane cannot serve maps to the effects' `Unavailable` refusal, exactly
/// as the daemon implementation did before the move.
#[async_trait]
pub trait InteractionPlaneRead: Send + Sync + 'static {
    /// One manager view by key, or `None` when the plane holds no such row.
    async fn get(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ()>;
    /// Every manager view matching the selector.
    async fn list(&self, selector: &ResourceSelector) -> Result<Vec<ResourceView>, ()>;
}

/// The daemon-supplied audio mediator source for one binding target.
///
/// The daemon implements this over its broker-backed mediator and the target
/// capability row; a target with no audio capability reports `None`, and the
/// binding publishes its degraded/unavailable status exactly as the daemon
/// registry did before the move.
pub trait AudioMediatorSource: Send + Sync + 'static {
    /// Build the production mediator for one binding's target, or report that
    /// the target carries no audio capability.
    fn build(&self, vm_name: &str, projection: bool) -> Option<Box<dyn AudioMediator>>;
}

/// The daemon-supplied facet set the interaction family's effects are built
/// from (U12).
///
/// Every facet is a provider-declared trait object or plain value the daemon
/// host supplies through the composition root; none is derived from caller
/// input, and none is a daemon state type. The per-zone audio registry lives
/// behind the facet set, so the shared effects port and the hosted service
/// reconcile the same controller state.
#[derive(Clone)]
pub struct InteractionEffectFacets {
    zone: ZoneId,
    identity: Arc<dyn InteractionIdentitySource>,
    plane: Arc<dyn InteractionPlaneRead>,
    audio_registry: Arc<AudioEffectRegistry>,
}

impl InteractionEffectFacets {
    /// Build the facet set for one Zone: the daemon-supplied identity, plane,
    /// and audio sources beside the crate-owned per-zone audio registry.
    pub fn new(
        zone: ZoneId,
        identity: Arc<dyn InteractionIdentitySource>,
        plane: Arc<dyn InteractionPlaneRead>,
        audio: Arc<dyn AudioMediatorSource>,
    ) -> Self {
        let audio_registry = Arc::new(AudioEffectRegistry::new(zone.clone(), Arc::clone(&audio)));
        Self {
            zone,
            identity,
            plane,
            audio_registry,
        }
    }

    /// The Zone this facet set serves.
    pub(crate) fn zone(&self) -> &ZoneId {
        &self.zone
    }

    pub(crate) fn identity(&self) -> &Arc<dyn InteractionIdentitySource> {
        &self.identity
    }

    pub(crate) fn plane(&self) -> &Arc<dyn InteractionPlaneRead> {
        &self.plane
    }

    pub(crate) fn audio_registry(&self) -> &Arc<AudioEffectRegistry> {
        &self.audio_registry
    }
}
