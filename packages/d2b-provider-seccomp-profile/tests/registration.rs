//! SeccompProfile family registration boundary: the driver declaration is what the plane
//! registers, and the registry serves this type's decoder and factory from it.

use d2b_provider_seccomp_profile::SECCOMP_PROFILE_TYPE_NAME;
use d2b_provider_seccomp_profile::seccomp_profile_descriptor;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    seccomp_profile_descriptor()
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[tokio::test]
async fn descriptor_declares_and_registers_the_seccomp_profile_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::SECCOMP_PROFILE);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        SECCOMP_PROFILE_TYPE_NAME
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN,
        "the controller family's types are built-in only"
    );
    assert!(
        descriptor.allowed_sources.requires_plane_registration(),
        "without a RUNTIME bit the type must be registered before the plane opens"
    );
    assert!(!descriptor.exportable, "a SeccompProfile row is never an export subject");
    assert_eq!(
        descriptor.execution,
        &["host"],
        "a SeccompProfile row carries no placement anchor, so the plane reconciles it on its Host"
    );
    assert_eq!(descriptor.reads, &[]);
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
    assert!(descriptor.startup.is_empty());
    assert!(descriptor.services.is_empty());

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(SECCOMP_PROFILE_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(SECCOMP_PROFILE_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(SECCOMP_PROFILE_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), SECCOMP_PROFILE_TYPE_NAME);
    factory
        .create(&ResourceKey::new("work", SECCOMP_PROFILE_TYPE_NAME, "sample"))
        .await;
}

/// One driver per resource type: a second registration for the same type is
/// refused without clobbering the first.
#[tokio::test]
async fn a_second_registration_of_the_type_is_refused() {
    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor()).expect("first registration");
    let error = providers
        .register_driver(&descriptor())
        .expect_err("duplicate registration");
    assert!(matches!(
        &error,
        ProviderDirectoryError::DuplicateType(type_name)
            if type_name.as_str() == SECCOMP_PROFILE_TYPE_NAME
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
        ProviderDirectoryError::RequiredBeforeOpen { type_name }
            if type_name.as_str() == SECCOMP_PROFILE_TYPE_NAME
    ));
}
