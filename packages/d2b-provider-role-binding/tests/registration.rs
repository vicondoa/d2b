//! RoleBinding registration boundary: the driver declaration is what the plane
//! registers, and the registry serves this type's decoder and factory from it.

use d2b_resource_types::WellKnownType;
use d2b_resource_types::assert_metadata_registration;

/// The declared `RoleBinding` type satisfies the shared declaration-only metadata
/// driver contract: the one type its crate owns, registered through its
/// declared decoderand factory.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_declaration_registers_the_role_binding_type() {
    assert_metadata_registration(
        &d2b_provider_role_binding::role_binding_descriptor(),
        WellKnownType::ROLE_BINDING,
    )
    .await;
}
