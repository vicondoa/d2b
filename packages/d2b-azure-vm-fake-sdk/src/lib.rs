//! In-process fake Azure VM SDK for compile-time provider conformance.
//!
//! This crate intentionally has no Azure, credential, endpoint, filesystem, or
//! network integration. Its closed DTOs model only the authority seams needed
//! by the non-production Azure VM provider scaffolds.

#![forbid(unsafe_code)]

mod binding;
mod client;
mod types;

pub use binding::{
    BindingMaterialError, InfrastructureBindingFingerprint, InfrastructureBindingMaterial,
};
pub use client::FakeAzureVmSdk;
pub use types::{
    ApplyDisposition, BootstrapBinding, CallDisposition, CallRecord, CallSnapshot,
    ConfiguredOutcome, DeploymentHandle, DeploymentMutation, DeploymentObservation,
    DeploymentState, FakeSdkError, FakeSdkErrorKind, InfrastructureHandle, InfrastructureMutation,
    InfrastructureObservation, MAX_CALL_LOG_ENTRIES, MAX_CONFIGURED_OUTCOMES, MAX_REPLAY_ENTRIES,
    MutationResult, OperationKey, OperationScope, PowerState, ResourceGeneration, ResourceId,
    SdkAxis, SdkCallContext, SdkOperation,
};

#[cfg(test)]
mod tests;
