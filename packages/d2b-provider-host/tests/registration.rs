//! Host family registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.

use std::sync::Arc;

use d2b_contracts_resource::v3::{
    ResourceRef, ResourceSpec,
    execution_policy::to_base_object,
    host::{HOST_PROVIDER_REF, HostSpec},
};
use d2b_provider_host::{HostDriverEffects, host_descriptor};
use d2b_provider_system_core::HostObservationReport;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{DriverRegistration, ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

/// The port instance the declaration carries; the registration boundary never
/// observes a host.
struct UnusedEffects;

#[async_trait::async_trait]
impl HostDriverEffects for UnusedEffects {
    async fn observe_host(
        &self,
        _host_ref: &ResourceRef,
        _provider_ref: &ResourceRef,
        _spec: &HostSpec,
    ) -> Result<HostObservationReport, String> {
        Err("registration boundary observes no host".to_owned())
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    host_descriptor(Arc::new(UnusedEffects))
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[tokio::test]
async fn descriptor_declares_and_registers_the_host_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::HOST);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        "Host"
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve the bootstrap Host rows without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no Host is an export subject");
    assert_eq!(
        descriptor.execution,
        &["host"],
        "a Host names no execution anchor: it is reconciled on its own Host"
    );
    assert!(
        descriptor.reads.is_empty(),
        "the observation reaches the local machine through the effect port"
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

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new("Host")]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new("Host")),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new("Host"))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), "Host");
    factory
        .create(&ResourceKey::new("work", "Host", "host-system"))
        .await;
}

/// The decoder a descriptor carries is the one the manager wires for the
/// type: it decodes a stored Host row envelope and refuses bytes that are not
/// a resource spec.
#[test]
fn the_declared_decoder_decodes_the_stored_host_envelope() {
    let base = to_base_object(&HostSpec::system_default()).expect("host base");
    let envelope = ResourceSpec::new(
        Some(ResourceRef::parse(HOST_PROVIDER_REF).expect("provider ref")),
        None,
        base,
        None,
    )
    .expect("admitted resource spec");

    let decoder = DriverRegistration::decoder(&descriptor());
    decoder
        .decode(&envelope.canonical_bytes().expect("canonical bytes"))
        .expect("a stored Host row envelope decodes");
    assert!(
        decoder.decode(b"{not-json").is_err(),
        "an unreadable envelope is refused"
    );
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
        ProviderDirectoryError::DuplicateType(type_name) if type_name.as_str() == "Host"
    ));
}

/// The declared mask carries the presence obligation: once the plane is open,
/// this driver can no longer arrive.
#[tokio::test]
async fn the_declaration_cannot_be_registered_after_the_plane_opens() {
    let mut providers = ProviderDirectory::new();
    providers.mark_plane_open();
    let error = providers
        .register_driver(&descriptor())
        .expect_err("late registration");
    assert!(matches!(
        &error,
        ProviderDirectoryError::RequiredBeforeOpen { type_name } if type_name.as_str() == "Host"
    ));
}
