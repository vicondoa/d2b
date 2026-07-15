//! Local VM and user-scope runtime provider implementation boundary.

#![forbid(unsafe_code)]

mod config;
mod control;
mod factory;
mod provider;

pub use config::{
    CLOUD_HYPERVISOR_IMPLEMENTATION_ID, LocalRuntimeConfiguration, LocalRuntimeConfigurationError,
    LocalRuntimeKind, MAX_RUNTIME_OPAQUE_ID_BYTES, QEMU_MEDIA_IMPLEMENTATION_ID,
    RuntimeBundleIntentId, RuntimeIntentBinding, RuntimeRunnerId, SYSTEMD_USER_IMPLEMENTATION_ID,
};
pub use control::{
    RuntimeAdoptionControl, RuntimeAdoptionMismatch, RuntimeAdoptionOutcome,
    RuntimeConfiguredItemControl, RuntimeControlContext, RuntimeControlContractError,
    RuntimeControlError, RuntimeControlPort, RuntimeEnsureControl, RuntimeHealth,
    RuntimeMutationOutcome, RuntimeObservedState, RuntimeOperationControl, RuntimePlanDecision,
    RuntimeResourceIdentity,
};
pub use factory::{LocalRuntimeProviderFactory, LocalRuntimeProviderFactoryEntry};
pub use provider::{
    LIVE_RUNTIME_METHODS, LocalRuntimeProvider, LocalRuntimeProviderBuildError,
    live_runtime_capabilities,
};
