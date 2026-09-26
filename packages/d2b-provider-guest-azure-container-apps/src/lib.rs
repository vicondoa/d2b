//! Canonical `Provider/runtime-azure-container-apps` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod controller;
mod effects;

pub use controller::{
    AcaClock, AcaController, AcaControllerError, AcaPhase, AcaReconcileOutcome,
    AzureContainerAppsRuntimeProvider, ACA_GUEST_FINALIZER, ACA_REPAIR_INTERVAL_SECS,
};
pub use effects::{
    AcaConfiguredDiskId, AcaConfiguredImageId, AcaControl, AcaControlContext, AcaControlError,
    AcaControlErrorKind, AcaControlHealth, AcaCpuMillis, AcaCredentialLease,
    AcaCredentialLeaseClient, AcaCredentialLeaseRequest, AcaCredentialPurpose, AcaDeleteOutcome,
    AcaDesiredDiskImage, AcaDesiredSandbox, AcaDiskImageCandidates, AcaDiskImageId,
    AcaDiskImageName, AcaDiskImageRecord, AcaDiskImageSource, AcaManagedIdentityBindingId,
    AcaMemoryMib, AcaOperationId, AcaProfileId, AcaProviderConfig, AcaReadinessPolicy,
    AcaResourceBinding, AcaRuntimeConfig, AcaSandboxCandidates, AcaSandboxId, AcaSandboxLifecycle,
    AcaSandboxProfile, AcaSandboxRecord, AcaTypeError, AcaWorkloadQuery, MAX_ACA_CANDIDATES,
    MAX_ACA_COMPLETED_OPERATIONS, MAX_ACA_PLAN_TTL_MS, MAX_ACA_READY_ATTEMPTS,
    MAX_ACA_READY_INTERVAL_MS, MAX_ACA_RESOURCE_ID_LEN,
};

/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-azure-container-apps";
/// Stable Guest finalizer.
pub const FINALIZER: &str = ACA_GUEST_FINALIZER;
