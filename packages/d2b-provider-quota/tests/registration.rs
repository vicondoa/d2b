//! Quota registration boundary: the driver declaration is what the plane
//! registers, and the registry serves this type's decoder and factory from
//! it.
//!
//! The type is registered by a SERVING declaration, not the declaration-only
//! metadata one: a `Quota` row that converged as opaque metadata enforced no
//! ceiling at all, so the assertions here are about a descriptor whose
//! decoder reads the ceiling contract and whose factory builds a driver.

use d2b_provider_quota::{QUOTA_RESOURCE_TYPE, quota_descriptor};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

fn zone() -> d2b_contracts_resource::v3::ZoneId {
    d2b_contracts_resource::v3::ZoneId::parse("work").expect("the fixture zone is canonical")
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    quota_descriptor(zone())
}

/// A ceiling row in the shape the contract admits.
fn ceiling_row(max_resources: u32) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        // The committed schema requires all six ceiling keys; the three
        // metered dimensions say "unbounded" as null rather than by being
        // absent, which are different things to a reader.
        "ceilings": {
            "maxResources": max_resources,
            "maxResourcesPerType": max_resources,
            "maxOwnerDepth": 4,
            "maxCpu": null,
            "maxMemoryMib": null,
            "maxStorageGib": null
        },
        "perTypeCeilings": {},
        "scope": "zone",
        "enforcementPolicy": "hard"
    }))
    .expect("the ceiling row serializes")
}

/// The declaration registers the one type it serves and keeps the facts the
/// plane resolves the type's presence against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_declaration_registers_the_quota_type() {
    let descriptor = descriptor();
    let type_name = ResourceTypeName::new(QUOTA_RESOURCE_TYPE);
    assert_eq!(descriptor.resource_type, WellKnownType::QUOTA);
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN,
        "the plane cannot serve the converted ceiling type without this driver"
    );
    assert!(!descriptor.exportable, "a ceiling is never an export subject");
    assert!(descriptor.operations.is_empty());
    assert!(descriptor.creations.is_empty());
    assert!(descriptor.services.is_empty());

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(providers.registered_types(), vec![type_name.clone()]);
    assert!(
        providers.decoders().contains_key(&type_name),
        "the registry serves the type's ceiling decoder from the declaration"
    );
    let factory = providers
        .lookup(&type_name)
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types(), std::slice::from_ref(&type_name));
    factory
        .create(&ResourceKey::new("work", QUOTA_RESOURCE_TYPE, "zone"))
        .await;

    let mut duplicate = ProviderDirectory::new();
    duplicate.register_driver(&descriptor).expect("first registration");
    let error = duplicate
        .register_driver(&descriptor)
        .expect_err("duplicate registration");
    assert!(matches!(
        &error,
        ProviderDirectoryError::DuplicateType(name) if name == &type_name
    ));

    let mut late = ProviderDirectory::new();
    late.mark_plane_open();
    let error = late
        .register_driver(&descriptor)
        .expect_err("late registration");
    assert!(matches!(
        &error,
        ProviderDirectoryError::RequiredBeforeOpen { type_name: name } if name == &type_name
    ));
}

/// The decoder is the ceiling contract, so a row that is not a ceiling is
/// refused at decode rather than reconciled as opaque metadata.
#[test]
fn the_registered_decoder_reads_the_ceiling_contract() {
    let decoder = descriptor().decoder;
    let decoded = decoder
        .decode(&ceiling_row(8))
        .expect("a ceiling row decodes");
    let policy = decoded
        .downcast_ref::<d2b_provider_quota::quota::QuotaPolicy>()
        .expect("the decoder yields the policy itself, not an opaque JSON object");
    assert_eq!(policy.ceilings().max_resources(), 8);
    assert_eq!(
        policy.enforcement(),
        d2b_provider_quota::quota::QuotaEnforcementPolicy::Hard
    );
    assert!(
        decoder.decode(b"{\"notACeiling\":true}").is_err(),
        "a row that is not a ceiling never reaches reconcile"
    );
    assert!(
        decoder.decode(&ceiling_row(0)).is_err(),
        "a ceiling of zero is not a ceiling this contract admits"
    );
}
