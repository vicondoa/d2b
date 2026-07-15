//! Canonical Azure Container Apps runtime provider.
//!
//! The provider implements the real sandbox lifecycle over an injected,
//! asynchronous control port. Cloud credentials are represented only by an
//! opaque co-located lease; this crate has no ambient credential, endpoint,
//! command, free-form payload, daemon, broker, or legacy-provider fallback.

#![forbid(unsafe_code)]

mod control;
mod factory;
mod provider;
mod types;

pub use control::{
    AcaControl, AcaControlContext, AcaControlError, AcaControlErrorBuildError, AcaControlErrorKind,
    AcaControlHealth, AcaCredentialLease, AcaCredentialLeaseClient, AcaCredentialLeaseRequest,
    AcaCredentialPurpose, AcaDiagnosticCode, MAX_ACA_LEASE_CLEANUP_MS, MAX_ACA_RETRY_AFTER_MS,
};
pub use factory::{
    AcaFactoryBuildError, AcaRuntimeProviderBinding, AzureContainerAppsRuntimeProviderFactory,
    aca_provider_factory_key,
};
pub use provider::{
    ACA_IMPLEMENTATION_ID, AcaProviderBuildError, AzureContainerAppsRuntimeProvider,
};
pub use types::{
    AcaConfiguredDiskId, AcaConfiguredImageId, AcaCpuMillis, AcaDeleteOutcome, AcaDesiredDiskImage,
    AcaDesiredSandbox, AcaDiskImageCandidates, AcaDiskImageId, AcaDiskImageName,
    AcaDiskImageRecord, AcaDiskImageSource, AcaManagedIdentityBindingId, AcaMemoryMib,
    AcaProfileId, AcaReadinessPolicy, AcaResourceBinding, AcaRuntimeConfig, AcaSandboxCandidates,
    AcaSandboxId, AcaSandboxLifecycle, AcaSandboxProfile, AcaSandboxRecord, AcaTypeError,
    AcaWorkloadQuery, MAX_ACA_CANDIDATES, MAX_ACA_COMPLETED_OPERATIONS, MAX_ACA_PLAN_TTL_MS,
    MAX_ACA_READY_ATTEMPTS, MAX_ACA_READY_INTERVAL_MS, MAX_ACA_RESOURCE_ID_LEN,
};

#[cfg(test)]
mod tests;
