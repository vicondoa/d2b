//! Credential family registration boundary: the driver declaration is what
//! the plane registers, and the registry serves this type's decoder and
//! factory from it.

use std::sync::Arc;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_credential::{
    CREDENTIAL_TYPE_NAME, CredentialDependencyFacts, CredentialDriverArgs, CredentialDriverEffects,
    CredentialLeaseFacts, CredentialSession, credential_descriptor,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, ChildCustody, WellKnownType};

/// The port instance the declaration carries; the registration boundary
/// never runs an effect.
struct UnusedEffects;

#[async_trait::async_trait]
impl CredentialDriverEffects for UnusedEffects {
    async fn dependency_facts(
        &self,
        _provider_ref: &ResourceRef,
        _execution_ref: &ResourceRef,
    ) -> Option<CredentialDependencyFacts> {
        None
    }

    async fn lease_facts(&self, _credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts> {
        None
    }

    async fn agent_ready(&self, _agent_ref: &ResourceRef) -> bool {
        false
    }

    fn session(&self, _provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>> {
        None
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    credential_descriptor(CredentialDriverArgs {
        zone: "work".to_owned(),
        controller_generation: d2b_contracts_resource::v3::ControllerGeneration::new(1)
            .expect("controller generation"),
        effects: Arc::new(UnusedEffects),
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[tokio::test]
async fn descriptor_declares_and_registers_the_credential_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::CREDENTIAL);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        CREDENTIAL_TYPE_NAME
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve credential rows without this driver"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no Credential is an export subject");
    assert_eq!(
        descriptor.execution,
        &["host", "guest"],
        "a Credential names a Host or a Guest execution target"
    );
    assert_eq!(
        descriptor.reads,
        &[
            WellKnownType::PROCESS,
            WellKnownType::HOST,
            WellKnownType::GUEST,
            WellKnownType::PROVIDER
        ],
        "the driver reads the Provider row, the execution target, and its agent child"
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
            "use-credential",
            "admin-credential",
        ]
    );
    assert!(descriptor.operations.is_empty());

    // The one declared creation is the managed-identity agent Process, under
    // the minijail Process Provider's own exported reference: the family does
    // not carry a second spelling of that Provider.
    assert_eq!(
        descriptor.creations,
        &[d2b_resource_types::ChildCreation {
            child: WellKnownType::PROCESS,
            provider_ref: d2b_provider_process_minijail::PROVIDER_REF,
            custody: ChildCustody::DriverOwned,
            order: 0,
        }]
    );

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(CREDENTIAL_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(CREDENTIAL_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(CREDENTIAL_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), CREDENTIAL_TYPE_NAME);
    factory
        .create(&ResourceKey::new("work", "Credential", "relay"))
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
            if type_name.as_str() == CREDENTIAL_TYPE_NAME
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
            if type_name.as_str() == CREDENTIAL_TYPE_NAME
    ));
}
