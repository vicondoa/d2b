//! AudioBinding registration and behavior boundary: the declaration the plane
//! registers, the row vocabulary the driver validates, and the manager child
//! rows the audio Provider's controller intents materialize into.

use std::sync::Arc;

use d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildIntent;
use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ZoneId};
use d2b_provider_audio_binding::{
    AudioBinding, AudioBindingChildRequest, AudioBindingChildSource,
    audio_binding_descriptor, audio_binding_spec_decoder,
};
use d2b_provider_audio_pipewire::{
    AudioBindingController, AudioBindingSpec, FakeAudioMediator,
};
use d2b_provider_wayland_policy::{
    AUDIO_BINDING_TYPE, InteractionChildContext, InteractionDriverArgs, InteractionDriverEffects,
    InteractionEffectError, InteractionEffectOutcome, InteractionEffectPhase,
    InteractionEffectRequest, InteractionFinalize, InteractionKind, InteractionSpecEnvelope,
    InteractionType,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
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

/// The audio Provider's own child derivation, as the daemon serves it.
struct ProviderChildren;

impl AudioBindingChildSource for ProviderChildren {
    fn binding_children(
        &self,
        request: &AudioBindingChildRequest<'_>,
    ) -> Result<Vec<BindingChildIntent>, InteractionEffectError> {
        AudioBindingController::<FakeAudioMediator>::child_resources(
            request.binding_ref,
            request.spec,
        )
        .map(|set| set.iter().cloned().collect())
        .map_err(|_| InteractionEffectError::InvalidResource)
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    audio_binding_descriptor(InteractionDriverArgs {
        zone: "work".to_owned(),
        controller_generation: ControllerGeneration::new(3).expect("generation"),
        effects: Arc::new(UnusedEffects),
        behavior: AudioBinding::new(Arc::new(ProviderChildren)),
    })
}

fn binding_spec() -> AudioBindingSpec {
    AudioBindingSpec::new(
        ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").expect("service"),
        ResourceRef::parse("Guest/workstation").expect("target"),
        "work",
    )
    .expect("binding spec")
}

fn envelope() -> InteractionSpecEnvelope {
    let bytes = serde_json::to_vec(&serde_json::to_value(binding_spec()).expect("spec json"))
        .expect("spec bytes");
    let decoded = audio_binding_spec_decoder()
        .decode(&bytes)
        .expect("the row decodes");
    *decoded
        .downcast::<InteractionSpecEnvelope>()
        .expect("the decoder yields the family envelope")
}

fn child_context<'a>(
    zone: &'a ZoneId,
    key: &'a ResourceKey,
    uid: &'a [u8; 16],
) -> InteractionChildContext<'a> {
    InteractionChildContext {
        zone,
        key,
        uid,
        generation: 4,
        controller_generation: 3,
    }
}

/// The declaration carries the binding row: the type, its runtime-admitted
/// source mask, and the rows a binding reads.
#[test]
fn the_declaration_serves_the_binding_row() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::AUDIO_BINDING);
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME
    );
    assert!(!descriptor.exportable);
    assert_eq!(
        descriptor.reads,
        &[WellKnownType::AUDIO_SERVICE, WellKnownType::GUEST]
    );
    assert!(descriptor.operations.is_empty() && descriptor.creations.is_empty());
    const { assert!(AudioBinding::SPEC_PROVIDER_SELECTOR); };
    assert_eq!(AudioBinding::RESOURCE_TYPE, AUDIO_BINDING_TYPE);
}

/// The registry serves the binding's decoder and factory from the
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
            .contains_key(&ResourceTypeName::new(AUDIO_BINDING_TYPE))
    );
    assert!(matches!(
        providers.register_driver(&descriptor()),
        Err(ProviderDirectoryError::DuplicateType(resource_type))
            if resource_type.as_str() == AUDIO_BINDING_TYPE
    ));
}

/// The binding validates and reads its service and target rows.
#[test]
fn the_binding_reads_its_service_and_target() {
    let envelope = envelope();
    let behavior = AudioBinding::new(Arc::new(ProviderChildren));
    behavior.validate(&envelope).expect("the binding validates");
    assert_eq!(
        behavior.dependencies(&envelope).expect("dependencies"),
        vec![
            ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").expect("service"),
            ResourceRef::parse("Guest/workstation").expect("target"),
        ]
    );
}

/// The Provider's child intents become manager child rows: the manager owns
/// the child identity, the controller owns the child body.
#[test]
fn the_provider_intents_become_manager_child_rows() {
    let envelope = envelope();
    let behavior = AudioBinding::new(Arc::new(ProviderChildren));
    let zone = ZoneId::parse("work").expect("zone");
    let key = ResourceKey::new("work", AUDIO_BINDING_TYPE, "guest-audio");
    let uid = [0x42u8; 16];
    let children = behavior
        .desired_children(&child_context(&zone, &key, &uid), &envelope)
        .expect("children");

    assert_eq!(children.len(), 4, "the audio binding owns four children");
    assert_eq!(children[0].type_name.as_str(), "Process");
    let spec: Value = serde_json::from_slice(&children[0].spec).expect("child spec");
    assert_eq!(spec["providerRef"], "Provider/system-minijail");
    let metadata: Value = serde_json::from_slice(&children[0].metadata).expect("child metadata");
    assert_eq!(
        metadata["ownerRef"],
        "audio.d2bus.org.AudioBinding/guest-audio"
    );
}

/// A row that is not a binding spec is refused.
#[test]
fn a_foreign_row_is_refused() {
    let bytes = serde_json::to_vec(&json!({"providerRef": "Provider/audio-pipewire"}))
        .expect("spec bytes");
    let envelope = audio_binding_spec_decoder()
        .decode(&bytes)
        .expect("the row decodes");
    let envelope = *envelope
        .downcast::<InteractionSpecEnvelope>()
        .expect("the decoder yields the family envelope");
    let behavior = AudioBinding::new(Arc::new(ProviderChildren));
    assert_eq!(
        behavior.validate(&envelope),
        Err(InteractionEffectError::InvalidResource)
    );
}
