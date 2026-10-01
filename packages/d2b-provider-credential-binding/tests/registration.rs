//! CredentialBinding family registration boundary: the driver declaration is
//! what the plane registers, and the registry serves this type's decoder and
//! factory from it.

use d2b_contracts_resource::v3::ZoneId;
use d2b_provider_credential_binding::test_support::FakeDeliveryEffects;
use d2b_provider_credential_binding::{
    CREDENTIAL_BINDING_CREATIONS, CREDENTIAL_BINDING_EFFECTS_SERVICE, CREDENTIAL_BINDING_PROVIDER_REF,
    CREDENTIAL_BINDING_TYPE_NAME, CredentialBindingDriverArgs, binding_descriptor,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    binding_descriptor(CredentialBindingDriverArgs {
        zone: ZoneId::parse("work").expect("zone"),
        facets: FakeDeliveryEffects::new().facet_set(),
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn descriptor_declares_and_registers_the_binding_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::CREDENTIAL_BINDING);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        CREDENTIAL_BINDING_TYPE_NAME
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
        &[
            WellKnownType::CREDENTIAL,
            WellKnownType::PROCESS,
            WellKnownType::EPHEMERAL_PROCESS,
            WellKnownType::GUEST,
        ],
        "the driver resolves the source Credential row and the consumer component row"
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
        &[CREDENTIAL_BINDING_EFFECTS_SERVICE],
        "the family's declared effects service rides the declaration"
    );

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(CREDENTIAL_BINDING_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(CREDENTIAL_BINDING_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(CREDENTIAL_BINDING_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), CREDENTIAL_BINDING_TYPE_NAME);
    factory
        .create(&ResourceKey::new("work", CREDENTIAL_BINDING_TYPE_NAME, "delivery"))
        .await;
}

/// The declaration mints no child row: a delivery is realized inside an
/// admitted session at a consumer component that already exists, so the row
/// licenses no creation at all.
#[test]
fn the_declaration_licenses_no_child_rows() {
    assert_eq!(CREDENTIAL_BINDING_CREATIONS, descriptor().creations);
    assert!(
        CREDENTIAL_BINDING_CREATIONS.is_empty(),
        "a delivery that also existed as a child row would outlive the \
         authority that admitted it"
    );
}

/// The serving provider this family owns is the one its rows name: a binding
/// row names the provider that realizes the delivery leg, exactly as a
/// volume binding row names the provider that serves its share.
#[test]
fn the_rows_name_the_provider_this_family_serves() {
    assert_eq!(CREDENTIAL_BINDING_PROVIDER_REF, "Provider/credential-binding");
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
            if type_name.as_str() == CREDENTIAL_BINDING_TYPE_NAME
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
            if type_name.as_str() == CREDENTIAL_BINDING_TYPE_NAME
    ));
}
