//! Local VM and user-scope runtime provider implementation boundary.

#![forbid(unsafe_code)]

mod config;
mod control;
mod factory;
mod provider;

pub use config::{
    CLOUD_HYPERVISOR_IMPLEMENTATION_ID, CloudHypervisorConfiguration, LocalRuntimeConfiguration,
    LocalRuntimeConfigurationError, LocalRuntimeKind, MAX_CONFIGURED_RUNTIME_ITEMS,
    QEMU_MEDIA_IMPLEMENTATION_ID, QemuMediaConfiguration, SYSTEMD_USER_IMPLEMENTATION_ID,
    SystemdUserConfiguration,
};
pub use control::{
    RuntimeAdoptionControl, RuntimeAdoptionMismatch, RuntimeAdoptionOutcome,
    RuntimeConfiguredItemControl, RuntimeControlContext, RuntimeControlContractError,
    RuntimeControlError, RuntimeControlPort, RuntimeEnsureControl, RuntimeHealth,
    RuntimeMutationOutcome, RuntimeObservedState, RuntimeOperationControl, RuntimePlanDecision,
    RuntimeResourceIdentity,
};
pub use factory::LocalRuntimeProviderFactory;
pub use provider::{
    LIVE_RUNTIME_METHODS, LocalRuntimeProvider, LocalRuntimeProviderBuildError,
    live_runtime_capabilities,
};
