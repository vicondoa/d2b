//! Canonical `Provider/runtime-azure-container-apps` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod audit;
mod controller;
mod deployment_service;
pub mod gateway_compat;
mod metrics;

pub use audit::{AcaAuditEvent, AcaAuditOutcome, AcaAuditSink};
pub use controller::{
    AcaClock, AcaController, AcaControllerError, AcaPhase, AcaReconcileOutcome, AcaRecoveryState,
    AcaStatus, AzureContainerAppsRuntimeProvider, CompletedOperationLedger, SystemAcaClock,
};
pub use d2b_contracts::provider_effects::aca::{
    AcaConfiguredDiskId, AcaConfiguredImageId, AcaControl, AcaControlContext, AcaControlError,
    AcaControlErrorKind, AcaControlHealth, AcaCpuMillis, AcaCredentialLease,
    AcaCredentialLeaseClient, AcaCredentialLeaseRequest, AcaCredentialPurpose, AcaDeleteOutcome,
    AcaDesiredDiskImage, AcaDesiredSandbox, AcaDiskImageCandidates, AcaDiskImageId,
    AcaDiskImageName, AcaDiskImageRecord, AcaDiskImageSource, AcaManagedIdentityBindingId,
    AcaMemoryMib, AcaOperationId, AcaProfileId, AcaProviderConfig, AcaReadinessPolicy,
    AcaResourceBinding, AcaRuntimeConfig, AcaSandboxCandidates, AcaSandboxId, AcaSandboxLifecycle,
    AcaSandboxProfile, AcaSandboxRecord, AcaTypeError, AcaWorkloadQuery, MAX_ACA_CANDIDATES,
    MAX_ACA_COMPLETED_OPERATIONS, MAX_ACA_LEASE_CLEANUP_MS, MAX_ACA_PLAN_TTL_MS,
    MAX_ACA_READY_ATTEMPTS, MAX_ACA_READY_INTERVAL_MS, MAX_ACA_RESOURCE_ID_LEN,
    MAX_ACA_RETRY_AFTER_MS,
};
pub use deployment_service::{
    AcaDeploymentRequest, AcaDeploymentResponse, AcaDeploymentService, AcaServiceError,
    AcaServiceMethod,
};
pub use metrics::{AcaMetricEvent, AcaMetricOutcome, AcaMetricValidationError};

/// Stable Provider implementation identifier.
pub const ACA_IMPLEMENTATION_ID: &str = "azure-container-apps";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-azure-container-apps";
