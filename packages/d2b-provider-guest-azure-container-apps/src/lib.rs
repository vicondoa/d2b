//! Canonical `Provider/runtime-azure-container-apps` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod controller;
#[allow(missing_docs)]
mod effects;

pub use controller::{
    AcaClock, AcaController, AcaControllerError, AcaPhase, AcaReconcileOutcome, AcaRecoveryState,
    AcaStatus, AzureContainerAppsRuntimeProvider, CompletedOperationLedger, SystemAcaClock,
    ACA_GUEST_FINALIZER, ACA_REPAIR_INTERVAL_SECS,
};
pub use effects::*;

/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-azure-container-apps";
/// Stable Guest finalizer.
pub const FINALIZER: &str = ACA_GUEST_FINALIZER;
