//! EmergencyPolicy registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.
//!
//! The type is registered by a SERVING declaration, not the declaration-only
//! metadata one: an `EmergencyPolicy` row that converged as opaque metadata
//! enforced nothing, so the assertions here are about a descriptor whose
//! decoder reads the policy contract and whose factory builds a driver.

use d2b_contracts_zone_session::v3::emergency_policy::{
    EMERGENCY_DRAIN_FINALIZER, EMERGENCY_POLICY_RESOURCE_TYPE,
};
use d2b_provider_emergency_policy::{emergency_policy_descriptor, held_drain_finalizer};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};

fn zone() -> d2b_contracts_resource::v3::ZoneId {
    d2b_contracts_resource::v3::ZoneId::parse("work").expect("the fixture zone is canonical")
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    emergency_policy_descriptor(zone())
}

/// The declaration registers the one type it serves and keeps the facts the
/// plane resolves the type's presence against.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_declaration_registers_the_emergency_policy_type() {
    let descriptor = descriptor();
    let type_name = ResourceTypeName::new(EMERGENCY_POLICY_RESOURCE_TYPE);
    assert_eq!(descriptor.resource_type, WellKnownType::EMERGENCY_POLICY);
    assert_eq!(
        type_name.as_str(),
        EMERGENCY_POLICY_RESOURCE_TYPE,
        "the registration names the contract's own constant"
    );
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN,
        "the plane cannot serve the converted policy type without this driver"
    );
    assert!(!descriptor.exportable, "a policy is never an export subject");
    assert!(descriptor.operations.is_empty());
    assert!(descriptor.creations.is_empty());
    assert!(descriptor.services.is_empty());

    let mut providers = ProviderDirectory::new();
    providers.register_driver(&descriptor).expect("register");
    assert_eq!(providers.registered_types(), vec![type_name.clone()]);
    assert!(
        providers.decoders().contains_key(&type_name),
        "the registry serves the type's policy decoder from the declaration"
    );
    let factory = providers
        .lookup(&type_name)
        .expect("the registry serves the declared factory");
    assert_eq!(factory.resource_types(), std::slice::from_ref(&type_name));
    factory
        .create(&ResourceKey::new("work", EMERGENCY_POLICY_RESOURCE_TYPE, "zone"))
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

/// The decoder is the policy contract, so a row that is not a policy is
/// refused at decode rather than reconciled as opaque metadata.
#[test]
fn the_registered_decoder_reads_the_policy_contract() {
    let decoder = descriptor().decoder;
    let policy = d2b_contracts_zone_session::v3::EmergencyPolicySpec::new(
        true,
        d2b_contracts_zone_session::v3::EmergencyScope::new(true, false, false, true),
        30,
        "operator reduction",
    )
    .expect("the policy validates");
    let encoded = serde_json::to_vec(&policy).expect("the policy serializes");
    let decoded = decoder.decode(&encoded).expect("a policy row decodes");
    assert!(
        decoded.downcast_ref::<d2b_contracts_zone_session::v3::EmergencyPolicySpec>().is_some(),
        "the decoder yields the policy itself, not an opaque JSON object"
    );
    assert!(
        decoder.decode(b"{\"notAPolicy\":true}").is_err(),
        "a row that is not a policy never reaches reconcile"
    );
}

/// The finalizer the driver holds is the contract's own name, so the gate the
/// driver applies and the name the teardown waits for cannot drift apart.
#[test]
fn the_driver_holds_the_contract_s_own_drain_finalizer() {
    assert_eq!(held_drain_finalizer(), EMERGENCY_DRAIN_FINALIZER);
}
