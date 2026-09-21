//! Endpoint family registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.

use d2b_provider_endpoint::{
    ENDPOINT_EFFECTS_SERVICE, EndpointDriverArgs, endpoint_descriptor,
};
use d2b_provider_endpoint::test_support::FakeSocketEffects;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

/// The runtime name of the one type this declaration serves.
fn endpoint_type() -> ResourceTypeName {
    WellKnownType::ENDPOINT.to_resource_type_name()
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    endpoint_descriptor(EndpointDriverArgs {
        zone: "work".to_owned(),
        facets: FakeSocketEffects::new().facet_set(),
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn descriptor_declares_and_registers_the_endpoint_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::ENDPOINT);
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve the converted endpoint shapes without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no endpoint is an export subject");
    assert_eq!(
        descriptor.execution,
        &["host"],
        "an Endpoint row carries no execution anchor, so the plane reconciles it on its Host"
    );
    assert_eq!(
        descriptor.reads,
        &[
            WellKnownType::PROCESS,
            WellKnownType::GUEST,
            WellKnownType::VOLUME_BINDING
        ],
        "the realization reads the producer rows and the binding's socket target"
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
    assert!(descriptor.creations.is_empty());
    assert_eq!(
        descriptor.services,
        &[ENDPOINT_EFFECTS_SERVICE],
        "the family's declared effects service rides the declaration (U6)"
    );

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(providers.registered_types(), vec![endpoint_type()]);
    assert!(
        providers.decoders().contains_key(&endpoint_type()),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&endpoint_type())
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0], endpoint_type());
    factory
        .create(&ResourceKey::new("work", "Endpoint", "endpoint"))
        .await;
}

/// One driver per resource type: a second registration for the same type is
/// refused without clobbering the first.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
            if type_name == &endpoint_type()
    ));
}

/// The declared mask carries the presence obligation: once the plane is
/// open, this driver can no longer arrive.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
            if type_name == &endpoint_type()
    ));
}