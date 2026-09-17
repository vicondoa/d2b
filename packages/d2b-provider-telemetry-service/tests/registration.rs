//! Telemetry Service registration boundary: the driver declaration is what
//! the plane registers, and the registry serves this type's decoder and
//! factory from it.

use d2b_provider_telemetry_service::{
    TELEMETRY_SERVICE_TYPE, telemetry_service_descriptor, telemetry_service_spec_decoder,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    telemetry_service_descriptor()
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn descriptor_declares_and_registers_the_service_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::TELEMETRY_SERVICE);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        TELEMETRY_SERVICE_TYPE
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve the Zone telemetry authority without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(
        descriptor.exportable,
        "a qualified *.d2bus.org.*Service type is exactly what ResourceExport admits"
    );
    assert!(descriptor.operations.is_empty());
    assert!(
        descriptor.creations.is_empty(),
        "a Service realizes nothing on a target and owns no child"
    );

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(TELEMETRY_SERVICE_TYPE)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(TELEMETRY_SERVICE_TYPE)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(TELEMETRY_SERVICE_TYPE))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), TELEMETRY_SERVICE_TYPE);
    factory
        .create(&ResourceKey::new("dev", TELEMETRY_SERVICE_TYPE, "ingest"))
        .await;
}

/// The declared decoder is the one the registry serves: it decodes the stored
/// spec envelope the manager hands the driver.
#[test]
fn the_declared_decoder_reads_a_stored_service_spec() {
    let decoder = telemetry_service_spec_decoder();
    let spec = serde_json::json!({
        "providerRef": "Provider/observability-otel",
        "serviceRole": "authority",
        "ingestEndpointRefs": ["Endpoint/ingest"],
        "signals": ["metrics"],
        "quota": {},
        "policy": {},
    });
    assert!(decoder.decode(serde_json::to_vec(&spec).unwrap().as_slice()).is_ok());
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
            if type_name.as_str() == TELEMETRY_SERVICE_TYPE
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
            if type_name.as_str() == TELEMETRY_SERVICE_TYPE
    ));
}
