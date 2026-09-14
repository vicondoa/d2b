//! Daemon-owned child-intent sources for the interaction family.
//!
//! Two interaction types own children whose realization only the daemon can
//! author: a display session's two workers and their private endpoints (the
//! display supervisor's launch material) and an audio binding's worker set
//! (the audio Provider's controller). Each per-type crate declares the port it
//! needs and materializes the returned intents into manager child rows; the
//! daemon implements the ports here, beside the production effects, so no
//! provider crate reaches host state.

use d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildIntent;
use d2b_core_controller::OwnedChildIntent;
use d2b_provider_audio_binding::{AudioBindingChildRequest, AudioBindingChildSource};
use d2b_provider_audio_pipewire::AudioBindingController;
use d2b_provider_wayland_policy::InteractionEffectError;
use d2b_provider_wayland_session::{DisplayChildRequest, DisplayChildSource};

use crate::audio_dispatch::DaemonAudioMediator;

/// The display supervisor's child intents for one session row.
pub(crate) struct ProductionDisplayChildSource;

impl DisplayChildSource for ProductionDisplayChildSource {
    fn display_children(
        &self,
        request: &DisplayChildRequest<'_>,
    ) -> Result<Vec<OwnedChildIntent>, InteractionEffectError> {
        crate::interaction_composition::display_owned_child_intents(
            request.zone,
            request.session_ref,
            request.session_uid,
            request.spec,
            request.process_generation,
            request.controller_generation,
        )
        .map_err(|_| InteractionEffectError::InvalidResource)
    }
}

/// The audio Provider controller's child intents for one binding row.
pub(crate) struct ProductionAudioBindingChildSource;

impl AudioBindingChildSource for ProductionAudioBindingChildSource {
    fn binding_children(
        &self,
        request: &AudioBindingChildRequest<'_>,
    ) -> Result<Vec<BindingChildIntent>, InteractionEffectError> {
        AudioBindingController::<DaemonAudioMediator>::child_resources(
            request.binding_ref,
            request.spec,
        )
        .map(|set| set.iter().cloned().collect())
        .map_err(|_| InteractionEffectError::InvalidResource)
    }
}
