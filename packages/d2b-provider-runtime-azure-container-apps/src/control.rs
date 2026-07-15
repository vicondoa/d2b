//! Typed in-process boundaries to the co-located credential module and ACA SDK
//! adapter. Implementations redeem only opaque lease IDs inside their shared
//! process; this interface has no credential-byte or raw SDK payload channel.

use std::{error::Error, fmt};

use async_trait::async_trait;
use d2b_contracts::v2_provider::{
    CredentialLease, OperationBinding, ProviderDescriptor, ProviderMethod,
    ProviderOperationContext, SdkOperationClass,
};

use crate::types::{
    AcaDeleteOutcome, AcaDesiredDiskImage, AcaDesiredSandbox, AcaDiskImageCandidates,
    AcaDiskImageRecord, AcaSandboxCandidates, AcaSandboxId, AcaSandboxRecord, AcaWorkloadQuery,
};

pub const MAX_ACA_RETRY_AFTER_MS: u32 = 5 * 60 * 1_000;

const HEALTH_OPERATIONS: &[SdkOperationClass] =
    &[SdkOperationClass::Authenticate, SdkOperationClass::Read];
const ENSURE_OPERATIONS: &[SdkOperationClass] = &[
    SdkOperationClass::Authenticate,
    SdkOperationClass::Discover,
    SdkOperationClass::Read,
    SdkOperationClass::Create,
];
const START_OPERATIONS: &[SdkOperationClass] = &[
    SdkOperationClass::Authenticate,
    SdkOperationClass::Discover,
    SdkOperationClass::Read,
    SdkOperationClass::Power,
];
const STOP_OPERATIONS: &[SdkOperationClass] = START_OPERATIONS;
const INSPECT_OPERATIONS: &[SdkOperationClass] = &[
    SdkOperationClass::Authenticate,
    SdkOperationClass::Discover,
    SdkOperationClass::Read,
];
const ADOPT_OPERATIONS: &[SdkOperationClass] = INSPECT_OPERATIONS;
const DESTROY_OPERATIONS: &[SdkOperationClass] = &[
    SdkOperationClass::Authenticate,
    SdkOperationClass::Discover,
    SdkOperationClass::Read,
    SdkOperationClass::Delete,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaCredentialPurpose {
    Health,
    Ensure,
    Start,
    Stop,
    Inspect,
    Adopt,
    Destroy,
}

impl AcaCredentialPurpose {
    pub const fn required_operations(self) -> &'static [SdkOperationClass] {
        match self {
            Self::Health => HEALTH_OPERATIONS,
            Self::Ensure => ENSURE_OPERATIONS,
            Self::Start => START_OPERATIONS,
            Self::Stop => STOP_OPERATIONS,
            Self::Inspect => INSPECT_OPERATIONS,
            Self::Adopt => ADOPT_OPERATIONS,
            Self::Destroy => DESTROY_OPERATIONS,
        }
    }
}

#[derive(Clone)]
pub struct AcaCredentialLeaseRequest {
    operation: ProviderOperationContext,
    purpose: AcaCredentialPurpose,
    requested_expiry_unix_ms: u64,
}

impl AcaCredentialLeaseRequest {
    pub(crate) fn new(
        operation: ProviderOperationContext,
        purpose: AcaCredentialPurpose,
        requested_expiry_unix_ms: u64,
    ) -> Self {
        Self {
            operation,
            purpose,
            requested_expiry_unix_ms,
        }
    }

    pub fn operation(&self) -> &ProviderOperationContext {
        &self.operation
    }

    pub const fn purpose(&self) -> AcaCredentialPurpose {
        self.purpose
    }

    pub fn required_operations(&self) -> &'static [SdkOperationClass] {
        self.purpose.required_operations()
    }

    pub const fn requested_expiry_unix_ms(&self) -> u64 {
        self.requested_expiry_unix_ms
    }
}

impl fmt::Debug for AcaCredentialLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaCredentialLeaseRequest")
            .field("purpose", &self.purpose)
            .field("operation_count", &self.required_operations().len())
            .finish_non_exhaustive()
    }
}

pub struct AcaCredentialLease {
    metadata: CredentialLease,
}

impl AcaCredentialLease {
    pub fn from_canonical(metadata: CredentialLease) -> Self {
        Self { metadata }
    }

    pub fn metadata(&self) -> &CredentialLease {
        &self.metadata
    }
}

impl fmt::Debug for AcaCredentialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AcaCredentialLease(<opaque>)")
    }
}

#[async_trait]
/// Co-located credential module used only to issue opaque canonical leases.
pub trait AcaCredentialLeaseClient: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;

    async fn acquire(
        &self,
        request: &AcaCredentialLeaseRequest,
    ) -> Result<AcaCredentialLease, AcaControlError>;
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaControlContext {
    binding: OperationBinding,
    method: ProviderMethod,
    operation_class: SdkOperationClass,
    deadline_remaining_ms: u32,
}

impl AcaControlContext {
    pub(crate) fn new(
        binding: OperationBinding,
        method: ProviderMethod,
        operation_class: SdkOperationClass,
        deadline_remaining_ms: u32,
    ) -> Self {
        Self {
            binding,
            method,
            operation_class,
            deadline_remaining_ms,
        }
    }

    pub fn binding(&self) -> &OperationBinding {
        &self.binding
    }

    pub const fn method(&self) -> ProviderMethod {
        self.method
    }

    pub const fn operation_class(&self) -> SdkOperationClass {
        self.operation_class
    }

    pub const fn deadline_remaining_ms(&self) -> u32 {
        self.deadline_remaining_ms
    }
}

impl fmt::Debug for AcaControlContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaControlContext")
            .field("method", &self.method)
            .field("operation_class", &self.operation_class)
            .field("deadline_remaining_ms", &self.deadline_remaining_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaControlHealth {
    Ready,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaControlErrorKind {
    Authentication,
    Authorization,
    RateLimited,
    Unavailable,
    Conflict,
    NotFound,
    InvalidResponse,
    Cancelled,
    DeadlineExpired,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaDiagnosticCode {
    None,
    AuthenticationFailed,
    AuthorizationFailed,
    TooManyRequests,
    ResourceConflict,
    ResourceMissing,
    InvalidResponse,
    ServiceUnavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaControlErrorBuildError {
    RetryBoundExceeded,
    RetryNotApplicable,
}

impl fmt::Display for AcaControlErrorBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RetryBoundExceeded => {
                "Azure Container Apps retry interval exceeds the supported bound"
            }
            Self::RetryNotApplicable => {
                "Azure Container Apps retry interval is invalid for this failure class"
            }
        })
    }
}

impl Error for AcaControlErrorBuildError {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AcaControlError {
    kind: AcaControlErrorKind,
    diagnostic: AcaDiagnosticCode,
    retry_after_ms: Option<u32>,
}

impl AcaControlError {
    pub fn new(
        kind: AcaControlErrorKind,
        diagnostic: AcaDiagnosticCode,
        retry_after_ms: Option<u32>,
    ) -> Result<Self, AcaControlErrorBuildError> {
        if retry_after_ms.is_some() && kind != AcaControlErrorKind::RateLimited {
            return Err(AcaControlErrorBuildError::RetryNotApplicable);
        }
        if retry_after_ms.is_some_and(|retry| retry == 0 || retry > MAX_ACA_RETRY_AFTER_MS) {
            return Err(AcaControlErrorBuildError::RetryBoundExceeded);
        }
        Ok(Self {
            kind,
            diagnostic,
            retry_after_ms,
        })
    }

    pub const fn closed(kind: AcaControlErrorKind, diagnostic: AcaDiagnosticCode) -> Self {
        Self {
            kind,
            diagnostic,
            retry_after_ms: None,
        }
    }

    pub const fn kind(self) -> AcaControlErrorKind {
        self.kind
    }

    pub const fn diagnostic(self) -> AcaDiagnosticCode {
        self.diagnostic
    }

    pub const fn retry_after_ms(self) -> Option<u32> {
        self.retry_after_ms
    }
}

impl fmt::Debug for AcaControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaControlError")
            .field("kind", &self.kind)
            .field("diagnostic", &self.diagnostic)
            .field("retry_after_ms", &self.retry_after_ms)
            .finish()
    }
}

impl fmt::Display for AcaControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            AcaControlErrorKind::Authentication => "aca-control-authentication",
            AcaControlErrorKind::Authorization => "aca-control-authorization",
            AcaControlErrorKind::RateLimited => "aca-control-rate-limited",
            AcaControlErrorKind::Unavailable => "aca-control-unavailable",
            AcaControlErrorKind::Conflict => "aca-control-conflict",
            AcaControlErrorKind::NotFound => "aca-control-not-found",
            AcaControlErrorKind::InvalidResponse => "aca-control-invalid-response",
            AcaControlErrorKind::Cancelled => "aca-control-cancelled",
            AcaControlErrorKind::DeadlineExpired => "aca-control-deadline-expired",
            AcaControlErrorKind::Ambiguous => "aca-control-ambiguous",
        })
    }
}

impl Error for AcaControlError {}

#[async_trait]
/// Closed ACA lifecycle port implemented by the credential-owning provider agent.
pub trait AcaControl: Send + Sync {
    async fn health(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
    ) -> Result<AcaControlHealth, AcaControlError>;

    async fn find_sandboxes(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        query: &AcaWorkloadQuery,
    ) -> Result<AcaSandboxCandidates, AcaControlError>;

    async fn find_disk_images(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageCandidates, AcaControlError>;

    async fn resolve_configured_disk(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError>;

    async fn create_disk_image(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError>;

    async fn create_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredSandbox,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    async fn resume_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    async fn stop_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    async fn delete_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaDeleteOutcome, AcaControlError>;
}
