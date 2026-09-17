//! SeccompProfile registration boundary: the driver declaration is what the plane
//! registers, and the registry serves this type's decoder and factory from it.

use d2b_resource_types::WellKnownType;
use d2b_resource_types::assert_metadata_registration;

/// The declared `SeccompProfile` type satisfies the shared declaration-only metadata
/// driver contract: the one type its crate owns, registered through its
/// declared decoderand factory.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_declaration_registers_the_seccomp_profile_type() {
    assert_metadata_registration(
        &d2b_provider_seccomp_profile::seccomp_profile_descriptor(),
        WellKnownType::SECCOMP_PROFILE,
    )
    .await;
}
