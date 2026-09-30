//! ExecutionPolicy registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.
//!
//! The contract half is proven here too, because the registration is only
//! meaningful if the type it names is the one the canonical contract declares:
//! a driver registered under a name the contract does not define would serve
//! rows nothing else could read.

use d2b_contracts_resource::v3::{EXECUTION_POLICY_RESOURCE_TYPE, NamespaceClass, ResourceTypeName};
use d2b_provider_execution_policy::{decode_policy_row, execution_policy_descriptor};
use d2b_resource_types::WellKnownType;
use d2b_resource_types::assert_metadata_registration;

/// The declared `ExecutionPolicy` type satisfies the shared declaration-only
/// metadata driver contract: the one type its crate owns, registered through
/// its declared decoder and factory.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_declaration_registers_the_execution_policy_type() {
    assert_metadata_registration(&execution_policy_descriptor(), WellKnownType::EXECUTION_POLICY)
        .await;
}

/// The registered name is the contract's canonical ResourceType name, so a
/// row the contract admits is a row this driver serves. The standard catalog
/// entry that makes the name resolvable as a `ResourceRef` is the cutover's
/// registration, so this asserts the two names agree and records that the
/// reference is still unconstructible rather than pretending otherwise.
#[test]
fn the_registered_name_is_the_contract_canonical_resource_type() {
    let registered = execution_policy_descriptor()
        .resource_type
        .to_resource_type_name();
    assert_eq!(registered.as_str(), EXECUTION_POLICY_RESOURCE_TYPE);
    assert_eq!(
        registered.as_str(),
        WellKnownType::EXECUTION_POLICY
            .to_resource_type_name()
            .as_str()
    );
    assert!(ResourceTypeName::parse(EXECUTION_POLICY_RESOURCE_TYPE).is_err());
}

/// The decode boundary reads the committed bytes as the canonical spec, and a
/// row carrying a field this resource does not define is refused there rather
/// than decoded with that field dropped.
#[test]
fn the_decode_boundary_reads_the_canonical_spec_only() {
    let row = serde_json::json!({
        "namespaces": { "classes": ["user", "mount"] },
        "capabilities": { "allowed": ["network-bind"] },
        "noNewPrivileges": true,
        "identity": { "userRef": null, "requireUserNamespace": false },
        "root": { "readOnlyRoot": true, "privateRoot": true },
        "seccomp": { "profileRef": "SeccompProfile/desktop" },
        "umask": 63
    });
    let bytes = serde_json::to_vec(&row).expect("the row serializes");
    let spec = decode_policy_row(&bytes).expect("the canonical row decodes");
    assert!(spec.no_new_privileges());
    assert!(spec.root().read_only_root());
    assert!(spec.namespaces().requires(NamespaceClass::User));
    assert_eq!(spec.umask(), Some(63));

    let mut retired = row.clone();
    retired
        .as_object_mut()
        .expect("the row is an object")
        .insert("mounts".to_owned(), serde_json::json!([]));
    decode_policy_row(&serde_json::to_vec(&retired).expect("the row serializes"))
        .expect_err("a policy row carrying mounts is not a policy row");
}
