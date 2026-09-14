//! WaylandPolicy registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.

use std::sync::Arc;

use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef};
use d2b_provider_wayland_policy::{
    InteractionDriverArgs, InteractionDriverEffects, InteractionEffectError,
    InteractionEffectOutcome, InteractionEffectPhase, InteractionEffectRequest,
    InteractionFinalize, InteractionKind, InteractionSpecEnvelope, InteractionType,
    WAYLAND_POLICY_PROVIDER_REF, WAYLAND_POLICY_RESYNC, WAYLAND_POLICY_TYPE, WaylandPolicy,
    wayland_policy_descriptor, wayland_policy_spec_decoder,
};
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

/// The port instance the declaration carries; the registration boundary never
/// runs an effect.
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
    wayland_policy_descriptor(InteractionDriverArgs {
        zone: "work".to_owned(),
        controller_generation: ControllerGeneration::new(3).expect("generation"),
        effects: Arc::new(UnusedEffects),
        behavior: WaylandPolicy,
    })
}

fn envelope(bytes: &[u8]) -> InteractionSpecEnvelope {
    let decoded = wayland_policy_spec_decoder()
        .decode(bytes)
        .expect("the row decodes");
    *decoded
        .downcast::<InteractionSpecEnvelope>()
        .expect("the decoder yields the family envelope")
}

/// The declaration carries the row every interaction type is registered by.
#[test]
fn the_declaration_serves_the_display_policy_row() {
    let descriptor = descriptor();
    assert_eq!(
        descriptor.resource_type,
        WellKnownType::WAYLAND_POLICY,
        "the declaration keys the type by its well-known name"
    );
    assert_eq!(WaylandPolicy::RESOURCE_TYPE, WAYLAND_POLICY_TYPE);
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME,
        "a host session capability may arrive late, so the mask admits runtime registration"
    );
    assert!(!descriptor.exportable, "a display policy is never exported");
    assert_eq!(descriptor.execution, &["host", "guest"]);
    assert!(descriptor.reads.is_empty(), "a policy reads nothing");
    assert!(descriptor.operations.is_empty());
    assert!(descriptor.creations.is_empty());
    assert_eq!(WAYLAND_POLICY_RESYNC.as_secs(), 30);
    assert_eq!(WAYLAND_POLICY_PROVIDER_REF, "Provider/display-wayland");
}

/// The registry serves the type's decoder and factory from the declaration,
/// and a second registration of the same type is refused.
#[test]
fn the_registry_serves_the_declaration_and_refuses_a_duplicate() {
    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&descriptor())
        .expect("the declaration registers");
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(WAYLAND_POLICY_TYPE)),
        "the decoder rides the declaration"
    );
    assert!(matches!(
        providers.register_driver(&descriptor()),
        Err(ProviderDirectoryError::DuplicateType(resource_type))
            if resource_type.as_str() == WAYLAND_POLICY_TYPE
    ));
}

/// The policy envelope is the whole contract: any JSON object validates, and
/// the decode refuses everything else.
#[test]
fn the_policy_envelope_is_the_whole_contract() {
    let row = envelope(br#"{"providerRef":"Provider/display-wayland","crossZone":false}"#);
    assert_eq!(row.provider_ref(), Some("Provider/display-wayland"));
    assert!(WaylandPolicy.validate(&row).is_ok());
    assert!(
        WaylandPolicy
            .dependencies(&row)
            .expect("no dependencies")
            .is_empty()
    );
    assert!(wayland_policy_spec_decoder().decode(b"[]").is_err());
    let _ = ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/policy");
}
