//! `DeviceBinding` family registration boundary: the driver declaration is
//! what the plane registers, and the registry serves this type's decoder and
//! factory from it.

use d2b_provider_device_binding::test_support::FakeAttachmentEffects;
use d2b_provider_device_binding::{
    DEVICE_BINDING_CREATIONS, DEVICE_BINDING_EFFECTS_SERVICE, DEVICE_BINDING_TYPE_NAME,
    DeviceBindingDriverArgs, binding_descriptor,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::AllowedSources;

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    binding_descriptor(DeviceBindingDriverArgs {
        facets: FakeAttachmentEffects::new().facet_set(),
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn descriptor_declares_and_registers_the_binding_type() {
    let descriptor = descriptor();
    assert_eq!(
        descriptor.resource_type,
        d2b_resource_types::WellKnownType::DEVICE_BINDING
    );
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        DEVICE_BINDING_TYPE_NAME
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
        descriptor.reads,
        &[d2b_resource_types::WellKnownType::DEVICE],
        "the driver reads the Device the row names, so a binding whose source is gone or          re-owned refuses instead of keeping a claim it cannot prove"
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
    assert_eq!(
        descriptor.services,
        &[DEVICE_BINDING_EFFECTS_SERVICE],
        "the family's declared effects service rides the declaration (U6)"
    );

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(DEVICE_BINDING_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(DEVICE_BINDING_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(DEVICE_BINDING_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), DEVICE_BINDING_TYPE_NAME);
    factory
        .create(&ResourceKey::new(
            "work",
            "DeviceBinding",
            "dev-binding-0001",
        ))
        .await;
}

/// The declaration licenses no child creation, and says so: the realized
/// attachment is a mediation over the Device provider's own trusted
/// inventory, so a binding row that minted a Process or Endpoint child would
/// be a second authority over a surface the Device driver already declares.
#[test]
fn the_declaration_licenses_no_child_creation() {
    assert_eq!(DEVICE_BINDING_CREATIONS, descriptor().creations);
    assert!(
        DEVICE_BINDING_CREATIONS.is_empty(),
        "the attachment surface belongs to the Device provider's own declaration"
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
            if type_name.as_str() == DEVICE_BINDING_TYPE_NAME
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
            if type_name.as_str() == DEVICE_BINDING_TYPE_NAME
    ));
}