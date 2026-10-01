//! NetworkBinding family registration boundary: the driver declaration is what
//! the plane registers, and the registry serves this type's decoder and
//! factory from it.

use d2b_contracts_resource::v3::{ResourceUid, ZoneId};
use d2b_provider_network_binding::test_support::FakeFabricEffects;
use d2b_provider_network_binding::{
    NETWORK_BINDING_CREATIONS, NETWORK_BINDING_PROVIDER_REF, NETWORK_BINDING_READS,
    NETWORK_BINDING_TYPE_NAME, NetworkBindingDriverArgs, binding_descriptor,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    binding_descriptor(NetworkBindingDriverArgs {
        zone: ZoneId::parse("work").expect("zone"),
        zone_uid: ResourceUid::parse("523e4567-e89b-42d3-a456-426614174004").expect("zone uid"),
        facets: FakeFabricEffects::new().facet_set(),
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn descriptor_declares_and_registers_the_binding_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::NETWORK_BINDING);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        NETWORK_BINDING_TYPE_NAME
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve the converted binding shapes without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no binding is an export subject");
    assert_eq!(
        descriptor.execution,
        &["host"],
        "a binding row carries no execution anchor, so the plane reconciles it on its Host"
    );
    assert_eq!(
        descriptor.reads, NETWORK_BINDING_READS,
        "the driver resolves the Network row for its fabric identity and the target row for its \
         consumer identity"
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
    assert!(descriptor.startup.is_empty());
    assert!(
        descriptor.services.is_empty(),
        "the family's only cross-provider surface is the driver effect port"
    );

    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&descriptor)
        .expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(NETWORK_BINDING_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(NETWORK_BINDING_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(NETWORK_BINDING_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), NETWORK_BINDING_TYPE_NAME);
    factory
        .create(&ResourceKey::new("work", "NetworkBinding", "membership"))
        .await;
}

/// A membership is realized on the source's own shared fabric, so the
/// declaration licenses no child row: nothing here mints a child a second
/// provider has to serve.
#[test]
fn the_declaration_licenses_no_child_row() {
    assert_eq!(NETWORK_BINDING_CREATIONS, descriptor().creations);
    assert!(
        NETWORK_BINDING_CREATIONS.is_empty(),
        "the interface, the routes, and the per-membership firewall entry are the Network \
         provider's host state, not rows this driver mints"
    );
}

/// The serving Provider the rows name is the one that owns the shared fabric,
/// so the crate states it once and the row's providerRef is checked against it.
#[test]
fn the_row_names_the_provider_that_owns_the_shared_fabric() {
    assert_eq!(
        NETWORK_BINDING_PROVIDER_REF, "Provider/network-local",
        "the membership is realized on the Network provider's fabric"
    );
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
            if type_name.as_str() == NETWORK_BINDING_TYPE_NAME
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
            if type_name.as_str() == NETWORK_BINDING_TYPE_NAME
    ));
}
