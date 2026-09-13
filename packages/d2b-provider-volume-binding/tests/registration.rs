//! VolumeBinding family registration boundary: the driver declaration is what
//! the plane registers, and the registry serves this type's decoder and
//! factory from it.

use std::sync::Arc;

use d2b_provider_volume_binding::{
    BINDING_CREATIONS, BINDING_TYPE_NAME, BindingDriverArgs, BindingDriverEffects,
    binding_descriptor,
};
use d2b_provider_volume_virtiofs::SocketIdentity;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, ChildCustody, WellKnownType};

/// The port instance the declaration carries; the registration boundary
/// never runs a serving effect.
struct UnusedEffects;

#[async_trait::async_trait]
impl BindingDriverEffects for UnusedEffects {
    async fn socket_ready(&self, _socket: &SocketIdentity) -> bool {
        false
    }

    async fn remove_socket(&self, _socket: &SocketIdentity) -> Result<(), String> {
        Err("registration boundary runs no serving effect".to_owned())
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    binding_descriptor(BindingDriverArgs {
        zone: "work".to_owned(),
        effects: Arc::new(UnusedEffects),
        vcpu_count: 1,
    })
}

/// The declaration registers the one type it serves and carries the
/// declaration the plane resolves roles, the CLI noun surface, and the
/// registry coverage check against.
#[tokio::test]
async fn descriptor_declares_and_registers_the_binding_type() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::VOLUME_BINDING);
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        BINDING_TYPE_NAME
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
        &[WellKnownType::VOLUME],
        "the driver resolves the owning Volume row for its view spec and owner fence"
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
        vec![ResourceTypeName::new(BINDING_TYPE_NAME)]
    );
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(BINDING_TYPE_NAME)),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&ResourceTypeName::new(BINDING_TYPE_NAME))
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), BINDING_TYPE_NAME);
    factory
        .create(&ResourceKey::new("work", "VolumeBinding", "binding"))
        .await;
}

/// The declaration is the license to mint the owned children: each row names
/// the child type, the Provider that serves it, and the rank that realizes
/// the producer before the socket it serves and retires the socket first.
#[test]
fn the_declaration_licenses_the_owned_worker_and_endpoint_children() {
    assert_eq!(BINDING_CREATIONS, descriptor().creations);
    assert_eq!(BINDING_CREATIONS.len(), 2);

    let worker = BINDING_CREATIONS[0];
    assert_eq!(worker.child, WellKnownType::PROCESS);
    assert_eq!(
        worker.provider_ref, "Provider/system-minijail",
        "the binding-owned worker runs under the minijail Process Provider"
    );
    assert_eq!(worker.custody, ChildCustody::DriverOwned);
    assert_eq!(
        worker.order, 0,
        "the worker is realized before its Endpoint"
    );

    let endpoint = BINDING_CREATIONS[1];
    assert_eq!(endpoint.child, WellKnownType::ENDPOINT);
    assert_eq!(
        endpoint.provider_ref, "Provider/volume-virtiofs",
        "the binding's own Provider serves the worker socket"
    );
    assert_eq!(endpoint.custody, ChildCustody::DriverOwned);
    assert!(
        endpoint.order > worker.order,
        "the Endpoint retires before the Process that produces it"
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
        ProviderDirectoryError::DuplicateType(type_name)
            if type_name.as_str() == BINDING_TYPE_NAME
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
            if type_name.as_str() == BINDING_TYPE_NAME
    ));
}
