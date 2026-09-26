//! ShellPool registration and behavior boundary: the declaration the plane
//! registers and the reference shapes the driver validates.

use std::sync::Arc;

use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ZoneId};
use d2b_provider_shell_pool::{
    SHELL_POOL_TYPE, ShellPool, shell_pool_descriptor, shell_pool_spec_decoder,
};
use d2b_provider_wayland_policy::{
    InteractionDriverArgs, InteractionDriverEffects, InteractionEffectError,
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
    shell_pool_descriptor(InteractionDriverArgs {
        zone: ZoneId::parse("work").expect("zone"),
        controller_generation: ControllerGeneration::new(3).expect("generation"),
        effects: Arc::new(UnusedEffects),
        behavior: ShellPool,
    })
}

fn pool_spec() -> Value {
    json!({
        "providerRef": "Provider/shell-terminal",
        "executionRef": "Host/host-system",
        "userRef": "User/alice",
        "loginShellRef": "artifact://shell",
    })
}

fn envelope(value: &Value) -> InteractionSpecEnvelope {
    let bytes = serde_json::to_vec(value).expect("spec bytes");
    let decoded = shell_pool_spec_decoder()
        .decode(&bytes)
        .expect("the row decodes");
    *decoded
        .downcast::<InteractionSpecEnvelope>()
        .expect("the decoder yields the family envelope")
}

/// The declaration carries the pool row: the type, its runtime-admitted source
/// mask, and the rows a pool reads.
#[test]
fn the_declaration_serves_the_pool_row() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::SHELL_POOL);
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME
    );
    assert!(!descriptor.exportable);
    assert_eq!(
        descriptor.reads,
        &[WellKnownType::HOST, WellKnownType::GUEST, WellKnownType::USER]
    );
    assert!(descriptor.operations.is_empty() && descriptor.creations.is_empty());
    const { assert!(ShellPool::SPEC_PROVIDER_SELECTOR); };
    assert_eq!(ShellPool::RESOURCE_TYPE, SHELL_POOL_TYPE);
}

/// The registry serves the pool's decoder and factory from the declaration,
/// and a second registration of the type is refused.
#[test]
fn the_registry_serves_the_declaration_and_refuses_a_duplicate() {
    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&descriptor())
        .expect("the declaration registers");
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(SHELL_POOL_TYPE))
    );
    assert!(matches!(
        providers.register_driver(&descriptor()),
        Err(ProviderDirectoryError::DuplicateType(resource_type))
            if resource_type.as_str() == SHELL_POOL_TYPE
    ));
}

/// The pool validates and reads its execution target and user.
#[test]
fn the_pool_reads_its_execution_target_and_user() {
    let envelope = envelope(&pool_spec());
    ShellPool.validate(&envelope).expect("the pool validates");
    assert_eq!(
        ShellPool.dependencies(&envelope).expect("dependencies"),
        vec![
            ResourceRef::parse("Host/host-system").expect("execution"),
            ResourceRef::parse("User/alice").expect("user"),
        ]
    );
    assert!(ShellPool.desired_children(
        &d2b_provider_wayland_policy::InteractionChildContext {
            zone: &d2b_contracts_resource::v3::ZoneId::parse("work").expect("zone"),
            key: &d2b_resource_runtime::identity::ResourceKey::new(
                "work",
                SHELL_POOL_TYPE,
                "work-shell"
            ),
            uid: &[0x42; 16],
            generation: 4,
            controller_generation: 3,
        },
        &envelope
    )
    .expect("no children")
    .is_empty());
}

/// The pool's reference shapes are refused exactly as they were: a user as the
/// execution target, a non-User user, a login shell that is not an artifact
/// reference, and a foreign Provider.
#[test]
fn malformed_references_are_refused() {
    for spec in [
        json!({
            "providerRef": "Provider/shell-terminal",
            "executionRef": "User/alice",
            "userRef": "User/alice",
            "loginShellRef": "artifact://shell",
        }),
        json!({
            "providerRef": "Provider/shell-terminal",
            "executionRef": "Host/host-system",
            "userRef": "Host/host-system",
            "loginShellRef": "artifact://shell",
        }),
        json!({
            "providerRef": "Provider/shell-terminal",
            "executionRef": "Host/host-system",
            "userRef": "User/alice",
            "loginShellRef": "/usr/bin/sh",
        }),
        json!({
            "providerRef": "Provider/other",
            "executionRef": "Host/host-system",
            "userRef": "User/alice",
            "loginShellRef": "artifact://shell",
        }),
    ] {
        assert_eq!(
            ShellPool.validate(&envelope(&spec)),
            Err(InteractionEffectError::InvalidResource),
            "the pool refuses {spec}"
        );
    }
}
