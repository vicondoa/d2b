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
use d2b_provider_audio_pipewire::{
    AUDIO_CAPTURE_OPERATION, AUDIO_DECLARED_METHODS, AUDIO_PLAYBACK_OPERATION,
    AUDIO_REPAIR_INTERVAL_SECS, AudioServiceSpec, service_declares_channel, validate_audio_service,
};
use d2b_resource_runtime::context::{ChildEnsure, SpecDecoder};
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};

use d2b_provider_wayland_policy::{
    AUDIO_SERVICE_TYPE,
    interaction::{
        InteractionChildContext, InteractionDriverArgs,
        InteractionDriverFactory, InteractionEffectError, InteractionKind,
        InteractionSpecEnvelope, InteractionType, spec_decoder,
    },
};

/// The Provider reference the type's rows select.
pub const AUDIO_SERVICE_PROVIDER_REF: &str = d2b_provider_audio_pipewire::PROVIDER_REF;

/// The preserved reconcile resync cadence of the type.
pub const AUDIO_SERVICE_RESYNC: Duration = Duration::from_secs(AUDIO_REPAIR_INTERVAL_SECS);

/// The closed operation vocabulary the audio family declares for its
/// Service.
///
/// One entry per stream direction, and one direction per declared method
/// pair: a Service row that names anything outside this set is refused at
/// admission, so the set is the single declared source of what an
/// `AudioService` can be asked to do and no second table restates it.
pub const AUDIO_SERVICE_OPERATIONS: [&str; 2] =
    [AUDIO_PLAYBACK_OPERATION, AUDIO_CAPTURE_OPERATION];

/// Whether one committed Service row declares one provider operation.
pub fn service_declares_operation(spec: &AudioServiceSpec, operation: &str) -> bool {
    spec.operations.iter().any(|declared| declared == operation)
}

/// Every declared method the Service row's operations authorize.
///
/// The Service's own declaration is derived from its operations, so the
/// methods a caller may invoke and the operations the row committed cannot
/// drift apart.
pub fn service_declared_methods(spec: &AudioServiceSpec) -> Vec<&'static str> {
    AUDIO_DECLARED_METHODS
        .iter()
        .map(|method| method.name())
        .filter(|name| {
            let channel = AUDIO_DECLARED_METHODS
                .iter()
                .find(|candidate| candidate.name() == *name)
                .map(|candidate| candidate.channel())
                .expect("the method came from the declared vocabulary");
            service_declares_channel(spec, channel)
        })
        .collect()
}

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

    /// The service spec decodes with its typed `providerRef` re-inserted and
    /// every operation it names is one the audio family declares.
    ///
    /// A row that names an operation outside the declared vocabulary would
    /// otherwise describe a capability nothing in this family implements.
    fn validate(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<(), InteractionEffectError> {
        let spec = envelope.spec_with_provider_ref::<AudioServiceSpec>()?;
        validate_audio_service(&spec).map_err(|error| {
            InteractionEffectError::InvalidSpec(error.to_string())
        })?;
        if spec.operations.is_empty()
            || !spec
                .operations
                .iter()
                .all(|operation| AUDIO_SERVICE_OPERATIONS.contains(&operation.as_str()))
        {
            return Err(InteractionEffectError::InvalidSpec(
                "audio-service operation is outside the declared vocabulary".to_owned(),
            ));
        }
        Ok(())
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
        verbs: CONVERTED_TYPE_VERBS,
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
