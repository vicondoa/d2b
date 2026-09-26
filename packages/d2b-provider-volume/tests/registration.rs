//! Volume family registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.

use std::sync::Arc;

use d2b_provider_volume::{
    VOLUME_CREATIONS, VOLUME_EFFECTS_SERVICE, VOLUME_TYPE_NAME, VolumeDriverArgs, volume_descriptor,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, ChildCustody, WellKnownType};

/// The facet set the declaration carries; the registration boundary never
/// runs an effect. The runtime double refuses every call, so a test that
/// accidentally drives an effect fails loudly instead of passing silently.
fn unused_facets() -> d2b_provider_volume::VolumeEffectFacets {
    d2b_provider_volume::VolumeEffectFacets {
        runtime: Arc::new(d2b_provider_volume::test_support::RefusingRuntime),
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    volume_descriptor(VolumeDriverArgs {
        facets: unused_facets(),
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn descriptor_declares_and_registers_the_volume_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::VOLUME);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        VOLUME_TYPE_NAME
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve the converted volume shapes without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no volume is an export subject");
    assert_eq!(
        descriptor.execution,
        &["host"],
        "a Volume row carries no execution anchor, so the plane reconciles it on its Host"
    );
    assert!(
        descriptor.reads.is_empty(),
        "the driver derives its children from its own stored spec"
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
    assert_eq!(
        descriptor.services,
        &[VOLUME_EFFECTS_SERVICE],
        "the family's declared effects service rides the declaration (U7)"
    );

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(VOLUME_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(VOLUME_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(VOLUME_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), VOLUME_TYPE_NAME);
    factory
        .create(&ResourceKey::new("work", "Volume", "data"))
        .await;
}

/// The declaration is the license to mint the derived binding children: the
/// one creation the driver may make names the child type, the Provider that
/// serves it, and the custody the driver holds over its teardown.
#[test]
fn the_declaration_licenses_the_derived_binding_children() {
    assert_eq!(VOLUME_CREATIONS, descriptor().creations);
    assert_eq!(VOLUME_CREATIONS.len(), 1);
    assert_eq!(VOLUME_CREATIONS[0].child, WellKnownType::VOLUME_BINDING);
    assert_eq!(
        VOLUME_CREATIONS[0].provider_ref, "Provider/volume-virtiofs",
        "the derived binding rows select the serving Provider"
    );
    assert_eq!(VOLUME_CREATIONS[0].custody, ChildCustody::DriverOwned);
    assert_eq!(VOLUME_CREATIONS[0].order, 0);
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
            if type_name.as_str() == VOLUME_TYPE_NAME
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
            if type_name.as_str() == VOLUME_TYPE_NAME
    ));
}
