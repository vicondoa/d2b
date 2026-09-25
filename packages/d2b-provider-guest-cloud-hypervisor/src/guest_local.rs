//! Guest-local seed vocabulary and the resolved Guest-control Endpoint.
//!
//! The signed Guest seed schema admits the ResourceTypes below; the
//! authenticated Guest-control Endpoint identity is the only locator-free fact
//! the controller publishes. Endpoint carriage, credentials, and target-local
//! effects remain behind the authenticated session and its effect owner.

use std::fmt;

use d2b_contracts_resource::v3::activation_nixos::NIXOS_GENERATION_RESOURCE_TYPE;

/// Authenticated, locator-free identity of a Guest-control Endpoint.
///
/// Owned by `d2b-resource-client`; re-exported here so the Guest-local
/// vocabulary keeps its stable public path.
pub use d2b_resource_client::GuestControlEndpoint;

/// Resource types admitted by the signed Guest seed schema.
pub const GUEST_SEED_RESOURCE_TYPES: &[&str] = &[
    "Process",
    "EphemeralProcess",
    "Endpoint",
    NIXOS_GENERATION_RESOURCE_TYPE,
];

/// Failures at the Guest-local session and seed boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLocalError {
    /// Endpoint identity or generation did not match the Guest contract.
    EndpointMismatch,
}

impl GuestLocalError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::EndpointMismatch => "guest-local-endpoint-mismatch",
        }
    }
}

impl fmt::Display for GuestLocalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GuestLocalError {}


