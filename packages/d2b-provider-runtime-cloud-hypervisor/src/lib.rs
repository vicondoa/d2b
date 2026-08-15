//! Canonical `Provider/runtime-cloud-hypervisor` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod adoption;
pub mod audit;
pub mod bootstrap_graph;
pub mod config;
pub mod controller;
pub mod health;
pub mod metrics;
pub mod state;
pub mod vmm_argv;

pub use config::{CloudHypervisorConfig, CloudHypervisorGuestSettings, ConsoleType};
pub use controller::{
    CloudHypervisorClock, CloudHypervisorController, CloudHypervisorEffectPort,
    CloudHypervisorError, CloudHypervisorPhase, CloudHypervisorReconcileOutcome,
    CloudHypervisorRecoveryState, SystemCloudHypervisorClock,
};
pub use health::{GuestControlHealth, GuestControlHealthError, GuestControlProbe};
pub use vmm_argv::{ChArgvError, ChArgvInput, ChNetIface, ChVsock, generate_ch_argv};

/// Stable Provider implementation identifier.
pub const CLOUD_HYPERVISOR_IMPLEMENTATION_ID: &str = "cloud-hypervisor";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-cloud-hypervisor";
