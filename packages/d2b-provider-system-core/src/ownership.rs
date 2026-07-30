//! The closed ownership boundary of `Provider/system-core`.
//!
//! `ADR-046-provider-model-and-packaging`, section "system-core bootstrap",
//! states what this Provider owns and then states, separately and
//! explicitly, what it does not. The negative half is load bearing: the
//! bootstrap controller is the only code trusted with ambient authority
//! (`ADR-046-security-and-threat-model`, section "Minimal core vs. semantic
//! Provider authority"), so a ResourceType silently drifting into its scope
//! widens the most privileged surface in the Zone.
//!
//! The boundary is therefore an allowlist. [`OWNED_RESOURCE_TYPES`] is the
//! whole of it, and anything absent is refused, whether or not it appears
//! in [`DISOWNED_RESOURCE_TYPES`]. That named list exists so a test can
//! assert the specification's exact negative enumeration is still refused,
//! not because refusal depends on membership in it.

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::host::HOST_RESOURCE_TYPE;
use d2b_contracts::v3::user::USER_RESOURCE_TYPE;

use crate::error::SystemCoreError;

/// Every ResourceType `system-core` reconciles. The list is exhaustive.
pub const OWNED_RESOURCE_TYPES: [&str; 2] = [HOST_RESOURCE_TYPE, USER_RESOURCE_TYPE];

/// The ResourceTypes the specification names as explicitly not owned.
///
/// Process and EphemeralProcess belong to `system-systemd` and
/// `system-minijail`; the rest belong to their own semantic Providers.
/// This is documentation and test material, never the decision surface.
pub const DISOWNED_RESOURCE_TYPES: [&str; 6] = [
    "Process",
    "EphemeralProcess",
    "Volume",
    "Network",
    "Device",
    "Credential",
];

/// Whether `system-core` reconciles this ResourceType.
pub fn owns(resource_type: &str) -> bool {
    OWNED_RESOURCE_TYPES.contains(&resource_type)
}

/// Require that a reference names an owned ResourceType.
///
/// Fails closed for everything else, including a ResourceType that no
/// Provider has claimed yet.
pub fn require_owned(reference: &ResourceRef) -> Result<(), SystemCoreError> {
    if owns(reference.resource_type().as_str()) {
        Ok(())
    } else {
        Err(SystemCoreError::ResourceTypeNotOwned)
    }
}

/// Require that a reference names exactly `expected`.
pub fn require_resource_type(
    reference: &ResourceRef,
    expected: &str,
) -> Result<(), SystemCoreError> {
    require_owned(reference)?;
    if reference.resource_type().as_str() == expected {
        Ok(())
    } else {
        Err(SystemCoreError::ResourceTypeNotOwned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owned_set_is_exactly_host_and_user() {
        assert!(owns(HOST_RESOURCE_TYPE));
        assert!(owns(USER_RESOURCE_TYPE));
        assert_eq!(OWNED_RESOURCE_TYPES.len(), 2);
    }

    #[test]
    fn every_named_disowned_type_is_refused() {
        for disowned in DISOWNED_RESOURCE_TYPES {
            assert!(!owns(disowned), "{disowned} must not be owned");
        }
    }

    #[test]
    fn an_unclaimed_resource_type_is_refused_too() {
        // The boundary is an allowlist, so a ResourceType that appears in
        // neither list is still refused.
        assert!(!owns("Guest"));
        assert!(!owns("Provider"));
        assert!(!owns("SomeFutureSemanticType"));
    }
}
