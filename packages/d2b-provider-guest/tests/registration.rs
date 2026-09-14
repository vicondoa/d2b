//! Guest family registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.

use std::sync::Arc;

use d2b_contracts_resource::v3::ControllerGeneration;
use d2b_provider_guest::{
    GUEST_TYPE_NAME, GuestDriverArgs, GuestDriverEffects, GuestEffectError, GuestEffectOutcome,
    GuestEffectRequest, GuestFinalizeStage, GuestKind, guest_descriptor,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, ChildCustody, WellKnownType};

/// The port instance the declaration carries; the registration boundary
/// never runs an effect.
struct UnusedEffects;

#[async_trait::async_trait]
impl GuestDriverEffects for UnusedEffects {
    async fn reconcile(
        &self,
        _kind: GuestKind,
        _request: &GuestEffectRequest<'_>,
    ) -> Result<GuestEffectOutcome, GuestEffectError> {
        Err(GuestEffectError::Unavailable)
    }

    async fn finalize(
        &self,
        _kind: GuestKind,
        _request: &GuestEffectRequest<'_>,
    ) -> Result<GuestFinalizeStage, GuestEffectError> {
        Err(GuestEffectError::Unavailable)
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    guest_descriptor(GuestDriverArgs {
        zone: "work".to_owned(),
        controller_generation: ControllerGeneration::new(1).expect("generation"),
        effects: Arc::new(UnusedEffects),
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[tokio::test]
async fn descriptor_declares_and_registers_the_guest_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::GUEST);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        GUEST_TYPE_NAME
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP,
        "the plane cannot serve a guest whose runtime Provider never registered"
    );
    assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
    assert!(!descriptor.exportable, "no guest is an export subject");
    assert_eq!(
        descriptor.execution,
        &["host", "guest"],
        "a guest row executes on a Host, or inside a containing Guest for a nested scope"
    );
    assert_eq!(
        descriptor.reads,
        &[
            WellKnownType::PROVIDER,
            WellKnownType::GUEST,
            WellKnownType::HOST,
            WellKnownType::VOLUME,
            WellKnownType::VOLUME_BINDING,
            WellKnownType::PROCESS,
            WellKnownType::ENDPOINT,
            WellKnownType::DEVICE,
            WellKnownType::NETWORK,
        ],
        "the family reads the selected Provider row, its owned children, and the spec's references"
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

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(
        providers.registered_types(),
        vec![ResourceTypeName::new(GUEST_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(GUEST_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(GUEST_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), GUEST_TYPE_NAME);
    factory.create(&ResourceKey::new("work", "Guest", "gateway")).await;
}

/// Every child the family creates is declared with the creator that creates
/// it: the driver commits the qemu-media and azure-container-apps children
/// through the manager child API, and the Cloud Hypervisor controller session
/// commits its fixed child roles through the plane's child bridge - the same
/// Volume and Process Providers, so each of those pairs carries one row per
/// creator.
#[tokio::test]
async fn the_declaration_names_every_child_and_its_creator() {
    let descriptor = descriptor();
    let declared = descriptor
        .creations
        .iter()
        .map(|creation| {
            (
                creation.child.to_resource_type_name().as_str().to_owned(),
                creation.provider_ref.to_owned(),
                creation.custody,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        declared,
        vec![
            (
                "Volume".to_owned(),
                d2b_provider_guest_cloud_hypervisor::identity::VOLUME_PROVIDER_REF.to_owned(),
                ChildCustody::DriverOwned,
            ),
            (
                "Volume".to_owned(),
                d2b_provider_guest_cloud_hypervisor::identity::VOLUME_PROVIDER_REF.to_owned(),
                ChildCustody::ControllerOwned,
            ),
            (
                "Process".to_owned(),
                d2b_provider_guest_cloud_hypervisor::identity::PROCESS_PROVIDER_REF.to_owned(),
                ChildCustody::DriverOwned,
            ),
            (
                "Process".to_owned(),
                d2b_provider_guest_cloud_hypervisor::identity::PROCESS_PROVIDER_REF.to_owned(),
                ChildCustody::ControllerOwned,
            ),
            (
                "Endpoint".to_owned(),
                d2b_provider_guest_cloud_hypervisor::PROVIDER_REF.to_owned(),
                ChildCustody::ControllerOwned,
            ),
            (
                "Endpoint".to_owned(),
                d2b_provider_guest_azure_container_apps::PROVIDER_REF.to_owned(),
                ChildCustody::DriverOwned,
            ),
        ],
        "the declaration must name every child the family creates and who creates it"
    );
}

/// The pairs the driver itself commits through the manager child API are
/// declared driver-owned, so the toolkit's creation fence authorizes them and
/// a creations-driven consumer sees the driver as their creator.
///
/// The pairs are the ones `qemu_child_ensures` (the runtime Volume and the
/// VMM Process) and `aca_child_ensures` (the sandbox-agent Endpoint) commit
/// through `ResourceContext::ensure_child`.
#[tokio::test]
async fn the_children_the_driver_commits_are_declared_driver_owned() {
    let descriptor = descriptor();
    let driver_committed = [
        (
            WellKnownType::VOLUME,
            d2b_provider_guest_cloud_hypervisor::identity::VOLUME_PROVIDER_REF,
        ),
        (
            WellKnownType::PROCESS,
            d2b_provider_guest_cloud_hypervisor::identity::PROCESS_PROVIDER_REF,
        ),
        (
            WellKnownType::ENDPOINT,
            d2b_provider_guest_azure_container_apps::PROVIDER_REF,
        ),
    ];
    for (child, provider_ref) in driver_committed {
        let custody = descriptor
            .creations
            .iter()
            .filter(|creation| creation.child == child && creation.provider_ref == provider_ref)
            .map(|creation| creation.custody)
            .collect::<Vec<_>>();
        assert!(
            custody.contains(&ChildCustody::DriverOwned),
            "{child:?} over {provider_ref} is created by the driver and must be declared \
             driver-owned, declared custody: {custody:?}"
        );
    }
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
            if type_name.as_str() == GUEST_TYPE_NAME
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
            if type_name.as_str() == GUEST_TYPE_NAME
    ));
}
