//! Canonical `Provider/runtime-azure-virtual-machine` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod authority;
pub mod bootstrap;
pub mod config;
pub mod controller;
pub mod declaration;
pub mod effect;
mod error;

pub use authority::{
    AZURE_VM_CONTROL_AUDIENCE, AzureVmAdmittedGuest, AzureVmAdmittedRemote, AzureVmCloudIdentity,
    AzureVmDeliveryContext, AzureVmReconciliationKey, AzureVmReleaseEvidence, AzureVmRemoteAuthority,
    AzureVmRemoteDeliveryPort, AzureVmRemoteGrant, AzureVmRemotePurpose, AzureVmRemoteRefusal,
};
pub use bootstrap::{
    BootstrapAdmission, BootstrapAdmissionState, BootstrapPsk, BootstrapService,
    BootstrapServiceState,
};
pub use config::{
    AzureVmConfig, AzureVmGuestSettings, BootstrapPskDelivery, DataDiskSpec, DiskSku,
};
pub use declaration::{
    AZURE_VM_ARTIFACT_ID, GUEST_CONTROLLER, azure_vm_bindings, azure_vm_declaration,
    declared_presentation,
};
pub use controller::{
    AdmittedRecoveryIdentity, AzureVmController, AzureVmPhase, AzureVmReconcileOutcome,
    AzureVmRecoveryState, AZURE_VM_GUEST_FINALIZER, AZURE_VM_REPAIR_INTERVAL_SECS,
};
pub use effect::{
    AzureAccessToken, AzureCredentialPort, AzureEffectPort, AzureOperationHandle, AzureVmHandle,
    AzureVmState, LroStatus, PskExtensionPayload, TagDigest,
};
pub use error::AzureVmError;

/// Stable Provider implementation identifier.
pub const AZURE_VM_IMPLEMENTATION_ID: &str = "azure-vm";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-azure-virtual-machine";
/// Stable Guest finalizer.
pub const FINALIZER: &str = AZURE_VM_GUEST_FINALIZER;
