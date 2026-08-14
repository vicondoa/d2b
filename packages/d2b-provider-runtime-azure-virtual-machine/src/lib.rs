//! Canonical `Provider/runtime-azure-virtual-machine` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod audit;
pub mod bootstrap;
pub mod bootstrap_svc;
pub mod config;
pub mod controller;
pub mod effect;
pub mod error;
pub mod idempotency;
pub mod telemetry;

pub use bootstrap::{BootstrapAdmission, BootstrapAdmissionState, BootstrapPsk};
pub use bootstrap_svc::{BootstrapService, BootstrapServiceState};
pub use config::{
    AzureVmConfig, AzureVmGuestSettings, BootstrapPskDelivery, DataDiskSpec, DiskSku,
};
pub use controller::{
    AzureVmClock, AzureVmController, AzureVmPhase, AzureVmReconcileOutcome, AzureVmStatus,
    SystemAzureVmClock,
};
pub use effect::{
    AzureCredentialPort, AzureEffectPort, AzureOperationHandle, AzureVmHandle, AzureVmState,
    LroStatus, PskExtensionPayload, TagDigest,
};
pub use error::AzureVmError;

/// Stable Provider implementation identifier.
pub const AZURE_VM_IMPLEMENTATION_ID: &str = "azure-vm";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-azure-virtual-machine";
