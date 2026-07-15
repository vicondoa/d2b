use async_trait::async_trait;
use d2b_contracts::v2_provider::{Fingerprint, OperationBinding};

use crate::{
    HostApplyOutcome, HostCheckReport, HostDescriptorBinding, HostOperationOwner,
    HostRemediationId, HostRemediationPlan, HostSubstrateConfiguration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCheckRequest {
    configuration: HostSubstrateConfiguration,
    descriptor: HostDescriptorBinding,
    owner: HostOperationOwner,
    operation: OperationBinding,
    deadline_remaining_ms: u32,
}

impl HostCheckRequest {
    pub(crate) fn new(
        configuration: HostSubstrateConfiguration,
        descriptor: HostDescriptorBinding,
        owner: HostOperationOwner,
        operation: OperationBinding,
        deadline_remaining_ms: u32,
    ) -> Self {
        Self {
            configuration,
            descriptor,
            owner,
            operation,
            deadline_remaining_ms,
        }
    }

    pub const fn configuration(&self) -> HostSubstrateConfiguration {
        self.configuration
    }

    pub fn descriptor(&self) -> &HostDescriptorBinding {
        &self.descriptor
    }

    pub fn owner(&self) -> &HostOperationOwner {
        &self.owner
    }

    pub fn operation(&self) -> &OperationBinding {
        &self.operation
    }

    pub const fn deadline_remaining_ms(&self) -> u32 {
        self.deadline_remaining_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPlanRequest {
    configuration: HostSubstrateConfiguration,
    descriptor: HostDescriptorBinding,
    owner: HostOperationOwner,
    operation: OperationBinding,
    latest_report_fingerprint: Option<Fingerprint>,
    deadline_remaining_ms: u32,
}

impl HostPlanRequest {
    pub(crate) fn new(
        configuration: HostSubstrateConfiguration,
        descriptor: HostDescriptorBinding,
        owner: HostOperationOwner,
        operation: OperationBinding,
        latest_report_fingerprint: Option<Fingerprint>,
        deadline_remaining_ms: u32,
    ) -> Self {
        Self {
            configuration,
            descriptor,
            owner,
            operation,
            latest_report_fingerprint,
            deadline_remaining_ms,
        }
    }

    pub const fn configuration(&self) -> HostSubstrateConfiguration {
        self.configuration
    }

    pub fn descriptor(&self) -> &HostDescriptorBinding {
        &self.descriptor
    }

    pub fn owner(&self) -> &HostOperationOwner {
        &self.owner
    }

    pub fn operation(&self) -> &OperationBinding {
        &self.operation
    }

    pub fn latest_report_fingerprint(&self) -> Option<&Fingerprint> {
        self.latest_report_fingerprint.as_ref()
    }

    pub const fn deadline_remaining_ms(&self) -> u32 {
        self.deadline_remaining_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPortError {
    Denied,
    Unavailable,
    StaleGeneration,
    Cancelled,
    DeadlineExpired,
    InvalidResponse,
}

#[async_trait]
pub trait HostSubstratePort: Send + Sync {
    /// Gather bounded semantic evidence without mutating host state.
    async fn check(&self, request: HostCheckRequest) -> Result<HostCheckReport, HostPortError>;

    /// Authorize and bind a remediation plan without applying it.
    async fn plan_remediation(
        &self,
        request: HostPlanRequest,
    ) -> Result<HostRemediationPlan, HostPortError>;

    /// The daemon-owned port resolves and authorizes this opaque ID. Repeated
    /// calls with one ID must not duplicate a mutation. An `Err` must mean no
    /// mutation occurred; ambiguity is an explicit outcome.
    async fn apply(
        &self,
        remediation_id: HostRemediationId,
    ) -> Result<HostApplyOutcome, HostPortError>;
}
