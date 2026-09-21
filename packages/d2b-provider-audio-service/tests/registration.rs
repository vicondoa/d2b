//! AudioService registration and behavior boundary: the declaration the plane
//! registers and the row vocabulary the driver validates.

use std::sync::Arc;

use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef};
use d2b_provider_audio_pipewire::AudioServiceSpec;
use d2b_provider_audio_service::{
    AudioService, audio_service_descriptor, audio_service_spec_decoder,
};
use d2b_provider_wayland_policy::{
    AUDIO_SERVICE_TYPE, InteractionDriverArgs, InteractionDriverEffects, InteractionEffectError,
    InteractionEffectOutcome, InteractionEffectPhase, InteractionEffectRequest,
    InteractionFinalize, InteractionKind, InteractionSpecEnvelope, InteractionType,
};
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};
use serde_json::{Value, json};

/// The port instance the declaration carries; the boundary never runs an
/// effect.
struct UnusedEffects;

#[async_trait::async_trait]
impl InteractionDriverEffects for UnusedEffects {
    async fn reconcile(
        &self,
        _kind: InteractionKind,
        _request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        Ok(InteractionEffectOutcome::phase(InteractionEffectPhase::Pending))
    }

    async fn finalize(
        &self,
        _kind: InteractionKind,
        _request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionFinalize, InteractionEffectError> {
        Ok(InteractionFinalize::Complete)
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    audio_service_descriptor(InteractionDriverArgs {
        zone: "work".to_owned(),
        controller_generation: ControllerGeneration::new(3).expect("generation"),
        effects: Arc::new(UnusedEffects),
        behavior: AudioService,
    })
}

/// One stored service row, as the audio Provider authors it.
fn service_value() -> Value {
    let spec = AudioServiceSpec::owner(
        ResourceRef::parse("Endpoint/audio-host").expect("endpoint"),
        "work",
    )
    .expect("service spec");
    serde_json::to_value(&spec).expect("spec json")
}

fn envelope(value: &Value) -> InteractionSpecEnvelope {
    let bytes = serde_json::to_vec(value).expect("spec bytes");
    let decoded = audio_service_spec_decoder()
        .decode(&bytes)
        .expect("the row decodes");
    *decoded
        .downcast::<InteractionSpecEnvelope>()
        .expect("the decoder yields the family envelope")
}

/// The declaration carries the service row and the row's Provider selector is
/// required: the typed audio spec carries the universal `providerRef`.
#[test]
fn the_declaration_serves_the_audio_service_row() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::AUDIO_SERVICE);
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME
    );
    assert!(!descriptor.exportable);
    assert!(descriptor.reads.is_empty(), "a service reads nothing");
    assert!(descriptor.operations.is_empty() && descriptor.creations.is_empty());
    assert_eq!(AudioService::RESOURCE_TYPE, AUDIO_SERVICE_TYPE);
    const { assert!(AudioService::SPEC_PROVIDER_SELECTOR, "the row must select Provider/audio-pipewire"); };
    assert_eq!(AudioService::PROVIDER_REF, "Provider/audio-pipewire");
}

/// The registry serves the service's decoder and factory from the
/// declaration, and a second registration of the type is refused.
#[test]
fn the_registry_serves_the_declaration_and_refuses_a_duplicate() {
    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&descriptor())
        .expect("the declaration registers");
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(AUDIO_SERVICE_TYPE))
    );
    assert!(matches!(
        providers.register_driver(&descriptor()),
        Err(ProviderDirectoryError::DuplicateType(resource_type))
            if resource_type.as_str() == AUDIO_SERVICE_TYPE
    ));
}

/// The service's typed spec decodes and it reads nothing.
#[test]
fn the_service_row_decodes_and_reads_nothing() {
    let envelope = envelope(&service_value());
    assert_eq!(envelope.provider_ref(), Some("Provider/audio-pipewire"));
    AudioService.validate(&envelope).expect("the service validates");
    assert!(AudioService.dependencies(&envelope).expect("none").is_empty());
    assert!(
        AudioService
            .desired_children(
                &d2b_provider_wayland_policy::InteractionChildContext {
                    zone: &d2b_contracts_resource::v3::ZoneId::parse("work").expect("zone"),
                    key: &d2b_resource_runtime::identity::ResourceKey::new(
                        "work",
                        AUDIO_SERVICE_TYPE,
                        "host-audio"
                    ),
                    uid: &[0x42; 16],
                    generation: 4,
                    controller_generation: 3,
                },
                &envelope
            )
            .expect("no children")
            .is_empty()
    );
}

/// A row that is not a service spec is refused.
#[test]
fn a_foreign_row_is_refused() {
    let envelope = envelope(&json!({"providerRef": "Provider/audio-pipewire"}));
    assert_eq!(
        AudioService.validate(&envelope),
        Err(InteractionEffectError::InvalidResource)
    );
    assert!(audio_service_spec_decoder().decode(b"[]").is_err());
}
