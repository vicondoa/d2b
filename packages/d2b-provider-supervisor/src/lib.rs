//! Core-owned Process Provider supervisor.
//!
//! [`ProviderSupervisor`] is the async [`ProcessLaunchEffectPort`] adapter. It
//! keeps blocking broker, service-manager, kernel, and filesystem operations
//! off the controller executor, retains local process authority, and exposes
//! only opaque conformance results to Process Providers.

#![deny(missing_docs)]

mod adapter;
mod broker;
mod systemd;

pub use adapter::{DEFAULT_BLOCKING_LIMIT, ProviderSupervisor};
pub use broker::{
    BrokerLaunchIntent, BrokerLaunchResolver, BrokerObservedProcess, BrokerPidfdHandle,
    BrokerProcessBackend,
};
pub use systemd::{
    SystemdEffectLaunch, SystemdEffectOwner, SystemdInvocationIdentity, SystemdProcessBackend,
};
