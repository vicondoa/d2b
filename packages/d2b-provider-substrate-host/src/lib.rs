//! NixOS and Linux host substrate provider implementation boundary.

#![forbid(unsafe_code)]
// The canonical provider contract intentionally returns the closed ProviderFailure DTO by value.
#![allow(clippy::result_large_err)]

mod factory;
mod model;
mod port;
mod provider;

pub use factory::HostSubstrateProviderFactory;
pub use model::{
    HostApplyInspection, HostApplyOutcome, HostCapability, HostCheckKind, HostCheckProfile,
    HostCheckReport, HostCheckSummary, HostDescriptorBinding, HostDiagnostic, HostEvidenceSource,
    HostFinding, HostFindingKind, HostFindingSeverity, HostKernelModule, HostModelError,
    HostOperationOwner, HostRemediationClass, HostRemediationId, HostRemediationPlan,
    HostRemediationPlanDisposition, HostSubstrateConfiguration, HostSubstrateInspection,
    HostSubstrateKind, HostSubstrateState, HostSupportEntry, HostSupportEvidence,
    HostSupportStatus, LINUX_IMPLEMENTATION_ID, MAX_CHECK_FINDINGS, MAX_FINDING_DIAGNOSTICS,
    MAX_PLAN_FINDINGS, MAX_REPORT_DIAGNOSTICS, NIXOS_IMPLEMENTATION_ID,
};
pub use port::{HostCheckRequest, HostPlanRequest, HostPortError, HostSubstratePort};
pub use provider::{
    GenericLinuxSubstrateProvider, HostProviderConstructionError, LinuxSubstrateProvider,
    NixOsSubstrateProvider, NixosSubstrateProvider,
};
