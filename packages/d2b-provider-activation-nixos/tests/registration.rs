//! Activation family registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.

use std::sync::Arc;

use d2b_contracts_broker::host_generation::HostGenerationHandoffIntent;
use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_activation_nixos::{
    ACTIVATION_RUNNER_CREATION, ACTIVATION_TYPE_NAME, ActivationDriverArgs,
    ActivationDriverEffects, FailClosedActivationVerifier, HostHandoffResult,
    activation_descriptor,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, ChildCustody, WellKnownType};

/// The port instance the declaration carries; the registration boundary never
/// dispatches a handoff.
struct UnusedEffects;

#[async_trait::async_trait]
impl ActivationDriverEffects for UnusedEffects {
    async fn apply_host_generation_handoff(
        &self,
        _target: ResourceRef,
        _intent: HostGenerationHandoffIntent,
    ) -> HostHandoffResult {
        HostHandoffResult::Incomplete
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    activation_descriptor(ActivationDriverArgs {
        zone: "work".to_owned(),
        effects: Arc::new(UnusedEffects),
        verifier: Arc::new(FailClosedActivationVerifier),
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[tokio::test]
async fn descriptor_declares_and_registers_the_generation_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::NIXOS_GENERATION);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        ACTIVATION_TYPE_NAME
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve the activation generations without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no generation is an export subject");
    assert_eq!(
        descriptor.execution,
        &["host", "guest"],
        "a generation carries the canonical executionRef anchor, which admits Host or Guest"
    );
    assert_eq!(
        descriptor.reads,
        &[WellKnownType::NIXOS_GENERATION],
        "the policy reads the prior generation row of the same execution reference"
    );
    assert_eq!(
        descriptor.verbs,
        &[
            "get",
            "list",
            "watch",
            "create",
            "update-spec",
            "update-status",
            "update-metadata",
            "update-finalizers",
            "delete",
        ]
    );
    assert!(descriptor.operations.is_empty());

    // The one child the driver mints is declared, with the Process Provider
    // and the custody the reconcile pass performs.
    assert_eq!(descriptor.creations, &[ACTIVATION_RUNNER_CREATION]);
    assert_eq!(ACTIVATION_RUNNER_CREATION.child, WellKnownType::EPHEMERAL_PROCESS);
    assert_eq!(ACTIVATION_RUNNER_CREATION.provider_ref, "Provider/system-minijail");
    assert_eq!(ACTIVATION_RUNNER_CREATION.custody, ChildCustody::DriverOwned);

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(ACTIVATION_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(ACTIVATION_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(ACTIVATION_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), ACTIVATION_TYPE_NAME);
    factory
        .create(&ResourceKey::new("work", ACTIVATION_TYPE_NAME, "gen-1"))
        .await;
}

/// One driver per resource type: a second registration for the same type is
/// refused without clobbering the first.
#[tokio::test]
async fn a_second_registration_of_the_type_is_refused() {
    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&descriptor())
        .expect("first registration");
    let error = providers
        .register_driver(&descriptor())
        .expect_err("duplicate registration");
    assert!(matches!(
        &error,
        ProviderDirectoryError::DuplicateType(type_name)
            if type_name.as_str() == ACTIVATION_TYPE_NAME
    ));
}

/// The declared mask carries the presence obligation: once the plane is
/// open, this driver can no longer arrive.
#[tokio::test]
async fn the_declaration_cannot_be_registered_after_the_plane_opens() {
    let mut providers = ProviderDirectory::new();
    providers.mark_plane_open();
    let error = providers
        .register_driver(&descriptor())
        .expect_err("late registration");
    assert!(matches!(
        &error,
        ProviderDirectoryError::RequiredBeforeOpen { type_name }
            if type_name.as_str() == ACTIVATION_TYPE_NAME
    ));
}
