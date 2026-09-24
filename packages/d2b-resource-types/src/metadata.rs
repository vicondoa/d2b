//! The shared declaration of the declaration-only metadata types.
//!
//! `Role`, `RoleBinding`, `Command`, `Operation`, `Quota`, `EmergencyPolicy`,
//! `ResourceImport`, `ResourceExport`, `SeccompProfile`, `Zone`, and
//! `ZoneLink` are declared and governed, but realize nothing on a target:
//! their rows converge as metadata, and whatever state they stand for lives
//! either in the controller session (the quota authority index) or in the
//! family crate that materializes the rows (the Zone status projection, the
//! ZoneLink enrollment-and-cursor machine). The v3 rewrite converted every
//! one of those types the same way, and the conversion itself lives in
//! [`d2b_resource_runtime::metadata`]; this module builds the descriptor
//! those types are registered by, so no two of them can diverge on the
//! declaration either.
//!
//! The per-type crate keeps the part that is not shared: the type's identity
//! in [`WellKnownType`], the crate's documentation, and the one-line
//! declaration ([`metadata_descriptor`]) the plane registers the type by.

use std::sync::Arc;

use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::metadata::{
    METADATA_EXECUTION_DOMAINS, MetadataDriverFactory, metadata_spec_decoder,
};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};

use crate::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};

/// The driver declaration of one declaration-only metadata type.
///
/// Every type declared through this function is `BUILTIN` (no RUNTIME bit):
/// the plane cannot serve the converted core types without it, so it must be
/// registered before the plane opens. The driver serves no broker operations
/// and creates no children through this declaration.
///
/// None of the types is exportable: `ResourceExport` admits only qualified
/// `*.d2bus.org.*Service` types, so a row of one of these types is never an
/// export subject.
pub fn metadata_descriptor(resource_type: WellKnownType) -> DriverDescriptor {
    DriverDescriptor {
        resource_type,
        allowed_sources: AllowedSources::BUILTIN,
        verbs: CONVERTED_TYPE_VERBS,
        execution: METADATA_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: metadata_spec_decoder(),
        factory: Arc::new(MetadataDriverFactory::new(
            resource_type.to_resource_type_name(),
        )),
    }
}

/// Assert the registration contract one declaration-only metadata type must
/// satisfy.
///
/// The declaration is what the plane registers, so it is what the registry
/// resolves the type's decoder and factory from: the descriptor declares the
/// one type its crate owns with the shared verb, execution-domain, and
/// allowed-source facts, the registry serves the declared decoder and factory
/// from it, a second registration of the same type is refused without
/// clobbering the first, and a registration arriving after the plane opens is
/// refused because the type's mask carries the presence obligation.
///
/// This is the assertion every per-type crate's registration test runs: the
/// coverage lives here once, and each crate contributes the one type it
/// declares.
pub async fn assert_metadata_registration(descriptor: &DriverDescriptor, expected: WellKnownType) {
    assert_eq!(descriptor.resource_type, expected);
    let type_name = descriptor.resource_type.to_resource_type_name();
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN,
        "the declaration-only metadata types are built-in only"
    );
    assert!(
        descriptor.allowed_sources.requires_plane_registration(),
        "without a RUNTIME bit the type must be registered before the plane opens"
    );
    assert!(
        !descriptor.exportable,
        "a declaration-only metadata row is never an export subject"
    );
    assert_eq!(
        descriptor.execution, METADATA_EXECUTION_DOMAINS,
        "a declaration-only metadata row carries no placement anchor, so the plane reconciles it on its Host"
    );
    assert!(descriptor.reads.is_empty());
    assert_eq!(descriptor.verbs, CONVERTED_TYPE_VERBS);
    assert!(descriptor.operations.is_empty());
    assert!(descriptor.creations.is_empty());
    assert!(descriptor.startup.is_empty());
    assert!(descriptor.services.is_empty());

    let mut providers = ProviderDirectory::new();
    providers.register_driver(descriptor).expect("register");
    assert_eq!(providers.registered_types(), vec![type_name.clone()]);
    assert!(
        providers.decoders().contains_key(&type_name),
        "the registry serves the type's decoder from the declaration"
    );
    let factory = providers
        .lookup(&type_name)
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types().len(), 1);
    assert_eq!(factory.resource_types()[0].as_str(), type_name.as_str());
    factory
        .create(&ResourceKey::new("work", type_name.as_str(), "sample"))
        .await;

    let mut duplicate = ProviderDirectory::new();
    duplicate
        .register_driver(descriptor)
        .expect("first registration");
    let error = duplicate
        .register_driver(descriptor)
        .expect_err("duplicate registration");
    assert!(matches!(
        &error,
        ProviderDirectoryError::DuplicateType(name) if name == &type_name
    ));

    let mut late = ProviderDirectory::new();
    late.mark_plane_open();
    let error = late
        .register_driver(descriptor)
        .expect_err("late registration");
    assert!(matches!(
        &error,
        ProviderDirectoryError::RequiredBeforeOpen { type_name: name } if name == &type_name
    ));
}
