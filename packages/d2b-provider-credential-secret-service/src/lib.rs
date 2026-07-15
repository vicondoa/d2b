//! User-session Secret Service credential provider.
//!
//! The provider is constructed only for a `d2b-userd` owner and talks to an
//! injected semantic Secret Service port. The port owns all interaction with
//! `oo7` and all credential material. This crate handles only bounded status
//! and opaque lease metadata.

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
        AdoptionState, AuthorizedProviderScope, CredentialLease, CredentialLeaseRequest,
        CredentialLeaseState, CredentialLeaseTransferPolicy, CredentialPlacementBinding,
        CredentialProvider, Generation, ImplementationId, LeaseId,
        MAX_CREDENTIAL_OPERATION_CLASSES, MAX_PROVIDER_LEASE_LIFETIME_MS, MAX_SAFE_JSON_INTEGER,
        MutationReceipt, MutationState, ObservationReason, ObservedLifecycleState,
        OperationBinding, Provider, ProviderCallContext, ProviderCapability, ProviderCapabilitySet,
        ProviderContractError, ProviderDescriptor, ProviderFactoryKey, ProviderFailure,
        ProviderFailureKind, ProviderFuture, ProviderHealth, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderObservation, ProviderOperationRequest,
        ProviderRemediation, RetryClass, SdkOperationClass, SourceVersion,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};
use tokio::time::timeout;

/// Canonical implementation ID advertised by this provider.
pub const IMPLEMENTATION_ID: &str = "secret-service";
pub const MAX_LOCAL_LEASES: usize = 256;

/// Canonical typed implementation identifier.
pub fn implementation_id() -> ImplementationId {
    ImplementationId::parse(IMPLEMENTATION_ID).unwrap_or_else(|_| unreachable!())
}

/// Canonical registry key for the Secret Service implementation.
pub fn provider_factory_key() -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Credential,
        implementation_id: implementation_id(),
    }
}

/// The only permitted process owner for this provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceOwner {
    /// The exact user's `d2b-userd` process.
    Userd,
}

/// Construction failures contain only closed, non-sensitive classifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceProviderError {
    InvalidDescriptor,
    InvalidConsumer,
    InvalidAuthorizedOperations,
    NotColocated,
}

impl fmt::Display for SecretServiceProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor => "invalid Secret Service provider descriptor",
            Self::InvalidConsumer => "invalid Secret Service consumer descriptor",
            Self::InvalidAuthorizedOperations => "invalid Secret Service authorized operation set",
            Self::NotColocated => "Secret Service provider and consumer are not co-located",
        })
    }
}

impl Error for SecretServiceProviderError {}

/// Closed Secret Service lock state. No collection or item data is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceState {
    Locked,
    Unlocked,
}

/// Closed backend lease state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceLeaseState {
    Active,
    Revoked,
    Expired,
}

/// Closed errors returned by the semantic Secret Service port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServicePortError {
    Locked,
    Denied,
    Unavailable,
    Cancelled,
    DeadlineExpired,
    LeaseExpired,
    LeaseRevoked,
    CompletionUnknown,
}

impl fmt::Display for SecretServicePortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Locked => "Secret Service is locked",
            Self::Denied => "Secret Service operation denied",
            Self::Unavailable => "Secret Service unavailable",
            Self::Cancelled => "Secret Service operation cancelled",
            Self::DeadlineExpired => "Secret Service operation deadline expired",
            Self::LeaseExpired => "Secret Service lease expired",
            Self::LeaseRevoked => "Secret Service lease revoked",
            Self::CompletionUnknown => "Secret Service mutation completion is unknown",
        })
    }
}

impl Error for SecretServicePortError {}

/// Owner-, generation-, and operation-bound request passed to the `oo7` port.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseRequest {
    pub operation: OperationBinding,
    pub credential_provider_id: ProviderId,
    pub credential_provider_generation: Generation,
    pub consumer_provider_id: ProviderId,
    pub consumer_provider_generation: Generation,
    pub placement_binding: CredentialPlacementBinding,
    pub allowed_operations: BoundedVec<SdkOperationClass, 1, MAX_CREDENTIAL_OPERATION_CLASSES>,
    pub requested_expiry_unix_ms: u64,
}

impl fmt::Debug for SecretServiceLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretServiceLeaseRequest")
            .field(
                "credential_provider_generation",
                &self.credential_provider_generation,
            )
            .field(
                "consumer_provider_generation",
                &self.consumer_provider_generation,
            )
            .field("placement_binding", &self.placement_binding)
            .field("operation_count", &self.allowed_operations.len())
            .field("requested_expiry_unix_ms", &self.requested_expiry_unix_ms)
            .finish_non_exhaustive()
    }
}

/// Opaque lease reference used for semantic inspect, refresh, and revoke calls.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseRef {
    pub lease_id: LeaseId,
    pub acquired_by: OperationBinding,
    pub credential_provider_id: ProviderId,
    pub credential_provider_generation: Generation,
    pub consumer_provider_id: ProviderId,
    pub consumer_provider_generation: Generation,
    pub placement_binding: CredentialPlacementBinding,
    pub allowed_operations: BoundedVec<SdkOperationClass, 1, MAX_CREDENTIAL_OPERATION_CLASSES>,
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub requested_expiry_unix_ms: u64,
}

impl fmt::Debug for SecretServiceLeaseRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretServiceLeaseRef")
            .field(
                "credential_provider_generation",
                &self.credential_provider_generation,
            )
            .field(
                "consumer_provider_generation",
                &self.consumer_provider_generation,
            )
            .field("placement_binding", &self.placement_binding)
            .field("operation_count", &self.allowed_operations.len())
            .field("rotation_generation", &self.rotation_generation)
            .field("requested_expiry_unix_ms", &self.requested_expiry_unix_ms)
            .finish_non_exhaustive()
    }
}

/// Metadata produced after the port has retained credential material locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseGrant {
    pub lease_id: LeaseId,
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub expires_at_unix_ms: u64,
}

/// Metadata returned by a semantic lease inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseInspection {
    pub state: SecretServiceLeaseState,
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub expires_at_unix_ms: u64,
    pub revoked_at_unix_ms: Option<u64>,
}

/// Metadata returned after an in-place refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseRenewal {
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub expires_at_unix_ms: u64,
}

/// Idempotent semantic revoke result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceLeaseRevocation {
    Revoked { revoked_at_unix_ms: u64 },
    AlreadyRevoked { revoked_at_unix_ms: u64 },
}

/// Async semantic boundary implemented by the `d2b-userd` `oo7` adapter.
///
/// None of these methods accepts or returns a password, secret value, token,
/// endpoint, path, file descriptor, or byte buffer.
#[async_trait]
pub trait Oo7SecretServicePort: Send + Sync {
    async fn state(&self) -> Result<SecretServiceState, SecretServicePortError>;

    async fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> Result<SecretServiceLeaseGrant, SecretServicePortError>;

    async fn inspect_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseInspection, SecretServicePortError>;

    async fn refresh_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseRenewal, SecretServicePortError>;

    async fn revoke_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseRevocation, SecretServicePortError>;
}

/// Registry factory bound to one userd consumer and semantic `oo7` port.
#[derive(Clone)]
pub struct SecretServiceCredentialProviderFactory {
    consumer: ProviderDescriptor,
    authorized_operations: Vec<SdkOperationClass>,
    port: Arc<dyn Oo7SecretServicePort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for SecretServiceCredentialProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretServiceCredentialProviderFactory")
            .field("provider_type", &ProviderType::Credential)
            .field("consumer_type", &self.consumer.provider_type())
            .field("consumer_generation", &self.consumer.registry_generation)
            .field(
                "authorized_operation_count",
                &self.authorized_operations.len(),
            )
            .finish_non_exhaustive()
    }
}

impl SecretServiceCredentialProviderFactory {
    /// Construct a production-clock factory for `ProviderRegistryBuilder`.
    pub fn new(
        consumer: ProviderDescriptor,
        authorized_operations: Vec<SdkOperationClass>,
        port: Arc<dyn Oo7SecretServicePort>,
    ) -> Result<Self, SecretServiceProviderError> {
        Self::new_with_clock(
            consumer,
            authorized_operations,
            port,
            Arc::new(SystemProviderClock),
        )
    }

    /// Construct a factory with an injected clock.
    pub fn new_with_clock(
        consumer: ProviderDescriptor,
        mut authorized_operations: Vec<SdkOperationClass>,
        port: Arc<dyn Oo7SecretServicePort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, SecretServiceProviderError> {
        consumer
            .validate()
            .map_err(|_| SecretServiceProviderError::InvalidConsumer)?;
        if !consumer_type_can_hold_credential(consumer.provider_type())
            || !matches!(
                consumer.placement.credential_binding(),
                Some(CredentialPlacementBinding::UserAgent { .. })
            )
        {
            return Err(SecretServiceProviderError::InvalidConsumer);
        }
        authorized_operations.sort_unstable();
        if authorized_operations.is_empty()
            || authorized_operations.len() > MAX_CREDENTIAL_OPERATION_CLASSES
            || authorized_operations
                .windows(2)
                .any(|pair| pair[0] == pair[1])
        {
            return Err(SecretServiceProviderError::InvalidAuthorizedOperations);
        }
        Ok(Self {
            consumer,
            authorized_operations,
            port,
            clock,
        })
    }

    /// Return the canonical registry key accepted by this factory.
    pub fn key() -> ProviderFactoryKey {
        provider_factory_key()
    }
}

impl ProviderFactory for SecretServiceCredentialProviderFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Credential
            || descriptor.implementation_id != implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let provider = SecretServiceCredentialProvider::new_userd_with_clock(
            descriptor.clone(),
            self.consumer.clone(),
            self.authorized_operations.clone(),
            self.port.clone(),
            self.clock.clone(),
        )
        .map_err(|_| FactoryError::Rejected)?;
        Ok(ProviderInstance::Credential(Arc::new(provider)))
    }
}

#[derive(Clone)]
struct LeaseRecord {
    lease: CredentialLease,
    acquired_by: OperationBinding,
}

/// Canonical `CredentialProvider` hosted by the exact user's `d2b-userd`.
pub struct SecretServiceCredentialProvider {
    descriptor: ProviderDescriptor,
    consumer: ProviderDescriptor,
    placement_binding: CredentialPlacementBinding,
    authorized_operations: Vec<SdkOperationClass>,
    port: Arc<dyn Oo7SecretServicePort>,
    clock: Arc<dyn ProviderClock>,
    leases: Mutex<BTreeMap<LeaseId, LeaseRecord>>,
    mutation_gate: tokio::sync::Mutex<()>,
}

impl fmt::Debug for SecretServiceCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretServiceCredentialProvider")
            .field("owner", &SecretServiceOwner::Userd)
            .field("generation", &self.descriptor.registry_generation)
            .field("placement_binding", &self.placement_binding)
            .field(
                "authorized_operation_count",
                &self.authorized_operations.len(),
            )
            .finish_non_exhaustive()
    }
}

impl SecretServiceCredentialProvider {
    /// Construct the provider for `d2b-userd` with an explicit `oo7` adapter.
    pub fn new_userd(
        descriptor: ProviderDescriptor,
        consumer: ProviderDescriptor,
        authorized_operations: Vec<SdkOperationClass>,
        port: Arc<dyn Oo7SecretServicePort>,
    ) -> Result<Self, SecretServiceProviderError> {
        Self::new_userd_with_clock(
            descriptor,
            consumer,
            authorized_operations,
            port,
            Arc::new(SystemProviderClock),
        )
    }

    /// Construct with an injected clock for deterministic lifecycle handling.
    pub fn new_userd_with_clock(
        descriptor: ProviderDescriptor,
        consumer: ProviderDescriptor,
        mut authorized_operations: Vec<SdkOperationClass>,
        port: Arc<dyn Oo7SecretServicePort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, SecretServiceProviderError> {
        descriptor
            .validate()
            .map_err(|_| SecretServiceProviderError::InvalidDescriptor)?;
        if descriptor.provider_type() != ProviderType::Credential
            || descriptor.implementation_id != implementation_id()
            || descriptor.capabilities != exact_capabilities()
        {
            return Err(SecretServiceProviderError::InvalidDescriptor);
        }
        let placement_binding = descriptor
            .placement
            .credential_binding()
            .filter(|binding| matches!(binding, CredentialPlacementBinding::UserAgent { .. }))
            .ok_or(SecretServiceProviderError::InvalidDescriptor)?;

        consumer
            .validate()
            .map_err(|_| SecretServiceProviderError::InvalidConsumer)?;
        if !consumer_type_can_hold_credential(consumer.provider_type())
            || consumer.provider_id == descriptor.provider_id
        {
            return Err(SecretServiceProviderError::InvalidConsumer);
        }
        if consumer.placement.credential_binding().as_ref() != Some(&placement_binding) {
            return Err(SecretServiceProviderError::NotColocated);
        }

        authorized_operations.sort_unstable();
        if authorized_operations.is_empty()
            || authorized_operations.len() > MAX_CREDENTIAL_OPERATION_CLASSES
            || authorized_operations
                .windows(2)
                .any(|pair| pair[0] == pair[1])
        {
            return Err(SecretServiceProviderError::InvalidAuthorizedOperations);
        }

        Ok(Self {
            descriptor,
            consumer,
            placement_binding,
            authorized_operations,
            port,
            clock,
            leases: Mutex::new(BTreeMap::new()),
            mutation_gate: tokio::sync::Mutex::new(()),
        })
    }

    pub const fn owner(&self) -> SecretServiceOwner {
        SecretServiceOwner::Userd
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
        let CredentialPlacementBinding::UserAgent { realm_id, .. } = &self.placement_binding else {
            return false;
        };
        matches!(
            scope,
            AuthorizedProviderScope::Realm {
                realm_id: scoped_realm
            } if scoped_realm == realm_id
        )
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
        if context.peer_role != EndpointRole::UserAgent
            || !self.scope_matches_owner(&context.operation.scope)
        {
            return Err(self.unauthorized(context));
        }
        Ok(())
    }

    async fn call_port<T, F>(
        &self,
        context: &ProviderCallContext<'_>,
        call: F,
    ) -> Result<T, ProviderFailure>
    where
        F: Future<Output = Result<T, SecretServicePortError>> + Send,
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
        let value = result.map_err(|error| self.map_port_error(context, error))?;
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

    fn map_port_error(
        &self,
        context: &ProviderCallContext<'_>,
        error: SecretServicePortError,
    ) -> ProviderFailure {
        match error {
            SecretServicePortError::Locked => self.failure(
                context,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterInteraction,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::OperatorInteraction,
            ),
            SecretServicePortError::Denied => self.unauthorized(context),
            SecretServicePortError::Unavailable => self.failure(
                context,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            SecretServicePortError::Cancelled => self.failure(
                context,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            SecretServicePortError::DeadlineExpired => self.failure(
                context,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ),
            SecretServicePortError::LeaseExpired | SecretServicePortError::LeaseRevoked => {
                self.invalid_lease(context)
            }
            SecretServicePortError::CompletionUnknown => self.failure(
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
        state: SecretServiceState,
    ) -> Result<ProviderHealth, ProviderContractError> {
        let (health_state, reason, remediation) = match state {
            SecretServiceState::Unlocked => (
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            SecretServiceState::Locked => (
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
        state: SecretServiceState,
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
                SecretServiceState::Unlocked => ObservedLifecycleState::Ready,
                SecretServiceState::Locked => ObservedLifecycleState::Stopped,
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
            || request.placement_binding != self.placement_binding
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
            || lease.placement_binding != self.placement_binding
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

    fn lease_ref(record: &LeaseRecord, requested_expiry_unix_ms: u64) -> SecretServiceLeaseRef {
        SecretServiceLeaseRef {
            lease_id: record.lease.lease_id.clone(),
            acquired_by: record.acquired_by.clone(),
            credential_provider_id: record.lease.credential_provider_id.clone(),
            credential_provider_generation: record.lease.credential_provider_generation,
            consumer_provider_id: record.lease.consumer_provider_id.clone(),
            consumer_provider_generation: record.lease.consumer_provider_generation,
            placement_binding: record.lease.placement_binding.clone(),
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
        inspection: &SecretServiceLeaseInspection,
    ) -> Result<(), ProviderFailure> {
        if inspection.state != SecretServiceLeaseState::Active
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

impl Provider for SecretServiceCredentialProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialStatus)?;
            let state = self.call_port(context, self.port.state()).await?;
            self.health_value(state)
                .map_err(|_| self.invariant_failure(context))
        })
    }
}

impl CredentialProvider for SecretServiceCredentialProvider {
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
            let state = self.call_port(context, self.port.state()).await?;
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
                        && record.lease.placement_binding == request.placement_binding
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

            let port_request = SecretServiceLeaseRequest {
                operation: acquisition.clone(),
                credential_provider_id: self.descriptor.provider_id.clone(),
                credential_provider_generation: self.descriptor.registry_generation,
                consumer_provider_id: self.consumer.provider_id.clone(),
                consumer_provider_generation: self.consumer.registry_generation,
                placement_binding: self.placement_binding.clone(),
                allowed_operations: request.allowed_operations.clone(),
                requested_expiry_unix_ms: request.requested_expiry_unix_ms,
            };
            let grant = self
                .call_port(context, self.port.issue_lease(&port_request))
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
                placement_binding: self.placement_binding.clone(),
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
                .call_port(context, self.port.inspect_lease(&reference))
                .await?;
            self.validate_active_inspection(context, &record, &inspection)?;

            let renewal = self
                .call_port(context, self.port.refresh_lease(&reference))
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
                .call_port(context, self.port.inspect_lease(&reference))
                .await?;
            match inspection.state {
                SecretServiceLeaseState::Active => {
                    self.validate_active_inspection(context, &record, &inspection)?;
                }
                SecretServiceLeaseState::Revoked => {
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
                SecretServiceLeaseState::Expired => return Err(self.invalid_lease(context)),
            }

            let revocation = self
                .call_port(context, self.port.revoke_lease(&reference))
                .await?;
            let (state, revoked_at) = match revocation {
                SecretServiceLeaseRevocation::Revoked { revoked_at_unix_ms } => {
                    (MutationState::Applied, revoked_at_unix_ms)
                }
                SecretServiceLeaseRevocation::AlreadyRevoked { revoked_at_unix_ms } => {
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
