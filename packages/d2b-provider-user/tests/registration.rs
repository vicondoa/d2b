//! User family registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.

use d2b_contracts_resource::v3::{ ResourceSpec, execution_policy::to_base_object };
use d2b_provider_system_core::user_spec::{ OsUsername, UserSpec };
use d2b_provider_user::test_support::RecordingEffects;
use d2b_provider_user::user_descriptor;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{DriverRegistration, ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    user_descriptor(RecordingEffects::new())
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn descriptor_declares_and_registers_the_user_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::USER);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        "User"
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve the bootstrap User rows without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no User is an export subject");
    assert_eq!(
        descriptor.execution,
        &["host"],
        "a User names no execution anchor: it is reconciled on the machine whose identity it names"
    );
    assert!(
        descriptor.reads.is_empty(),
        "discovery reaches the local machine through the effect port"
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
        vec![ResourceTypeName::new("User")]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new("User")),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new("User"))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), "User");
    factory
        .create(&ResourceKey::new("work", "User", "alice"))
        .await;
}

/// The decoder a descriptor carries is the one the manager wires for the
/// type: it decodes a stored User row envelope and refuses bytes that are not
/// a resource spec.
#[test]
fn the_declared_decoder_decodes_the_stored_user_envelope() {
    let base = to_base_object(&UserSpec::minimal(
        OsUsername::parse("alice").expect("username"),
    ))
    .expect("user base");
    let envelope = ResourceSpec::new(None, None, base, None).expect("admitted resource spec");

    let decoder = DriverRegistration::decoder(&descriptor());
    decoder
        .decode(&envelope.canonical_bytes().expect("canonical bytes"))
        .expect("a stored User row envelope decodes");
    assert!(
        decoder.decode(b"{not-json").is_err(),
        "an unreadable envelope is refused"
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
        ProviderDirectoryError::DuplicateType(type_name) if type_name.as_str() == "User"
    ));
}

/// The declared mask carries the presence obligation: once the plane is open,
/// this driver can no longer arrive.
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
        ProviderDirectoryError::RequiredBeforeOpen { type_name } if type_name.as_str() == "User"
    ));
}
