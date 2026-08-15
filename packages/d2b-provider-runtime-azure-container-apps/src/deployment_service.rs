//! Bounded ACA deployment service dispatch.

use std::sync::Arc;

use d2b_contracts::provider_effects::aca::{
    AcaControl, AcaControlContext, AcaControlHealth, AcaCredentialLease, AcaCredentialLeaseClient,
    AcaCredentialLeaseRequest, AcaCredentialPurpose, AcaDeleteOutcome, AcaDesiredDiskImage,
    AcaDesiredSandbox, AcaDiskImageRecord, AcaOperationId, AcaProfileId, AcaResourceBinding,
    AcaSandboxCandidates, AcaSandboxId, AcaSandboxProfile, AcaSandboxRecord, AcaTypeError,
    AcaWorkloadQuery,
};

use crate::controller::{AcaClock, SystemAcaClock};
use tokio::sync::Semaphore;
use tokio::time::{Duration, Instant, timeout_at};

/// Methods exported by the ACA deployment service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaServiceMethod {
    /// Provision a sandbox.
    GuestProvision,
    /// Start a sandbox.
    GuestStart,
    /// Stop a sandbox.
    GuestStop,
    /// Destroy a sandbox.
    GuestDestroy,
    /// Adopt an existing sandbox.
    GuestAdopt,
    /// Inspect sandbox state.
    GuestInspect,
    /// Probe ACA environment health.
    GuestHealth,
}

/// Request to the deployment service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcaDeploymentRequest {
    /// Find candidate sandboxes.
    Find {
        /// Operation identity.
        operation_id: AcaOperationId,
        /// Query binding.
        query: AcaWorkloadQuery,
    },
    /// Ensure the disk image and sandbox for one bound Guest.
    Provision {
        /// Operation identity.
        operation_id: AcaOperationId,
        /// Provider binding.
        binding: AcaResourceBinding,
        /// Desired sandbox profile.
        profile: Box<AcaSandboxProfile>,
        /// Desired disk image.
        disk_image: AcaDesiredDiskImage,
        /// Optional provider network reference.
        network_ref: Option<d2b_contracts::v3::ResourceRef>,
        /// Provider-selected sandbox transport alias.
        sandbox_transport_alias: AcaProfileId,
    },
    /// Start a candidate.
    Start {
        /// Operation identity.
        operation_id: AcaOperationId,
        /// Sandbox handle.
        sandbox_id: AcaSandboxId,
    },
    /// Stop a candidate.
    Stop {
        /// Operation identity.
        operation_id: AcaOperationId,
        /// Sandbox handle.
        sandbox_id: AcaSandboxId,
    },
    /// Delete a candidate.
    Destroy {
        /// Operation identity.
        operation_id: AcaOperationId,
        /// Sandbox handle.
        sandbox_id: AcaSandboxId,
    },
    /// Probe the authenticated ACA control endpoint.
    Health {
        /// Operation identity.
        operation_id: AcaOperationId,
    },
}

/// Response from the deployment service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcaDeploymentResponse {
    /// Candidate list.
    Candidates(AcaSandboxCandidates),
    /// Disk image record.
    DiskImage(AcaDiskImageRecord),
    /// Sandbox record.
    Sandbox(AcaSandboxRecord),
    /// Delete outcome.
    Deleted(AcaDeleteOutcome),
    /// Health result.
    Health(AcaControlHealth),
}

/// Deployment service failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaServiceError {
    /// The request was not authorized for the configured gateway.
    ExecutionBoundaryDenied,
    /// The requested method does not match the request shape.
    MethodMismatch,
    /// The effect port failed.
    Effect(crate::AcaControlErrorKind),
    /// The request was malformed.
    InvalidRequest,
}

impl std::fmt::Display for AcaServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ExecutionBoundaryDenied => "aca-service-execution-boundary-denied",
            Self::MethodMismatch => "aca-service-method-mismatch",
            Self::Effect(kind) => kind.code(),
            Self::InvalidRequest => "aca-service-invalid-request",
        })
    }
}

impl std::error::Error for AcaServiceError {}

/// The service-side ACA effect dispatcher.
pub struct AcaDeploymentService<C, L> {
    control: Arc<C>,
    leases: Arc<L>,
    max_in_flight: usize,
    in_flight: Arc<Semaphore>,
    clock: Arc<dyn AcaClock>,
}

impl<C, L> AcaDeploymentService<C, L>
where
    C: AcaControl + 'static,
    L: AcaCredentialLeaseClient + 'static,
{
    /// Construct a bounded deployment service.
    pub fn new(control: Arc<C>, leases: Arc<L>) -> Self {
        Self {
            control,
            leases,
            max_in_flight: 64,
            in_flight: Arc::new(Semaphore::new(64)),
            clock: Arc::new(SystemAcaClock),
        }
    }

    /// Replace the wall clock used for lease expiry.
    pub fn with_clock(mut self, clock: Arc<dyn AcaClock>) -> Self {
        self.clock = clock;
        self
    }

    /// Return the fixed service concurrency bound.
    pub const fn max_in_flight(&self) -> usize {
        self.max_in_flight
    }

    /// Dispatch one request using one short-lived credential lease.
    pub async fn dispatch(
        &self,
        method: AcaServiceMethod,
        request: AcaDeploymentRequest,
        deadline_remaining_ms: u32,
    ) -> Result<AcaDeploymentResponse, AcaServiceError> {
        if deadline_remaining_ms == 0 {
            return Err(AcaServiceError::Effect(
                crate::AcaControlErrorKind::DeadlineExpired,
            ));
        }
        let deadline = Instant::now() + Duration::from_millis(u64::from(deadline_remaining_ms));
        let permit = timeout_at(deadline, self.in_flight.clone().acquire_owned())
            .await
            .map_err(|_| AcaServiceError::Effect(crate::AcaControlErrorKind::DeadlineExpired))?
            .map_err(|_| AcaServiceError::Effect(crate::AcaControlErrorKind::Unavailable))?;
        let (operation_id, purpose) = request_binding(&request, method)?;
        let context = AcaControlContext::new(operation_id.clone(), deadline_remaining_ms);
        let lease_request = AcaCredentialLeaseRequest::new(
            operation_id,
            purpose,
            self.clock
                .now_unix_ms()
                .saturating_add(u64::from(deadline_remaining_ms)),
        );
        let lease = timeout_at(deadline, self.leases.acquire(&lease_request))
            .await
            .map_err(|_| AcaServiceError::Effect(crate::AcaControlErrorKind::DeadlineExpired))?
            .map_err(|error| AcaServiceError::Effect(error.kind()))?;
        if lease.expires_at_unix_ms() <= self.clock.now_unix_ms()
            || lease.expires_at_unix_ms() < lease_request.requested_expiry_unix_ms()
        {
            let _ = timeout_at(deadline, self.leases.revoke(&lease)).await;
            drop(permit);
            return Err(AcaServiceError::Effect(
                crate::AcaControlErrorKind::DeadlineExpired,
            ));
        }
        let response = timeout_at(
            deadline,
            self.dispatch_with_lease(method, request, &context, &lease),
        )
        .await
        .map_err(|_| AcaServiceError::Effect(crate::AcaControlErrorKind::DeadlineExpired))
        .and_then(|result| result);
        let revoked = timeout_at(deadline, self.leases.revoke(&lease))
            .await
            .map_err(|_| AcaServiceError::Effect(crate::AcaControlErrorKind::DeadlineExpired))
            .and_then(|result| result.map_err(|error| AcaServiceError::Effect(error.kind())));
        let result = match (response, revoked) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        };
        drop(permit);
        result
    }

    async fn dispatch_with_lease(
        &self,
        method: AcaServiceMethod,
        request: AcaDeploymentRequest,
        context: &AcaControlContext,
        lease: &AcaCredentialLease,
    ) -> Result<AcaDeploymentResponse, AcaServiceError> {
        match (method, request) {
            (
                AcaServiceMethod::GuestProvision,
                AcaDeploymentRequest::Provision {
                    binding,
                    profile,
                    disk_image,
                    network_ref,
                    sandbox_transport_alias,
                    ..
                },
            ) => {
                let candidates = self
                    .control
                    .find_disk_images(lease, context, &disk_image)
                    .await
                    .map_err(|error| AcaServiceError::Effect(error.kind()))?;
                let image = match candidates.as_slice() {
                    [] => self
                        .control
                        .create_disk_image(lease, context, &disk_image)
                        .await
                        .map_err(|error| AcaServiceError::Effect(error.kind()))?,
                    [record] if record.generation == binding.provider_generation => record.clone(),
                    [_] => {
                        return Err(AcaServiceError::Effect(
                            crate::AcaControlErrorKind::Conflict,
                        ));
                    }
                    _ => {
                        return Err(AcaServiceError::Effect(
                            crate::AcaControlErrorKind::Ambiguous,
                        ));
                    }
                };
                self.control
                    .create_sandbox(
                        lease,
                        context,
                        &AcaDesiredSandbox {
                            binding,
                            profile: *profile,
                            disk_image: image,
                            network_ref,
                            sandbox_transport_alias,
                        },
                    )
                    .await
                    .map(AcaDeploymentResponse::Sandbox)
                    .map_err(|error| AcaServiceError::Effect(error.kind()))
            }
            (
                AcaServiceMethod::GuestInspect | AcaServiceMethod::GuestAdopt,
                AcaDeploymentRequest::Find { query, .. },
            ) => self
                .control
                .find_sandboxes(lease, context, &query)
                .await
                .map(AcaDeploymentResponse::Candidates)
                .map_err(|error| AcaServiceError::Effect(error.kind())),
            (AcaServiceMethod::GuestStart, AcaDeploymentRequest::Start { sandbox_id, .. }) => self
                .control
                .resume_sandbox(lease, context, &sandbox_id)
                .await
                .map(AcaDeploymentResponse::Sandbox)
                .map_err(|error| AcaServiceError::Effect(error.kind())),
            (AcaServiceMethod::GuestStop, AcaDeploymentRequest::Stop { sandbox_id, .. }) => self
                .control
                .stop_sandbox(lease, context, &sandbox_id)
                .await
                .map(AcaDeploymentResponse::Sandbox)
                .map_err(|error| AcaServiceError::Effect(error.kind())),
            (AcaServiceMethod::GuestDestroy, AcaDeploymentRequest::Destroy { sandbox_id, .. }) => {
                self.control
                    .delete_sandbox(lease, context, &sandbox_id)
                    .await
                    .map(AcaDeploymentResponse::Deleted)
                    .map_err(|error| AcaServiceError::Effect(error.kind()))
            }
            (AcaServiceMethod::GuestHealth, AcaDeploymentRequest::Health { .. }) => self
                .control
                .health(lease, context)
                .await
                .map(AcaDeploymentResponse::Health)
                .map_err(|error| AcaServiceError::Effect(error.kind())),
            _ => Err(AcaServiceError::MethodMismatch),
        }
    }
}

fn request_binding(
    request: &AcaDeploymentRequest,
    method: AcaServiceMethod,
) -> Result<(AcaOperationId, AcaCredentialPurpose), AcaServiceError> {
    let (operation_id, purpose) = match (method, request) {
        (
            AcaServiceMethod::GuestProvision,
            AcaDeploymentRequest::Provision { operation_id, .. },
        ) => (operation_id.clone(), AcaCredentialPurpose::Ensure),
        (AcaServiceMethod::GuestInspect, AcaDeploymentRequest::Find { operation_id, .. }) => {
            (operation_id.clone(), AcaCredentialPurpose::Inspect)
        }
        (AcaServiceMethod::GuestAdopt, AcaDeploymentRequest::Find { operation_id, .. }) => {
            (operation_id.clone(), AcaCredentialPurpose::Adopt)
        }
        (AcaServiceMethod::GuestStart, AcaDeploymentRequest::Start { operation_id, .. }) => {
            (operation_id.clone(), AcaCredentialPurpose::Start)
        }
        (AcaServiceMethod::GuestStop, AcaDeploymentRequest::Stop { operation_id, .. }) => {
            (operation_id.clone(), AcaCredentialPurpose::Stop)
        }
        (AcaServiceMethod::GuestDestroy, AcaDeploymentRequest::Destroy { operation_id, .. }) => {
            (operation_id.clone(), AcaCredentialPurpose::Destroy)
        }
        (AcaServiceMethod::GuestHealth, AcaDeploymentRequest::Health { operation_id }) => {
            (operation_id.clone(), AcaCredentialPurpose::Health)
        }
        _ => return Err(AcaServiceError::MethodMismatch),
    };
    Ok((operation_id, purpose))
}

impl From<AcaTypeError> for AcaServiceError {
    fn from(_: AcaTypeError) -> Self {
        Self::InvalidRequest
    }
}
