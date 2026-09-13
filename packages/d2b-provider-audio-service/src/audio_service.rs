//! The `AudioService` resource type: the audio backing a Guest binds to.
//!
//! A service row names the host realization the audio Provider owns (its
//! effect endpoint) and the grants the Zone admits; bindings reference it. The
//! service realizes nothing through resource rows of its own, so its desired
//! child set is empty and its teardown refuses while a binding still
//! references it - that refusal is the Provider's, served through the family
//! effect port.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_audio_pipewire::{AUDIO_REPAIR_INTERVAL_SECS, AudioServiceSpec};
use d2b_resource_runtime::context::{ChildEnsure, SpecDecoder};
use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};

use d2b_provider_wayland_policy::interaction::{
    InteractionChildContext, InteractionDriver, InteractionDriverArgs, InteractionDriverFactory,
    InteractionEffectError, InteractionKind, InteractionSpecEnvelope, InteractionType,
    spec_decoder,
};

/// The canonical ResourceType name of an audio service.
pub const AUDIO_SERVICE_TYPE: &str = "audio.d2bus.org.AudioService";

/// The Provider reference the type's rows select.
pub const AUDIO_SERVICE_PROVIDER_REF: &str = d2b_provider_audio_pipewire::PROVIDER_REF;

/// The preserved reconcile resync cadence of the type.
pub const AUDIO_SERVICE_RESYNC: Duration = Duration::from_secs(AUDIO_REPAIR_INTERVAL_SECS);

/// The `AudioService` driver behavior and declaration.
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioService;

impl InteractionType for AudioService {
    const KIND: InteractionKind = InteractionKind::AudioService;
    const RESOURCE_TYPE: &'static str = AUDIO_SERVICE_TYPE;
    const PROVIDER_REF: &'static str = AUDIO_SERVICE_PROVIDER_REF;
    /// The audio Provider's typed specs carry the universal `providerRef`
    /// themselves, so the row must select this Provider.
    const SPEC_PROVIDER_SELECTOR: bool = true;

    fn resync(&self) -> Duration {
        AUDIO_SERVICE_RESYNC
    }

    /// The service spec decodes with its typed `providerRef` re-inserted.
    fn validate(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<(), InteractionEffectError> {
        envelope.spec_with_provider_ref::<AudioServiceSpec>().map(|_| ())
    }

    /// A service reads nothing while reconciling.
    fn dependencies(
        &self,
        _envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ResourceRef>, InteractionEffectError> {
        Ok(Vec::new())
    }

    /// A service realizes nothing through resource rows.
    fn desired_children(
        &self,
        _children: &InteractionChildContext<'_>,
        _envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ChildEnsure>, InteractionEffectError> {
        Ok(Vec::new())
    }
}

/// The driver for one `AudioService` row.
pub type AudioServiceDriver = InteractionDriver<AudioService>;

/// The factory the registry serves for `AudioService`.
pub type AudioServiceFactory = InteractionDriverFactory<AudioService>;

/// The manager-wired decode hook for `AudioService` rows.
pub fn audio_service_spec_decoder() -> Arc<dyn SpecDecoder> {
    spec_decoder()
}

/// The `AudioService` driver declaration.
///
/// The registry keys the type by this declaration, so the type reaches the
/// plane only through it.
pub fn audio_service_descriptor(args: InteractionDriverArgs<AudioService>) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::AUDIO_SERVICE,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME,
        verbs: d2b_provider_wayland_policy::INTERACTION_VERBS,
        execution: d2b_provider_wayland_policy::INTERACTION_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: audio_service_spec_decoder(),
        factory: Arc::new(AudioServiceFactory::new(args)),
    }
}
