//! Canonical provider contract family.

pub use d2b_contracts::foundation_effects::{
    CredentialContractError, CredentialLeaseHandle, OpaqueAzureRef, MAX_AZURE_REF_BYTES,
    MAX_CREDENTIAL_LEASE_HANDLE_BYTES,
};
pub use d2b_contracts_resource::v3::*;
pub use d2b_contracts_resource::v3::identity;

pub mod credential;
pub mod credential_controller;
pub mod provider;
pub mod provider_registry;
pub mod semantic_services;
pub mod telemetry_frame;
pub mod telemetry_policy;

pub use credential::*;
pub use credential_controller::*;
pub use provider::*;
pub use provider_registry::*;
pub use semantic_services::*;
pub use telemetry_frame::*;
pub use telemetry_policy::*;
