//! Canonical `Provider/runtime-azure-container-apps` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod audit;
mod controller;
mod deployment_service;
#[allow(missing_docs)]
mod effects;
mod metrics;

pub use audit::{AcaAuditEvent, AcaAuditOutcome, AcaAuditSink};
pub use controller::{
    AcaClock, AcaController, AcaControllerError, AcaPhase, AcaReconcileOutcome, AcaRecoveryState,
    AcaStatus, AzureContainerAppsRuntimeProvider, CompletedOperationLedger, SystemAcaClock,
};
pub use effects::*;
pub use deployment_service::{
    AcaDeploymentRequest, AcaDeploymentResponse, AcaDeploymentService, AcaServiceError,
    AcaServiceMethod,
};
pub use metrics::{AcaMetricEvent, AcaMetricOutcome, AcaMetricValidationError};

/// Stable Provider implementation identifier.
pub const ACA_IMPLEMENTATION_ID: &str = "azure-container-apps";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-azure-container-apps";
