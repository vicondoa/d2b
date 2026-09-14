//! Telemetry Binding registration boundary: the driver declaration is what
//! the plane registers, and the registry serves this type's decoder and
//! factory from it.

use d2b_provider_telemetry_binding::{
    TELEMETRY_BINDING_COLLECTOR_CREATION, TELEMETRY_BINDING_ENDPOINT_CREATION,
    TELEMETRY_BINDING_TYPE, telemetry_binding_descriptor,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    telemetry_binding_descriptor()
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[tokio::test]
async fn descriptor_declares_and_registers_the_binding_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::TELEMETRY_BINDING);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        TELEMETRY_BINDING_TYPE
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve the Zone telemetry producers without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no binding is an export subject");
    assert!(descriptor.operations.is_empty());

    assert!(
        TELEMETRY_BINDING_COLLECTOR_CREATION.order < TELEMETRY_BINDING_ENDPOINT_CREATION.order,
        "an Endpoint is produced by the worker Process the declaration names, so it follows it"
    );

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(TELEMETRY_BINDING_TYPE)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(TELEMETRY_BINDING_TYPE)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(TELEMETRY_BINDING_TYPE))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), TELEMETRY_BINDING_TYPE);
    factory
        .create(&ResourceKey::new("dev", TELEMETRY_BINDING_TYPE, "metrics"))
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
            if type_name.as_str() == TELEMETRY_BINDING_TYPE
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
            if type_name.as_str() == TELEMETRY_BINDING_TYPE
    ));
}
