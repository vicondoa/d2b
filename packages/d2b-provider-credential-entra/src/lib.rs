//! Entra credential provider for an exact co-located cloud consumer.
//!
//! The injected client retains all token material and is implemented in the
//! same provider-agent process as its SDK consumer. Only bounded opaque lease
//! metadata can leave this module.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    future::Future,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole},
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        AdoptionState, AgentPlacementBinding, AuthorizedProviderScope, CredentialLease,
        CredentialLeaseRequest, CredentialLeaseState, CredentialLeaseTransferPolicy,
        CredentialProvider, Generation, LeaseId, MAX_CREDENTIAL_OPERATION_CLASSES,
        MAX_PROVIDER_LEASE_LIFETIME_MS, MAX_SAFE_JSON_INTEGER, MutationReceipt, MutationState,
        ObservationReason, ObservedLifecycleState, OperationBinding, Provider, ProviderCallContext,
        ProviderCapability, ProviderCapabilitySet, ProviderContractError, ProviderDescriptor,
        ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHealth, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderObservation, ProviderOperationRequest,
        ProviderRemediation, RetryClass, SdkOperationClass, SourceVersion,
    },
};
use d2b_provider::{ProviderClock, SystemProviderClock};
use tokio::time::timeout;

pub const IMPLEMENTATION_ID: &str = "entra";
pub const MAX_LOCAL_LEASES: usize = 256;

/// The credential authority is owned by one exact co-located consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraCredentialOwner {
    ExactConsumer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraProviderError {
    InvalidDescriptor,
    InvalidConsumer,
    InvalidAuthorizedOperations,
    NotColocated,
}

impl fmt::Display for EntraProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor => "invalid Entra provider descriptor",
            Self::InvalidConsumer => "invalid Entra consumer descriptor",
            Self::InvalidAuthorizedOperations => "invalid Entra authorized operation set",
            Self::NotColocated => "Entra provider and consumer are not co-located",
        })
    }
}

impl Error for EntraProviderError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraClientState {
    Ready,
    InteractionRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraLeaseState {
    Active,
    Revoked,
    Expired,
}

/// Closed client failures. SDK response bodies and identity details are absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraClientError {
    InteractionRequired,
    Denied,
    Unavailable,
    Cancelled,
    DeadlineExpired,
    LeaseExpired,
    LeaseRevoked,
    CompletionUnknown,
}

impl fmt::Display for EntraClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InteractionRequired => "Entra interaction required",
            Self::Denied => "Entra operation denied",
            Self::Unavailable => "Entra client unavailable",
            Self::Cancelled => "Entra operation cancelled",
            Self::DeadlineExpired => "Entra operation deadline expired",
            Self::LeaseExpired => "Entra lease expired",
            Self::LeaseRevoked => "Entra lease revoked",
            Self::CompletionUnknown => "Entra mutation completion is unknown",
        })
    }
}

impl Error for EntraClientError {}

/// Exact-consumer request presented to the injected co-located client.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraLeaseRequest {
    pub operation: OperationBinding,
    pub credential_provider_id: ProviderId,
    pub credential_provider_generation: Generation,
    pub consumer_provider_id: ProviderId,
    pub consumer_provider_generation: Generation,
    pub agent_binding: AgentPlacementBinding,
    pub allowed_operations: BoundedVec<SdkOperationClass, 1, MAX_CREDENTIAL_OPERATION_CLASSES>,
    pub requested_expiry_unix_ms: u64,
}

impl fmt::Debug for EntraLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntraLeaseRequest")
            .field(
                "credential_provider_generation",
                &self.credential_provider_generation,
            )
            .field(
                "consumer_provider_generation",
                &self.consumer_provider_generation,
            )
            .field("agent_binding", &self.agent_binding)
            .field("operation_count", &self.allowed_operations.len())
            .field("requested_expiry_unix_ms", &self.requested_expiry_unix_ms)
            .finish_non_exhaustive()
    }
}

/// Opaque internal reference for inspect, refresh, and revoke.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraLeaseRef {
    pub lease_id: LeaseId,
    pub acquired_by: OperationBinding,
    pub credential_provider_id: ProviderId,
    pub credential_provider_generation: Generation,
    pub consumer_provider_id: ProviderId,
    pub consumer_provider_generation: Generation,
    pub agent_binding: AgentPlacementBinding,
    pub allowed_operations: BoundedVec<SdkOperationClass, 1, MAX_CREDENTIAL_OPERATION_CLASSES>,
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub requested_expiry_unix_ms: u64,
}

impl fmt::Debug for EntraLeaseRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntraLeaseRef")
            .field(
                "credential_provider_generation",
                &self.credential_provider_generation,
            )
            .field(
                "consumer_provider_generation",
                &self.consumer_provider_generation,
            )
            .field("agent_binding", &self.agent_binding)
            .field("operation_count", &self.allowed_operations.len())
            .field("rotation_generation", &self.rotation_generation)
            .field("requested_expiry_unix_ms", &self.requested_expiry_unix_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntraLeaseGrant {
    pub lease_id: LeaseId,
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntraLeaseInspection {
    pub state: EntraLeaseState,
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub expires_at_unix_ms: u64,
    pub revoked_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntraLeaseRenewal {
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraLeaseRevocation {
    Revoked { revoked_at_unix_ms: u64 },
    AlreadyRevoked { revoked_at_unix_ms: u64 },
}

/// Async client implemented beside the exact cloud SDK consumer.
///
/// It owns the token cache and performs SDK authorization locally. No token,
/// host keyring handle, endpoint, path, file descriptor, or byte buffer is
/// accepted or returned here.
#[async_trait]
pub trait EntraCredentialClient: Send + Sync {
    async fn state(&self) -> Result<EntraClientState, EntraClientError>;

    async fn issue_lease(
        &self,
        request: &EntraLeaseRequest,
    ) -> Result<EntraLeaseGrant, EntraClientError>;

    async fn inspect_lease(
        &self,
        lease: &EntraLeaseRef,
    ) -> Result<EntraLeaseInspection, EntraClientError>;

    async fn refresh_lease(
        &self,
        lease: &EntraLeaseRef,
    ) -> Result<EntraLeaseRenewal, EntraClientError>;

    async fn revoke_lease(
        &self,
        lease: &EntraLeaseRef,
    ) -> Result<EntraLeaseRevocation, EntraClientError>;
}

#[derive(Clone)]
struct LeaseRecord {
    lease: CredentialLease,
    acquired_by: OperationBinding,
}

pub struct EntraCredentialProvider {
    descriptor: ProviderDescriptor,
    consumer: ProviderDescriptor,
    agent_binding: AgentPlacementBinding,
    authorized_operations: Vec<SdkOperationClass>,
    client: Arc<dyn EntraCredentialClient>,
    clock: Arc<dyn ProviderClock>,
    leases: Mutex<BTreeMap<LeaseId, LeaseRecord>>,
    mutation_gate: tokio::sync::Mutex<()>,
}

impl fmt::Debug for EntraCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntraCredentialProvider")
            .field("owner", &EntraCredentialOwner::ExactConsumer)
            .field("generation", &self.descriptor.registry_generation)
            .field("agent_binding", &self.agent_binding)
            .field(
                "authorized_operation_count",
                &self.authorized_operations.len(),
            )
            .finish_non_exhaustive()
    }
}

impl EntraCredentialProvider {
    pub fn new_colocated(
        descriptor: ProviderDescriptor,
        consumer: ProviderDescriptor,
        authorized_operations: Vec<SdkOperationClass>,
        client: Arc<dyn EntraCredentialClient>,
    ) -> Result<Self, EntraProviderError> {
        Self::new_colocated_with_clock(
            descriptor,
            consumer,
            authorized_operations,
            client,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn new_colocated_with_clock(
        descriptor: ProviderDescriptor,
        consumer: ProviderDescriptor,
        mut authorized_operations: Vec<SdkOperationClass>,
        client: Arc<dyn EntraCredentialClient>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, EntraProviderError> {
        descriptor
            .validate()
            .map_err(|_| EntraProviderError::InvalidDescriptor)?;
        if descriptor.provider_type() != ProviderType::Credential
            || descriptor.implementation_id.as_str() != IMPLEMENTATION_ID
            || descriptor.capabilities != exact_capabilities()
        {
            return Err(EntraProviderError::InvalidDescriptor);
        }
        let agent_binding = descriptor
            .placement
            .agent_binding()
            .ok_or(EntraProviderError::InvalidDescriptor)?;

        consumer
            .validate()
            .map_err(|_| EntraProviderError::InvalidConsumer)?;
        if !consumer_type_can_hold_credential(consumer.provider_type())
            || consumer.provider_id == descriptor.provider_id
        {
            return Err(EntraProviderError::InvalidConsumer);
        }
        if consumer.placement.agent_binding().as_ref() != Some(&agent_binding) {
            return Err(EntraProviderError::NotColocated);
        }

        authorized_operations.sort_unstable();
        if authorized_operations.is_empty()
            || authorized_operations.len() > MAX_CREDENTIAL_OPERATION_CLASSES
            || authorized_operations
                .windows(2)
                .any(|pair| pair[0] == pair[1])
        {
            return Err(EntraProviderError::InvalidAuthorizedOperations);
        }

        Ok(Self {
            descriptor,
            consumer,
            agent_binding,
            authorized_operations,
            client,
            clock,
            leases: Mutex::new(BTreeMap::new()),
            mutation_gate: tokio::sync::Mutex::new(()),
        })
    }

    pub const fn owner(&self) -> EntraCredentialOwner {
        EntraCredentialOwner::ExactConsumer
    }

    pub fn consumer_descriptor(&self) -> &ProviderDescriptor {
        &self.consumer
    }

    fn now(&self) -> u64 {
        self.clock.now_unix_ms().min(MAX_SAFE_JSON_INTEGER)
    }

    fn failure(
        &self,
        context: &ProviderCallContext<'_>,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Credential,
            binding: context.operation.binding(),
            correlation_id: context.operation.correlation_id.clone(),
            occurred_at_unix_ms: self.now(),
            reason,
            remediation,
        }
    }

    fn invalid_request(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::CapabilityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn unauthorized(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::UnauthorizedScope,
            RetryClass::Never,
            ProviderHealthReason::IdentityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn invalid_lease(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::CredentialLeaseInvalid,
            RetryClass::Never,
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::InspectProvider,
        )
    }

    fn invariant_failure(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::InvariantViolation,
            RetryClass::Never,
            ProviderHealthReason::ProviderDegraded,
            ProviderRemediation::RestartAgent,
        )
    }

    fn queue_pressure(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::Unavailable,
            RetryClass::SameOperation,
            ProviderHealthReason::QueuePressure,
            ProviderRemediation::RetryBounded,
        )
    }

    fn mutation_guard(
        &self,
        context: &ProviderCallContext<'_>,
    ) -> Result<tokio::sync::MutexGuard<'_, ()>, ProviderFailure> {
        self.mutation_gate
            .try_lock()
            .map_err(|_| self.queue_pressure(context))
    }

    fn scope_matches_owner(&self, scope: &AuthorizedProviderScope) -> bool {
        match scope {
            AuthorizedProviderScope::Workload {
                realm_id,
                workload_id,
            } => {
                realm_id == &self.agent_binding.realm_id
                    && workload_id == &self.agent_binding.workload_id
            }
            AuthorizedProviderScope::WorkloadRole {
                realm_id,
                workload_id,
                role_id,
            } => {
                realm_id == &self.agent_binding.realm_id
                    && workload_id == &self.agent_binding.workload_id
                    && role_id == &self.agent_binding.role_id
            }
            AuthorizedProviderScope::Realm { .. } => false,
        }
    }

    fn preflight(
        &self,
        context: &ProviderCallContext<'_>,
        expected: ProviderMethod,
    ) -> Result<(), ProviderFailure> {
        if context.cancelled {
            return Err(self.failure(
                context,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.failure(
                context,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.validate().is_err()
            || context
                .operation
                .validate(&self.descriptor, self.now())
                .is_err()
            || context.operation.method != expected
        {
            return Err(self.invalid_request(context));
        }
        if !matches!(
            context.peer_role,
            EndpointRole::ProviderAgent | EndpointRole::RealmController
        ) || !self.scope_matches_owner(&context.operation.scope)
        {
            return Err(self.unauthorized(context));
        }
        Ok(())
    }

    async fn call_client<T, F>(
        &self,
        context: &ProviderCallContext<'_>,
        call: F,
    ) -> Result<T, ProviderFailure>
    where
        F: Future<Output = Result<T, EntraClientError>> + Send,
    {
        let deadline = Duration::from_millis(u64::from(context.monotonic_deadline_remaining_ms));
        let result = timeout(deadline, call).await.map_err(|_| {
            self.failure(
                context,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            )
        })?;
        let value = result.map_err(|error| self.map_client_error(context, error))?;
        if self.now() >= context.operation.expires_at_unix_ms {
            return Err(self.failure(
                context,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ));
        }
        Ok(value)
    }

    fn map_client_error(
        &self,
        context: &ProviderCallContext<'_>,
        error: EntraClientError,
    ) -> ProviderFailure {
        match error {
            EntraClientError::InteractionRequired => self.failure(
                context,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterInteraction,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::OperatorInteraction,
            ),
            EntraClientError::Denied => self.unauthorized(context),
            EntraClientError::Unavailable => self.failure(
                context,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            EntraClientError::Cancelled => self.failure(
                context,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            EntraClientError::DeadlineExpired => self.failure(
                context,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ),
            EntraClientError::LeaseExpired | EntraClientError::LeaseRevoked => {
                self.invalid_lease(context)
            }
            EntraClientError::CompletionUnknown => self.failure(
                context,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
        }
    }

    fn lock_leases(
        &self,
        context: &ProviderCallContext<'_>,
    ) -> Result<MutexGuard<'_, BTreeMap<LeaseId, LeaseRecord>>, ProviderFailure> {
        self.leases
            .lock()
            .map_err(|_| self.invariant_failure(context))
    }

    fn health_value(
        &self,
        state: EntraClientState,
    ) -> Result<ProviderHealth, ProviderContractError> {
        let (health_state, reason, remediation) = match state {
            EntraClientState::Ready => (
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            EntraClientState::InteractionRequired => (
                ProviderHealthState::Degraded,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::OperatorInteraction,
            ),
        };
        let health = ProviderHealth {
            provider_id: self.descriptor.provider_id.clone(),
            registry_generation: self.descriptor.registry_generation,
            observed_at_unix_ms: self.now(),
            state: health_state,
            reason,
            remediation,
        };
        health.validate()?;
        Ok(health)
    }

    fn status_observation(
        &self,
        request: &ProviderOperationRequest,
        state: EntraClientState,
    ) -> Result<ProviderObservation, ProviderContractError> {
        let health = self.health_value(state)?;
        let observation = ProviderObservation {
            provider_id: self.descriptor.provider_id.clone(),
            provider_generation: self.descriptor.registry_generation,
            realm_id: request.target.realm_id().clone(),
            workload_id: request.target.workload_id().cloned(),
            handle_id: None,
            resource_generation: None,
            observed_at_unix_ms: health.observed_at_unix_ms,
            lifecycle: match state {
                EntraClientState::Ready => ObservedLifecycleState::Ready,
                EntraClientState::InteractionRequired => ObservedLifecycleState::Stopped,
            },
            adoption: AdoptionState::NotAttempted,
            reason: ObservationReason::None,
            health,
        };
        observation.validate()?;
        Ok(observation)
    }

    fn operations_authorized(&self, operations: &[SdkOperationClass]) -> bool {
        !operations.is_empty()
            && operations.len() <= MAX_CREDENTIAL_OPERATION_CLASSES
            && !operations.windows(2).any(|pair| pair[0] >= pair[1])
            && operations
                .iter()
                .all(|operation| self.authorized_operations.binary_search(operation).is_ok())
    }

    fn validate_acquire_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &CredentialLeaseRequest,
    ) -> Result<(), ProviderFailure> {
        let now = self.now();
        if request.context != *context.operation
            || request.consumer_provider_id != self.consumer.provider_id
            || request.agent_binding != self.agent_binding
            || !self.operations_authorized(&request.allowed_operations)
            || request.requested_expiry_unix_ms <= now
            || request.requested_expiry_unix_ms > MAX_SAFE_JSON_INTEGER
            || request
                .requested_expiry_unix_ms
                .checked_sub(now)
                .is_none_or(|lifetime| lifetime > MAX_PROVIDER_LEASE_LIFETIME_MS)
        {
            return Err(self.unauthorized(context));
        }
        Ok(())
    }

    fn validate_lease_owner(
        &self,
        context: &ProviderCallContext<'_>,
        lease: &CredentialLease,
    ) -> Result<(), ProviderFailure> {
        if lease.credential_provider_id != self.descriptor.provider_id
            || lease.credential_provider_generation != self.descriptor.registry_generation
            || lease.consumer_provider_id != self.consumer.provider_id
            || lease.consumer_provider_generation != self.consumer.registry_generation
            || lease.agent_binding != self.agent_binding
            || lease.transfer_policy != CredentialLeaseTransferPolicy::Forbidden
            || !self.operations_authorized(&lease.allowed_operations)
        {
            return Err(self.unauthorized(context));
        }
        Ok(())
    }

    fn active_record(
        &self,
        context: &ProviderCallContext<'_>,
        lease: &CredentialLease,
    ) -> Result<LeaseRecord, ProviderFailure> {
        self.validate_lease_owner(context, lease)?;
        let now = self.now();
        let mut leases = self.lock_leases(context)?;
        let record = leases
            .get_mut(&lease.lease_id)
            .ok_or_else(|| self.invalid_lease(context))?;
        if record.lease.state == CredentialLeaseState::Active
            && now >= record.lease.expires_at_unix_ms
        {
            record.lease.state = CredentialLeaseState::Expired;
        }
        if record.lease != *lease || record.lease.state != CredentialLeaseState::Active {
            return Err(self.invalid_lease(context));
        }
        Ok(record.clone())
    }

    fn lease_ref(record: &LeaseRecord, requested_expiry_unix_ms: u64) -> EntraLeaseRef {
        EntraLeaseRef {
            lease_id: record.lease.lease_id.clone(),
            acquired_by: record.acquired_by.clone(),
            credential_provider_id: record.lease.credential_provider_id.clone(),
            credential_provider_generation: record.lease.credential_provider_generation,
            consumer_provider_id: record.lease.consumer_provider_id.clone(),
            consumer_provider_generation: record.lease.consumer_provider_generation,
            agent_binding: record.lease.agent_binding.clone(),
            allowed_operations: record.lease.allowed_operations.clone(),
            source_version: record.lease.source_version.clone(),
            rotation_generation: record.lease.rotation_generation,
            requested_expiry_unix_ms,
        }
    }

    fn validate_active_inspection(
        &self,
        context: &ProviderCallContext<'_>,
        record: &LeaseRecord,
        inspection: &EntraLeaseInspection,
    ) -> Result<(), ProviderFailure> {
        if inspection.state != EntraLeaseState::Active
            || inspection.source_version != record.lease.source_version
            || inspection.rotation_generation != record.lease.rotation_generation
            || inspection.expires_at_unix_ms != record.lease.expires_at_unix_ms
            || inspection.revoked_at_unix_ms.is_some()
        {
            return Err(self.invalid_lease(context));
        }
        Ok(())
    }

    fn receipt(&self, context: &ProviderCallContext<'_>, state: MutationState) -> MutationReceipt {
        MutationReceipt {
            binding: context.operation.binding(),
            state,
            observed_at_unix_ms: self.now(),
            observation_required_before_retry: false,
        }
    }
}

impl Provider for EntraCredentialProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialStatus)?;
            let state = self.call_client(context, self.client.state()).await?;
            self.health_value(state)
                .map_err(|_| self.invariant_failure(context))
        })
    }
}

impl CredentialProvider for EntraCredentialProvider {
    fn status<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialStatus)?;
            request
                .validate_method(
                    &self.descriptor,
                    self.now(),
                    ProviderMethod::CredentialStatus,
                )
                .map_err(|_| self.invalid_request(context))?;
            if !self.scope_matches_owner(&request.context.scope) {
                return Err(self.unauthorized(context));
            }
            let state = self.call_client(context, self.client.state()).await?;
            self.status_observation(request, state)
                .map_err(|_| self.invariant_failure(context))
        })
    }

    fn acquire_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a CredentialLeaseRequest,
    ) -> ProviderFuture<'a, CredentialLease> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialAcquireLease)?;
            self.validate_acquire_request(context, request)?;
            let _mutation = self.mutation_guard(context)?;
            let acquisition = request.context.binding();
            {
                let now = self.now();
                let mut leases = self.lock_leases(context)?;
                leases.retain(|_, record| {
                    record.lease.state == CredentialLeaseState::Active
                        && now < record.lease.expires_at_unix_ms
                });
                if let Some(record) = leases.values().find(|record| {
                    record.acquired_by.operation_id == acquisition.operation_id
                        || record.acquired_by.idempotency_key == acquisition.idempotency_key
                }) {
                    if record.acquired_by == acquisition
                        && record.lease.consumer_provider_id == request.consumer_provider_id
                        && record.lease.agent_binding == request.agent_binding
                        && record.lease.allowed_operations == request.allowed_operations
                        && record.lease.expires_at_unix_ms <= request.requested_expiry_unix_ms
                    {
                        return Ok(record.lease.clone());
                    }
                    return Err(self.invalid_request(context));
                }
                if leases.len() >= MAX_LOCAL_LEASES {
                    return Err(self.queue_pressure(context));
                }
            }
            let client_request = EntraLeaseRequest {
                operation: acquisition.clone(),
                credential_provider_id: self.descriptor.provider_id.clone(),
                credential_provider_generation: self.descriptor.registry_generation,
                consumer_provider_id: self.consumer.provider_id.clone(),
                consumer_provider_generation: self.consumer.registry_generation,
                agent_binding: self.agent_binding.clone(),
                allowed_operations: request.allowed_operations.clone(),
                requested_expiry_unix_ms: request.requested_expiry_unix_ms,
            };
            let grant = self
                .call_client(context, self.client.issue_lease(&client_request))
                .await?;
            let now = self.now();
            if grant.expires_at_unix_ms <= now
                || grant.expires_at_unix_ms > MAX_SAFE_JSON_INTEGER
                || grant.expires_at_unix_ms > request.requested_expiry_unix_ms
                || grant
                    .expires_at_unix_ms
                    .checked_sub(now)
                    .is_none_or(|lifetime| lifetime > MAX_PROVIDER_LEASE_LIFETIME_MS)
            {
                return Err(self.invariant_failure(context));
            }
            let lease = CredentialLease {
                lease_id: grant.lease_id,
                credential_provider_id: self.descriptor.provider_id.clone(),
                consumer_provider_id: self.consumer.provider_id.clone(),
                agent_binding: self.agent_binding.clone(),
                allowed_operations: request.allowed_operations.clone(),
                issued_at_unix_ms: now,
                expires_at_unix_ms: grant.expires_at_unix_ms,
                credential_provider_generation: self.descriptor.registry_generation,
                consumer_provider_generation: self.consumer.registry_generation,
                source_version: grant.source_version,
                rotation_generation: grant.rotation_generation,
                state: CredentialLeaseState::Active,
                transfer_policy: CredentialLeaseTransferPolicy::Forbidden,
                revoked_at_unix_ms: None,
            };
            lease
                .validate(&self.descriptor, &self.consumer, now)
                .map_err(|_| self.invariant_failure(context))?;
            let mut leases = self.lock_leases(context)?;
            if leases.contains_key(&lease.lease_id) {
                return Err(self.invariant_failure(context));
            }
            leases.insert(
                lease.lease_id.clone(),
                LeaseRecord {
                    lease: lease.clone(),
                    acquired_by: acquisition,
                },
            );
            Ok(lease)
        })
    }

    fn refresh_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        lease: &'a CredentialLease,
    ) -> ProviderFuture<'a, CredentialLease> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialRefreshLease)?;
            let _mutation = self.mutation_guard(context)?;
            let record = self.active_record(context, lease)?;
            let now = self.now();
            let lifetime = record
                .lease
                .expires_at_unix_ms
                .saturating_sub(record.lease.issued_at_unix_ms)
                .min(MAX_PROVIDER_LEASE_LIFETIME_MS);
            let requested_expiry = now
                .checked_add(lifetime)
                .filter(|expiry| *expiry <= MAX_SAFE_JSON_INTEGER)
                .ok_or_else(|| self.invalid_lease(context))?;
            let reference = Self::lease_ref(&record, requested_expiry);
            let inspection = self
                .call_client(context, self.client.inspect_lease(&reference))
                .await?;
            self.validate_active_inspection(context, &record, &inspection)?;
            let renewal = self
                .call_client(context, self.client.refresh_lease(&reference))
                .await?;
            let refreshed_at = self.now();
            if renewal.expires_at_unix_ms <= refreshed_at
                || renewal.expires_at_unix_ms > requested_expiry
                || renewal.rotation_generation < record.lease.rotation_generation
            {
                return Err(self.invariant_failure(context));
            }
            let mut refreshed = record.lease.clone();
            refreshed
                .refresh(
                    refreshed_at,
                    renewal.expires_at_unix_ms,
                    renewal.source_version,
                    renewal.rotation_generation,
                )
                .map_err(|_| self.invariant_failure(context))?;
            refreshed
                .validate(&self.descriptor, &self.consumer, refreshed_at)
                .map_err(|_| self.invariant_failure(context))?;
            let mut leases = self.lock_leases(context)?;
            let current = leases
                .get_mut(&refreshed.lease_id)
                .ok_or_else(|| self.invalid_lease(context))?;
            if current.lease != record.lease || current.acquired_by != record.acquired_by {
                return Err(self.invariant_failure(context));
            }
            current.lease = refreshed.clone();
            Ok(refreshed)
        })
    }

    fn revoke_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        lease: &'a CredentialLease,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialRevokeLease)?;
            let _mutation = self.mutation_guard(context)?;
            self.validate_lease_owner(context, lease)?;
            let now = self.now();
            let record = {
                let mut leases = self.lock_leases(context)?;
                let record = leases
                    .get_mut(&lease.lease_id)
                    .ok_or_else(|| self.invalid_lease(context))?;
                if record.lease.state == CredentialLeaseState::Active
                    && now >= record.lease.expires_at_unix_ms
                {
                    record.lease.state = CredentialLeaseState::Expired;
                }
                if record.lease.state == CredentialLeaseState::Revoked {
                    return Ok(self.receipt(context, MutationState::AlreadyApplied));
                }
                if record.lease != *lease || record.lease.state != CredentialLeaseState::Active {
                    return Err(self.invalid_lease(context));
                }
                record.clone()
            };
            let reference = Self::lease_ref(&record, record.lease.expires_at_unix_ms);
            let inspection = self
                .call_client(context, self.client.inspect_lease(&reference))
                .await?;
            match inspection.state {
                EntraLeaseState::Active => {
                    self.validate_active_inspection(context, &record, &inspection)?;
                }
                EntraLeaseState::Revoked => {
                    let revoked_at = inspection
                        .revoked_at_unix_ms
                        .ok_or_else(|| self.invariant_failure(context))?;
                    let mut leases = self.lock_leases(context)?;
                    let current = leases
                        .get_mut(&lease.lease_id)
                        .ok_or_else(|| self.invalid_lease(context))?;
                    current
                        .lease
                        .revoke(revoked_at)
                        .map_err(|_| self.invariant_failure(context))?;
                    return Ok(self.receipt(context, MutationState::AlreadyApplied));
                }
                EntraLeaseState::Expired => return Err(self.invalid_lease(context)),
            }
            let revocation = self
                .call_client(context, self.client.revoke_lease(&reference))
                .await?;
            let (state, revoked_at) = match revocation {
                EntraLeaseRevocation::Revoked { revoked_at_unix_ms } => {
                    (MutationState::Applied, revoked_at_unix_ms)
                }
                EntraLeaseRevocation::AlreadyRevoked { revoked_at_unix_ms } => {
                    (MutationState::AlreadyApplied, revoked_at_unix_ms)
                }
            };
            let mut leases = self.lock_leases(context)?;
            let current = leases
                .get_mut(&lease.lease_id)
                .ok_or_else(|| self.invalid_lease(context))?;
            if current.lease != record.lease || current.acquired_by != record.acquired_by {
                return Err(self.invariant_failure(context));
            }
            current
                .lease
                .revoke(revoked_at)
                .map_err(|_| self.invariant_failure(context))?;
            Ok(self.receipt(context, state))
        })
    }
}

fn exact_capabilities() -> ProviderCapabilitySet {
    ProviderCapabilitySet::new(vec![
        ProviderCapability(ProviderMethod::CredentialStatus),
        ProviderCapability(ProviderMethod::CredentialAcquireLease),
        ProviderCapability(ProviderMethod::CredentialRefreshLease),
        ProviderCapability(ProviderMethod::CredentialRevokeLease),
    ])
    .unwrap_or_else(|_| unreachable!())
}

const fn consumer_type_can_hold_credential(provider_type: ProviderType) -> bool {
    matches!(
        provider_type,
        ProviderType::Runtime
            | ProviderType::Infrastructure
            | ProviderType::Transport
            | ProviderType::Network
            | ProviderType::Storage
            | ProviderType::Observability
    )
}

#[cfg(test)]
mod tests;
