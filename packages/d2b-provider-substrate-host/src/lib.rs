//! NixOS and Linux host substrate provider implementation boundary.

#![forbid(unsafe_code)]
// The canonical provider contract intentionally returns the closed ProviderFailure DTO by value.
#![allow(clippy::result_large_err)]

mod model;
mod port;
mod provider;

pub use model::{
    HostApplyInspection, HostApplyOutcome, HostCapability, HostCheckKind, HostCheckProfile,
    HostCheckReport, HostCheckSummary, HostDescriptorBinding, HostDiagnostic, HostEvidenceSource,
    HostFinding, HostFindingKind, HostFindingSeverity, HostKernelModule, HostModelError,
    HostOperationOwner, HostRemediationClass, HostRemediationId, HostRemediationPlan,
    HostRemediationPlanDisposition, HostSubstrateConfiguration, HostSubstrateInspection,
    HostSubstrateKind, HostSubstrateState, HostSupportEntry, HostSupportEvidence,
    HostSupportStatus, MAX_CHECK_FINDINGS, MAX_FINDING_DIAGNOSTICS, MAX_PLAN_FINDINGS,
    MAX_REPORT_DIAGNOSTICS,
};
pub use port::{HostCheckRequest, HostPlanRequest, HostPortError, HostSubstratePort};
pub use provider::{
    GenericLinuxSubstrateProvider, HostProviderConstructionError, LinuxSubstrateProvider,
    NixOsSubstrateProvider, NixosSubstrateProvider,
};
