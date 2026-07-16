//! Local storage provider implementation boundary.
//!
//! Nix and the generated bundle remain the sole authority for paths, ownership,
//! modes, ACLs, repair policy, and synchronization policy. This crate accepts
//! only opaque generated storage identifiers and forwards semantic operations
//! to an injected async effect port. It never calls a broker.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{collections::BTreeMap, error::Error, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::BoundedVec,
    v2_identity::{ProviderId, ProviderType, RealmId, WorkloadId},
    v2_provider::{
        AdoptionRequest, AdoptionState, AuthorizedProviderScope, Fingerprint, Generation, HandleId,
        HandleOwner, IdempotencyKey, ImplementationId, MAX_PROVIDER_PLAN_RESOURCES,
        MutationReceipt, MutationState, ObservationReason, ObservedLifecycleState,
        OperationBinding, PlanId, PlannedResourceClass, PrincipalRef, Provider,
        ProviderCallContext, ProviderCapabilitySet, ProviderDescriptor, ProviderFactoryKey,
        ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle, ProviderHealth,
        ProviderHealthReason, ProviderHealthState, ProviderMethod, ProviderObservation,
        ProviderOperationInput, ProviderOperationRequest, ProviderPlacement, ProviderPlan,
        ProviderRemediation, ProviderResult, ProviderTarget, RetryClass, StorageSnapshotId,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, StorageProvider,
    SystemProviderClock,
};
use d2b_provider_toolkit::ProviderValues;
use tokio::sync::Mutex;

const MAX_TRACKED_OPERATIONS: usize = 128;
const PLAN_TTL_MS: u64 = 30_000;
pub const IMPLEMENTATION_ID: &str = "local";

pub fn implementation_id() -> ImplementationId {
    ImplementationId::parse(IMPLEMENTATION_ID)
        .unwrap_or_else(|_| unreachable!("local implementation ID is valid"))
}

pub fn provider_factory_key() -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Storage,
        implementation_id: implementation_id(),
    }
}

macro_rules! opaque_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, OpaqueIdError> {
                let value = value.into();
                let valid = !value.is_empty()
                    && value.len() <= 64
                    && value.as_bytes()[0].is_ascii_lowercase()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    });
                if valid {
                    Ok(Self(value))
                } else {
                    Err(OpaqueIdError)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&"<redacted>")
                    .finish()
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpaqueIdError;

impl fmt::Display for OpaqueIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid opaque storage identifier")
    }
}

impl Error for OpaqueIdError {}

opaque_id!(
    GeneratedStorageId,
    "An opaque generated storage contract identifier."
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageLiveCapabilities {
    pub local_state: bool,
    pub disk_images: bool,
    pub store_view: bool,
    pub closure_sync: bool,
    pub media: bool,
    pub snapshots: bool,
}

impl StorageLiveCapabilities {
    pub const REQUIRED: Self = Self {
        local_state: true,
        disk_images: true,
        store_view: true,
        closure_sync: true,
        media: true,
        snapshots: true,
    };

    const fn is_complete(self) -> bool {
        self.local_state
            && self.disk_images
            && self.store_view
            && self.closure_sync
            && self.media
            && self.snapshots
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LocalStorageBinding {
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub local_state_id: GeneratedStorageId,
    pub disk_set_id: GeneratedStorageId,
    pub store_view_id: GeneratedStorageId,
    pub closure_sync_id: GeneratedStorageId,
    pub media_set_id: GeneratedStorageId,
    pub resource_generation: Generation,
}

impl fmt::Debug for LocalStorageBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalStorageBinding")
            .field("resource_generation", &self.resource_generation)
            .finish_non_exhaustive()
    }
}

impl LocalStorageBinding {
    fn owner(&self) -> HandleOwner {
        HandleOwner::RealmController {
            realm_id: self.realm_id.clone(),
        }
    }

    fn resources_are_distinct(&self) -> bool {
        let resources = [
            &self.local_state_id,
            &self.disk_set_id,
            &self.store_view_id,
            &self.closure_sync_id,
            &self.media_set_id,
        ];
        resources
            .iter()
            .enumerate()
            .all(|(index, resource)| resources[index + 1..].iter().all(|other| resource != other))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageResourceBinding {
    pub provider_id: ProviderId,
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub owner: HandleOwner,
    pub provider_generation: Generation,
    pub resource_generation: Generation,
    pub configuration_fingerprint: Fingerprint,
}

impl fmt::Debug for StorageResourceBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageResourceBinding")
            .field("provider_generation", &self.provider_generation)
            .field("resource_generation", &self.resource_generation)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageTopology {
    pub local_state_id: GeneratedStorageId,
    pub disk_set_id: GeneratedStorageId,
    pub store_view_id: GeneratedStorageId,
    pub closure_sync_id: GeneratedStorageId,
    pub media_set_id: GeneratedStorageId,
}

impl fmt::Debug for StorageTopology {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageTopology")
            .field("semantic_resource_count", &5)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct StorageEffectContext {
    pub operation: OperationBinding,
    pub scope: AuthorizedProviderScope,
    pub principal: PrincipalRef,
    pub authorization_decision_digest: Fingerprint,
    pub resource: StorageResourceBinding,
    pub deadline_remaining_ms: u32,
}

impl fmt::Debug for StorageEffectContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageEffectContext")
            .field("provider_generation", &self.operation.provider_generation)
            .field("resource", &self.resource)
            .field("deadline_remaining_ms", &self.deadline_remaining_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct StorageEffectRequest {
    pub context: StorageEffectContext,
    pub topology: StorageTopology,
    pub plan_id: Option<PlanId>,
    pub handle_id: Option<HandleId>,
}

impl fmt::Debug for StorageEffectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageEffectRequest")
            .field("context", &self.context)
            .field("topology", &self.topology)
            .field("has_plan", &self.plan_id.is_some())
            .field("has_handle", &self.handle_id.is_some())
            .finish()
    }
}

#[derive(Clone)]
pub struct StorageSnapshotRequest {
    pub effect: StorageEffectRequest,
    pub snapshot_id: StorageSnapshotId,
}

impl fmt::Debug for StorageSnapshotRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageSnapshotRequest")
            .field("effect", &self.effect)
            .field("snapshot_id", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageEffectPlan {
    pub plan_id: PlanId,
    pub resource: StorageResourceBinding,
}

impl fmt::Debug for StorageEffectPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageEffectPlan")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageEffectHandle {
    pub handle_id: HandleId,
    pub resource: StorageResourceBinding,
}

impl fmt::Debug for StorageEffectHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageEffectHandle")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEffectHealth {
    Healthy,
    Degraded,
    Unavailable,
    Failed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageEffectObservation {
    pub resource: StorageResourceBinding,
    pub lifecycle: ObservedLifecycleState,
    pub health: StorageEffectHealth,
}

impl fmt::Debug for StorageEffectObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageEffectObservation")
            .field("resource", &self.resource)
            .field("lifecycle", &self.lifecycle)
            .field("health", &self.health)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageAdoptionRejection {
    IdentityMismatch,
    ConfigurationMismatch,
    GenerationMismatch,
    OwnerMismatch,
    MissingEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageAdoptionState {
    Adopted { lifecycle: ObservedLifecycleState },
    Rejected(StorageAdoptionRejection),
    Ambiguous,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageAdoptionOutcome {
    pub resource: StorageResourceBinding,
    pub state: StorageAdoptionState,
}

impl fmt::Debug for StorageAdoptionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageAdoptionOutcome")
            .field("resource", &self.resource)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageMutationOutcome {
    pub resource: StorageResourceBinding,
    pub state: MutationState,
}

impl fmt::Debug for StorageMutationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageMutationOutcome")
            .field("resource", &self.resource)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEffectError {
    Unavailable,
    Rejected,
    Ambiguous,
    Cancelled,
}

impl fmt::Display for StorageEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "storage effect unavailable",
            Self::Rejected => "storage effect rejected",
            Self::Ambiguous => "storage effect completion ambiguous",
            Self::Cancelled => "storage effect cancelled",
        })
    }
}

impl Error for StorageEffectError {}

#[async_trait]
pub trait StorageEffectPort: Send + Sync {
    fn live_capabilities(&self) -> StorageLiveCapabilities;

    async fn health(
        &self,
        request: &StorageEffectRequest,
    ) -> Result<StorageEffectHealth, StorageEffectError>;

    async fn plan(
        &self,
        request: &StorageEffectRequest,
    ) -> Result<StorageEffectPlan, StorageEffectError>;

    async fn ensure(
        &self,
        request: &StorageEffectRequest,
    ) -> Result<StorageEffectHandle, StorageEffectError>;

    async fn inspect(
        &self,
        request: &StorageEffectRequest,
    ) -> Result<StorageEffectObservation, StorageEffectError>;

    async fn adopt(
        &self,
        request: &StorageEffectRequest,
    ) -> Result<StorageAdoptionOutcome, StorageEffectError>;

    async fn snapshot(
        &self,
        request: &StorageSnapshotRequest,
    ) -> Result<StorageEffectHandle, StorageEffectError>;

    async fn destroy(
        &self,
        request: &StorageEffectRequest,
    ) -> Result<StorageMutationOutcome, StorageEffectError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBuildError {
    InvalidDescriptor,
    WrongProviderType,
    WrongImplementation,
    WrongPlacement,
    ScopeMismatch,
    DuplicateResource,
    MissingLiveCapability,
}

impl fmt::Display for StorageBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor => "invalid storage provider descriptor",
            Self::WrongProviderType => "descriptor is not a storage provider",
            Self::WrongImplementation => "storage implementation is not local",
            Self::WrongPlacement => "local storage provider must run in the owning controller",
            Self::ScopeMismatch => "storage binding is outside the configured provider scope",
            Self::DuplicateResource => "storage semantic resource identifiers must be distinct",
            Self::MissingLiveCapability => "required live local-storage capability is unavailable",
        })
    }
}

impl Error for StorageBuildError {}

#[derive(Clone)]
pub struct LocalStorageFactory {
    binding: LocalStorageBinding,
    effects: Arc<dyn StorageEffectPort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for LocalStorageFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalStorageFactory")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl LocalStorageFactory {
    pub fn new(binding: LocalStorageBinding, effects: Arc<dyn StorageEffectPort>) -> Self {
        Self::with_clock(binding, effects, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        binding: LocalStorageBinding,
        effects: Arc<dyn StorageEffectPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Self {
        Self {
            binding,
            effects,
            clock,
        }
    }
}

impl ProviderFactory for LocalStorageFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Storage
            || descriptor.implementation_id != implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let provider = LocalStorageProvider::with_clock(
            descriptor.clone(),
            self.binding.clone(),
            self.effects.clone(),
            self.clock.clone(),
        )
        .map_err(|error| match error {
            StorageBuildError::MissingLiveCapability => FactoryError::Unavailable,
            _ => FactoryError::Rejected,
        })?;
        Ok(ProviderInstance::Storage(Arc::new(provider)))
    }
}

#[derive(Clone)]
enum CachedResult {
    Plan {
        method: ProviderMethod,
        digest: Fingerprint,
        value: Box<ProviderPlan>,
    },
    Handle {
        method: ProviderMethod,
        digest: Fingerprint,
        value: Box<ProviderHandle>,
    },
    Receipt {
        method: ProviderMethod,
        digest: Fingerprint,
        value: Box<MutationReceipt>,
    },
}

impl CachedResult {
    fn matches(&self, method: ProviderMethod, digest: &Fingerprint) -> bool {
        match self {
            Self::Plan {
                method: cached,
                digest: cached_digest,
                ..
            }
            | Self::Handle {
                method: cached,
                digest: cached_digest,
                ..
            }
            | Self::Receipt {
                method: cached,
                digest: cached_digest,
                ..
            } => *cached == method && cached_digest == digest,
        }
    }
}

#[derive(Default)]
struct ProviderState {
    operations: BTreeMap<IdempotencyKey, CachedResult>,
    handles: BTreeMap<HandleId, ProviderHandle>,
}

pub struct LocalStorageProvider {
    descriptor: ProviderDescriptor,
    binding: LocalStorageBinding,
    effects: Arc<dyn StorageEffectPort>,
    clock: Arc<dyn ProviderClock>,
    state: Mutex<ProviderState>,
}

impl fmt::Debug for LocalStorageProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalStorageProvider")
            .field("descriptor", &self.descriptor)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl LocalStorageProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        binding: LocalStorageBinding,
        effects: Arc<dyn StorageEffectPort>,
    ) -> Result<Self, StorageBuildError> {
        Self::with_clock(descriptor, binding, effects, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        binding: LocalStorageBinding,
        effects: Arc<dyn StorageEffectPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, StorageBuildError> {
        descriptor
            .validate()
            .map_err(|_| StorageBuildError::InvalidDescriptor)?;
        if descriptor.provider_type() != ProviderType::Storage {
            return Err(StorageBuildError::WrongProviderType);
        }
        if descriptor.implementation_id != implementation_id() {
            return Err(StorageBuildError::WrongImplementation);
        }
        if !matches!(
            descriptor.placement,
            ProviderPlacement::TrustedFirstPartyInProcess { .. }
        ) {
            return Err(StorageBuildError::WrongPlacement);
        }
        if descriptor.placement.realm_id() != &binding.realm_id {
            return Err(StorageBuildError::ScopeMismatch);
        }
        if !binding.resources_are_distinct() {
            return Err(StorageBuildError::DuplicateResource);
        }
        if !effects.live_capabilities().is_complete() {
            return Err(StorageBuildError::MissingLiveCapability);
        }
        Ok(Self {
            descriptor,
            binding,
            effects,
            clock,
            state: Mutex::new(ProviderState::default()),
        })
    }

    fn now(&self) -> u64 {
        self.clock.now_unix_ms()
    }

    fn resource_binding(&self) -> StorageResourceBinding {
        StorageResourceBinding {
            provider_id: self.descriptor.provider_id.clone(),
            realm_id: self.binding.realm_id.clone(),
            workload_id: self.binding.workload_id.clone(),
            owner: self.binding.owner(),
            provider_generation: self.descriptor.registry_generation,
            resource_generation: self.binding.resource_generation,
            configuration_fingerprint: self.descriptor.configuration_schema_fingerprint.clone(),
        }
    }

    fn topology(&self) -> StorageTopology {
        StorageTopology {
            local_state_id: self.binding.local_state_id.clone(),
            disk_set_id: self.binding.disk_set_id.clone(),
            store_view_id: self.binding.store_view_id.clone(),
            closure_sync_id: self.binding.closure_sync_id.clone(),
            media_set_id: self.binding.media_set_id.clone(),
        }
    }

    fn failure(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Storage,
            binding: OperationBinding {
                operation_id: operation.operation_id.clone(),
                idempotency_key: operation.idempotency_key.clone(),
                request_digest: operation.request_digest.clone(),
                provider_id: self.descriptor.provider_id.clone(),
                provider_generation: self.descriptor.registry_generation,
            },
            correlation_id: operation.correlation_id.clone(),
            occurred_at_unix_ms: self.now(),
            reason,
            remediation,
        }
    }

    fn invalid_request(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::CapabilityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn validate_call(
        &self,
        context: &ProviderCallContext<'_>,
        expected: ProviderMethod,
    ) -> ProviderResult<()> {
        if context.cancelled {
            return Err(self.effect_failure(context.operation, StorageEffectError::Cancelled));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.deadline_failure(context.operation, false));
        }
        context
            .validate()
            .map_err(|_| self.invalid_request(context.operation))?;
        context
            .operation
            .validate(&self.descriptor, self.now())
            .map_err(|_| self.invalid_request(context.operation))?;
        if context.operation.method != expected
            || context.operation.scope.realm_id() != &self.binding.realm_id
            || context.operation.scope.workload_id() != Some(&self.binding.workload_id)
        {
            return Err(self.invalid_request(context.operation));
        }
        let ProviderPlacement::TrustedFirstPartyInProcess {
            controller_role, ..
        } = &self.descriptor.placement
        else {
            return Err(self.invalid_request(context.operation));
        };
        if context.peer_role != *controller_role {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        Ok(())
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
    ) -> ProviderResult<()> {
        self.validate_call(context, expected)?;
        if context.operation != &request.context {
            return Err(self.invalid_request(context.operation));
        }
        request
            .validate_method(&self.descriptor, self.now(), expected)
            .map_err(|_| self.invalid_request(context.operation))?;
        if request.target.realm_id() != &self.binding.realm_id
            || request.target.workload_id() != Some(&self.binding.workload_id)
        {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        Ok(())
    }

    fn validate_plan(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
    ) -> ProviderResult<()> {
        self.validate_call(context, ProviderMethod::StorageEnsure)?;
        if plan.schema_version != d2b_contracts::v2_provider::PROVIDER_SCHEMA_VERSION
            || plan.binding.provider_id != self.descriptor.provider_id
            || plan.binding.provider_generation != self.descriptor.registry_generation
            || plan.realm_id != self.binding.realm_id
            || plan.workload_id.as_ref() != Some(&self.binding.workload_id)
            || plan.method != ProviderMethod::StoragePlan
            || plan.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || plan.created_at_unix_ms > self.now()
            || plan.expires_at_unix_ms <= self.now()
            || plan.resources.as_slice() != [PlannedResourceClass::Storage]
        {
            return Err(self.invalid_request(context.operation));
        }
        Ok(())
    }

    fn effect_request(
        &self,
        context: &ProviderCallContext<'_>,
        plan_id: Option<PlanId>,
        handle_id: Option<HandleId>,
    ) -> StorageEffectRequest {
        StorageEffectRequest {
            context: StorageEffectContext {
                operation: context.operation.binding(),
                scope: context.operation.scope.clone(),
                principal: context.operation.principal.clone(),
                authorization_decision_digest: context
                    .operation
                    .authorization_decision_digest
                    .clone(),
                resource: self.resource_binding(),
                deadline_remaining_ms: context.monotonic_deadline_remaining_ms,
            },
            topology: self.topology(),
            plan_id,
            handle_id,
        }
    }

    fn effect_failure(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        error: StorageEffectError,
    ) -> ProviderFailure {
        match error {
            StorageEffectError::Unavailable => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
            StorageEffectError::Rejected => self.failure(
                operation,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            StorageEffectError::Ambiguous => self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
            StorageEffectError::Cancelled => self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
        }
    }

    fn deadline_failure(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        mutation: bool,
    ) -> ProviderFailure {
        if mutation {
            self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::InspectProvider,
            )
        } else {
            self.failure(
                operation,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            )
        }
    }

    async fn run_effect<T>(
        &self,
        context: &ProviderCallContext<'_>,
        mutation: bool,
        future: impl std::future::Future<Output = Result<T, StorageEffectError>>,
    ) -> ProviderResult<T> {
        tokio::time::timeout(
            Duration::from_millis(u64::from(context.monotonic_deadline_remaining_ms)),
            future,
        )
        .await
        .map_err(|_| self.deadline_failure(context.operation, mutation))?
        .map_err(|error| self.effect_failure(context.operation, error))
    }

    fn validate_effect_binding(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        resource: &StorageResourceBinding,
    ) -> ProviderResult<()> {
        if resource == &self.resource_binding() {
            Ok(())
        } else {
            Err(self.failure(
                operation,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ))
        }
    }

    fn values(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> ProviderResult<ProviderValues> {
        ProviderValues::new(&self.descriptor, self.now())
            .map_err(|_| self.invalid_request(operation))
    }

    fn health_fields(
        health: StorageEffectHealth,
    ) -> (
        ProviderHealthState,
        ProviderHealthReason,
        ProviderRemediation,
    ) {
        match health {
            StorageEffectHealth::Healthy => (
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            StorageEffectHealth::Degraded => (
                ProviderHealthState::Degraded,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
            StorageEffectHealth::Unavailable => (
                ProviderHealthState::Unavailable,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            StorageEffectHealth::Failed => (
                ProviderHealthState::Failed,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
        }
    }

    fn adoption_fields(
        state: StorageAdoptionState,
    ) -> (
        ObservedLifecycleState,
        AdoptionState,
        ObservationReason,
        ProviderHealthState,
        ProviderHealthReason,
        ProviderRemediation,
    ) {
        match state {
            StorageAdoptionState::Adopted { lifecycle } => (
                lifecycle,
                AdoptionState::Adopted,
                ObservationReason::None,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            StorageAdoptionState::Rejected(reason) => {
                let (observation, health, remediation) = match reason {
                    StorageAdoptionRejection::IdentityMismatch
                    | StorageAdoptionRejection::OwnerMismatch
                    | StorageAdoptionRejection::MissingEvidence => (
                        ObservationReason::IdentityMismatch,
                        ProviderHealthReason::IdentityMismatch,
                        ProviderRemediation::ReEnrollPeer,
                    ),
                    StorageAdoptionRejection::ConfigurationMismatch => (
                        ObservationReason::ConfigurationMismatch,
                        ProviderHealthReason::ConfigurationMismatch,
                        ProviderRemediation::RepairConfiguration,
                    ),
                    StorageAdoptionRejection::GenerationMismatch => (
                        ObservationReason::GenerationMismatch,
                        ProviderHealthReason::GenerationMismatch,
                        ProviderRemediation::ReplaceGeneration,
                    ),
                };
                (
                    ObservedLifecycleState::Unknown,
                    AdoptionState::Rejected,
                    observation,
                    ProviderHealthState::Failed,
                    health,
                    remediation,
                )
            }
            StorageAdoptionState::Ambiguous => (
                ObservedLifecycleState::Quarantined,
                AdoptionState::Ambiguous,
                ObservationReason::MultipleCandidates,
                ProviderHealthState::Failed,
                ProviderHealthReason::AdoptionAmbiguous,
                ProviderRemediation::OperatorInteraction,
            ),
        }
    }

    fn target_handle_id(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        target: &ProviderTarget,
        required: bool,
    ) -> ProviderResult<Option<HandleId>> {
        match target {
            ProviderTarget::Handle {
                handle_id,
                handle_generation,
                workload_id: Some(workload_id),
                ..
            } if workload_id == &self.binding.workload_id
                && *handle_generation == self.binding.resource_generation =>
            {
                Ok(Some(handle_id.clone()))
            }
            ProviderTarget::Handle { .. } => Err(self.invalid_request(operation)),
            ProviderTarget::Workload { workload_id, .. }
                if !required && workload_id == &self.binding.workload_id =>
            {
                Ok(None)
            }
            ProviderTarget::Realm { .. } | ProviderTarget::Workload { .. } => {
                Err(self.invalid_request(operation))
            }
        }
    }

    async fn known_handle(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        handle_id: Option<&HandleId>,
    ) -> ProviderResult<Option<ProviderHandle>> {
        let Some(handle_id) = handle_id else {
            return Ok(None);
        };
        let state = self.state.lock().await;
        let Some(handle) = state.handles.get(handle_id).cloned() else {
            return Err(self.invalid_request(operation));
        };
        if handle.owner != self.binding.owner()
            || handle.provider_id != self.descriptor.provider_id
            || handle.provider_generation != self.descriptor.registry_generation
            || handle.realm_id != self.binding.realm_id
            || handle.workload_id.as_ref() != Some(&self.binding.workload_id)
            || handle.resource_generation != self.binding.resource_generation
            || handle.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
        {
            return Err(self.invalid_request(operation));
        }
        Ok(Some(handle))
    }

    async fn health_inner(
        &self,
        context: &ProviderCallContext<'_>,
    ) -> ProviderResult<ProviderHealth> {
        self.validate_call(context, context.operation.method)?;
        let request = self.effect_request(context, None, None);
        let health = self
            .run_effect(context, false, self.effects.health(&request))
            .await?;
        let (state, reason, remediation) = Self::health_fields(health);
        self.values(context.operation)?
            .health(state, reason, remediation)
            .map_err(|_| self.invalid_request(context.operation))
    }

    async fn plan_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderPlan> {
        self.validate_request(context, request, ProviderMethod::StoragePlan)?;
        if !matches!(request.target, ProviderTarget::Workload { .. }) {
            return Err(self.invalid_request(context.operation));
        }
        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::StoragePlan,
                &context.operation.request_digest,
            ) {
                return Err(self.invalid_request(context.operation));
            }
            if let CachedResult::Plan { value, .. } = cached {
                return Ok(value.as_ref().clone());
            }
            return Err(self.invalid_request(context.operation));
        }
        if state.operations.len() >= MAX_TRACKED_OPERATIONS {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterObservation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::InspectProvider,
            ));
        }
        let effect_request = self.effect_request(context, None, None);
        let effect_plan = self
            .run_effect(context, false, self.effects.plan(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &effect_plan.resource)?;
        let resources =
            BoundedVec::<PlannedResourceClass, 0, MAX_PROVIDER_PLAN_RESOURCES>::new(vec![
                PlannedResourceClass::Storage,
            ])
            .map_err(|_| self.invalid_request(context.operation))?;
        let expires = self
            .now()
            .saturating_add(PLAN_TTL_MS)
            .min(request.context.expires_at_unix_ms);
        let plan = self
            .values(context.operation)?
            .plan(request, effect_plan.plan_id, expires, resources)
            .map_err(|_| self.invalid_request(context.operation))?;
        state.operations.insert(
            context.operation.idempotency_key.clone(),
            CachedResult::Plan {
                method: ProviderMethod::StoragePlan,
                digest: context.operation.request_digest.clone(),
                value: Box::new(plan.clone()),
            },
        );
        Ok(plan)
    }

    async fn ensure_inner(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
    ) -> ProviderResult<ProviderHandle> {
        self.validate_plan(context, plan)?;
        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::StorageEnsure,
                &context.operation.request_digest,
            ) {
                return Err(self.invalid_request(context.operation));
            }
            if let CachedResult::Handle { value, .. } = cached {
                return Ok(value.as_ref().clone());
            }
            return Err(self.invalid_request(context.operation));
        }
        if state.operations.len() >= MAX_TRACKED_OPERATIONS {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterObservation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::InspectProvider,
            ));
        }
        let effect_request = self.effect_request(context, Some(plan.plan_id.clone()), None);
        let ensured = self
            .run_effect(context, true, self.effects.ensure(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &ensured.resource)?;
        let handle = self
            .values(context.operation)?
            .handle_from_plan(
                plan,
                ensured.handle_id,
                self.binding.owner(),
                self.binding.resource_generation,
                None,
            )
            .map_err(|_| self.invalid_request(context.operation))?;
        state
            .handles
            .insert(handle.handle_id.clone(), handle.clone());
        state.operations.insert(
            context.operation.idempotency_key.clone(),
            CachedResult::Handle {
                method: ProviderMethod::StorageEnsure,
                digest: context.operation.request_digest.clone(),
                value: Box::new(handle.clone()),
            },
        );
        Ok(handle)
    }

    async fn inspect_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderObservation> {
        self.validate_request(context, request, ProviderMethod::StorageInspect)?;
        let handle_id = self.target_handle_id(context.operation, &request.target, false)?;
        let handle = self
            .known_handle(context.operation, handle_id.as_ref())
            .await?;
        let effect_request = self.effect_request(context, None, handle_id);
        let observed = self
            .run_effect(context, false, self.effects.inspect(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &observed.resource)?;
        let (health_state, health_reason, remediation) = Self::health_fields(observed.health);
        self.values(context.operation)?
            .observation(
                context.operation,
                handle.as_ref(),
                observed.lifecycle,
                AdoptionState::NotAttempted,
                ObservationReason::None,
                health_state,
                health_reason,
                remediation,
            )
            .map_err(|_| self.invalid_request(context.operation))
    }

    async fn adopt_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
    ) -> ProviderResult<ProviderObservation> {
        self.validate_call(context, ProviderMethod::StorageAdopt)?;
        if context.operation != &request.context {
            return Err(self.invalid_request(context.operation));
        }
        request
            .validate(&self.descriptor, self.now())
            .map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::AdoptionRejected,
                    RetryClass::AfterObservation,
                    ProviderHealthReason::GenerationMismatch,
                    ProviderRemediation::ReplaceGeneration,
                )
            })?;
        if request.handle.realm_id != self.binding.realm_id
            || request.handle.workload_id.as_ref() != Some(&self.binding.workload_id)
            || request.handle.provider_id != self.descriptor.provider_id
            || request.handle.provider_generation != self.descriptor.registry_generation
            || request.handle.resource_generation != self.binding.resource_generation
            || request.expected_owner != self.binding.owner()
            || request.expected_configuration_fingerprint
                != self.descriptor.configuration_schema_fingerprint
            || request.expected_resource_generation != self.binding.resource_generation
        {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::AdoptionRejected,
                RetryClass::AfterObservation,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ));
        }
        let effect_request =
            self.effect_request(context, None, Some(request.handle.handle_id.clone()));
        let outcome = self
            .run_effect(context, false, self.effects.adopt(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &outcome.resource)?;
        let (lifecycle, adoption, reason, health_state, health_reason, remediation) =
            Self::adoption_fields(outcome.state);
        if adoption == AdoptionState::Adopted {
            self.state
                .lock()
                .await
                .handles
                .insert(request.handle.handle_id.clone(), request.handle.clone());
        }
        self.values(context.operation)?
            .observation(
                context.operation,
                Some(&request.handle),
                lifecycle,
                adoption,
                reason,
                health_state,
                health_reason,
                remediation,
            )
            .map_err(|_| self.invalid_request(context.operation))
    }

    async fn snapshot_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderHandle> {
        self.validate_request(context, request, ProviderMethod::StorageSnapshot)?;
        let ProviderOperationInput::StorageSnapshot { snapshot_id } = &request.input else {
            return Err(self.invalid_request(context.operation));
        };
        let source_handle_id = self
            .target_handle_id(context.operation, &request.target, true)?
            .ok_or_else(|| self.invalid_request(context.operation))?;
        self.known_handle(context.operation, Some(&source_handle_id))
            .await?;

        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::StorageSnapshot,
                &context.operation.request_digest,
            ) {
                return Err(self.invalid_request(context.operation));
            }
            if let CachedResult::Handle { value, .. } = cached {
                return Ok(value.as_ref().clone());
            }
            return Err(self.invalid_request(context.operation));
        }
        if state.operations.len() >= MAX_TRACKED_OPERATIONS {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterObservation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::InspectProvider,
            ));
        }

        let effect = self.effect_request(context, None, Some(source_handle_id));
        let effect_request = StorageSnapshotRequest {
            effect,
            snapshot_id: snapshot_id.clone(),
        };
        let snapshot = self
            .run_effect(context, true, self.effects.snapshot(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &snapshot.resource)?;
        let handle = self
            .values(context.operation)?
            .handle_from_request(
                request,
                snapshot.handle_id,
                self.binding.owner(),
                self.binding.resource_generation,
                None,
            )
            .map_err(|_| self.invalid_request(context.operation))?;
        state
            .handles
            .insert(handle.handle_id.clone(), handle.clone());
        state.operations.insert(
            context.operation.idempotency_key.clone(),
            CachedResult::Handle {
                method: ProviderMethod::StorageSnapshot,
                digest: context.operation.request_digest.clone(),
                value: Box::new(handle.clone()),
            },
        );
        Ok(handle)
    }

    async fn destroy_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<MutationReceipt> {
        self.validate_request(context, request, ProviderMethod::StorageDestroy)?;
        let handle_id = self
            .target_handle_id(context.operation, &request.target, true)?
            .ok_or_else(|| self.invalid_request(context.operation))?;
        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::StorageDestroy,
                &context.operation.request_digest,
            ) {
                return Err(self.invalid_request(context.operation));
            }
            if let CachedResult::Receipt { value, .. } = cached {
                return Ok(value.as_ref().clone());
            }
            return Err(self.invalid_request(context.operation));
        }
        let Some(handle) = state.handles.get(&handle_id) else {
            return Err(self.invalid_request(context.operation));
        };
        if handle.resource_generation != self.binding.resource_generation {
            return Err(self.invalid_request(context.operation));
        }
        if state.operations.len() >= MAX_TRACKED_OPERATIONS {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterObservation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::InspectProvider,
            ));
        }
        let effect_request = self.effect_request(context, None, Some(handle_id));
        let outcome = self
            .run_effect(context, true, self.effects.destroy(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &outcome.resource)?;
        let receipt = self
            .values(context.operation)?
            .receipt(context.operation, outcome.state)
            .map_err(|_| self.invalid_request(context.operation))?;
        state.operations.insert(
            context.operation.idempotency_key.clone(),
            CachedResult::Receipt {
                method: ProviderMethod::StorageDestroy,
                digest: context.operation.request_digest.clone(),
                value: Box::new(receipt.clone()),
            },
        );
        Ok(receipt)
    }
}

impl Provider for LocalStorageProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move { self.health_inner(context).await })
    }
}

impl StorageProvider for LocalStorageProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn plan<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderPlan> {
        Box::pin(async move { self.plan_inner(context, request).await })
    }

    fn ensure<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        plan: &'a ProviderPlan,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { self.ensure_inner(context, plan).await })
    }

    fn inspect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move { self.inspect_inner(context, request).await })
    }

    fn adopt<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a AdoptionRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move { self.adopt_inner(context, request).await })
    }

    fn snapshot<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { self.snapshot_inner(context, request).await })
    }

    fn destroy<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move { self.destroy_inner(context, request).await })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicU64, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use d2b_contracts::{
        v2_component_session::{EndpointRole, ServicePackage},
        v2_provider::{
            PROVIDER_SCHEMA_VERSION, ProviderApiVersion, ProviderAuthority, ProviderCapability,
            ProviderOperationContext, ProviderOperationInput,
        },
    };

    use super::*;

    const NOW: u64 = 1_700_000_000_000;

    #[derive(Debug)]
    struct TestClock;

    impl ProviderClock for TestClock {
        fn now_unix_ms(&self) -> u64 {
            NOW
        }
    }

    #[derive(Default)]
    struct FakeEffects {
        calls: AtomicUsize,
        plan_calls: AtomicUsize,
        ensure_calls: AtomicUsize,
        adopt_calls: AtomicUsize,
        snapshot_calls: AtomicUsize,
        destroy_calls: AtomicUsize,
        delay_ms: AtomicU64,
        last: StdMutex<Option<StorageEffectRequest>>,
        last_snapshot: StdMutex<Option<StorageSnapshotId>>,
    }

    impl FakeEffects {
        fn record(&self, request: &StorageEffectRequest) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().expect("last request lock") = Some(request.clone());
        }

        async fn delay(&self) {
            let delay = self.delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }

    #[async_trait]
    impl StorageEffectPort for FakeEffects {
        fn live_capabilities(&self) -> StorageLiveCapabilities {
            StorageLiveCapabilities::REQUIRED
        }

        async fn health(
            &self,
            request: &StorageEffectRequest,
        ) -> Result<StorageEffectHealth, StorageEffectError> {
            self.record(request);
            Ok(StorageEffectHealth::Healthy)
        }

        async fn plan(
            &self,
            request: &StorageEffectRequest,
        ) -> Result<StorageEffectPlan, StorageEffectError> {
            self.record(request);
            self.plan_calls.fetch_add(1, Ordering::SeqCst);
            Ok(StorageEffectPlan {
                plan_id: PlanId::parse("storage-plan").expect("plan"),
                resource: request.context.resource.clone(),
            })
        }

        async fn ensure(
            &self,
            request: &StorageEffectRequest,
        ) -> Result<StorageEffectHandle, StorageEffectError> {
            self.record(request);
            self.ensure_calls.fetch_add(1, Ordering::SeqCst);
            self.delay().await;
            Ok(StorageEffectHandle {
                handle_id: HandleId::parse("storage-handle").expect("handle"),
                resource: request.context.resource.clone(),
            })
        }

        async fn inspect(
            &self,
            request: &StorageEffectRequest,
        ) -> Result<StorageEffectObservation, StorageEffectError> {
            self.record(request);
            Ok(StorageEffectObservation {
                resource: request.context.resource.clone(),
                lifecycle: ObservedLifecycleState::Ready,
                health: StorageEffectHealth::Healthy,
            })
        }

        async fn adopt(
            &self,
            request: &StorageEffectRequest,
        ) -> Result<StorageAdoptionOutcome, StorageEffectError> {
            self.record(request);
            self.adopt_calls.fetch_add(1, Ordering::SeqCst);
            Ok(StorageAdoptionOutcome {
                resource: request.context.resource.clone(),
                state: StorageAdoptionState::Adopted {
                    lifecycle: ObservedLifecycleState::Ready,
                },
            })
        }

        async fn snapshot(
            &self,
            request: &StorageSnapshotRequest,
        ) -> Result<StorageEffectHandle, StorageEffectError> {
            self.record(&request.effect);
            self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
            *self.last_snapshot.lock().expect("snapshot lock") = Some(request.snapshot_id.clone());
            self.delay().await;
            Ok(StorageEffectHandle {
                handle_id: HandleId::parse("storage-snapshot-handle").expect("handle"),
                resource: request.effect.context.resource.clone(),
            })
        }

        async fn destroy(
            &self,
            request: &StorageEffectRequest,
        ) -> Result<StorageMutationOutcome, StorageEffectError> {
            self.record(request);
            self.destroy_calls.fetch_add(1, Ordering::SeqCst);
            Ok(StorageMutationOutcome {
                resource: request.context.resource.clone(),
                state: MutationState::Applied,
            })
        }
    }

    fn short_id(letter: char) -> String {
        format!("{}a", letter.to_string().repeat(19))
    }

    fn fingerprint(value: u8) -> Fingerprint {
        Fingerprint::parse(format!("{value:064x}")).expect("fingerprint")
    }

    fn descriptor() -> ProviderDescriptor {
        ProviderDescriptor {
            schema_version: PROVIDER_SCHEMA_VERSION,
            provider_id: ProviderId::parse(short_id('b')).expect("provider"),
            authority: ProviderAuthority::Storage,
            implementation_id: implementation_id(),
            api_version: ProviderApiVersion::V2,
            capabilities: ProviderCapabilitySet::new(vec![
                ProviderCapability(ProviderMethod::StoragePlan),
                ProviderCapability(ProviderMethod::StorageEnsure),
                ProviderCapability(ProviderMethod::StorageInspect),
                ProviderCapability(ProviderMethod::StorageAdopt),
                ProviderCapability(ProviderMethod::StorageSnapshot),
                ProviderCapability(ProviderMethod::StorageDestroy),
            ])
            .expect("capabilities"),
            configuration_schema_fingerprint: fingerprint(1),
            configured_scope_digest: fingerprint(2),
            registry_generation: Generation::new(1).expect("generation"),
            placement: ProviderPlacement::TrustedFirstPartyInProcess {
                realm_id: RealmId::parse(short_id('a')).expect("realm"),
                controller_role: EndpointRole::RealmController,
            },
        }
    }

    fn binding() -> LocalStorageBinding {
        LocalStorageBinding {
            realm_id: RealmId::parse(short_id('a')).expect("realm"),
            workload_id: WorkloadId::parse(short_id('c')).expect("workload"),
            local_state_id: GeneratedStorageId::parse("local-state").expect("storage"),
            disk_set_id: GeneratedStorageId::parse("disk-set").expect("storage"),
            store_view_id: GeneratedStorageId::parse("store-view").expect("storage"),
            closure_sync_id: GeneratedStorageId::parse("closure-sync").expect("storage"),
            media_set_id: GeneratedStorageId::parse("media-set").expect("storage"),
            resource_generation: Generation::new(1).expect("generation"),
        }
    }

    fn provider(effects: Arc<FakeEffects>) -> LocalStorageProvider {
        LocalStorageProvider::with_clock(descriptor(), binding(), effects, Arc::new(TestClock))
            .expect("provider")
    }

    fn operation(method: ProviderMethod, id: &str) -> ProviderOperationContext {
        let descriptor = descriptor();
        ProviderOperationContext {
            schema_version: PROVIDER_SCHEMA_VERSION,
            operation_id: d2b_contracts::v2_provider::OperationId::parse(id).expect("operation"),
            idempotency_key: IdempotencyKey::parse(format!("{id}-idempotency"))
                .expect("idempotency"),
            request_digest: fingerprint(match method {
                ProviderMethod::StoragePlan => 3,
                ProviderMethod::StorageEnsure => 4,
                ProviderMethod::StorageInspect => 5,
                ProviderMethod::StorageAdopt => 6,
                ProviderMethod::StorageSnapshot => 7,
                ProviderMethod::StorageDestroy => 8,
                _ => 11,
            }),
            scope: AuthorizedProviderScope::Workload {
                realm_id: binding().realm_id,
                workload_id: binding().workload_id,
            },
            principal: PrincipalRef::parse("storage-principal").expect("principal"),
            provider_id: descriptor.provider_id,
            provider_type: ProviderType::Storage,
            provider_generation: Generation::new(1).expect("generation"),
            capability: ProviderCapability(method),
            method,
            policy_epoch: Generation::new(1).expect("generation"),
            authorization_decision_digest: fingerprint(9),
            issued_at_unix_ms: NOW - 1_000,
            expires_at_unix_ms: NOW + 60_000,
            correlation_id: d2b_contracts::v2_provider::CorrelationId::parse("storage-correlation")
                .expect("correlation"),
            trace_id: fingerprint(10),
        }
    }

    fn request(method: ProviderMethod, input: ProviderOperationInput) -> ProviderOperationRequest {
        ProviderOperationRequest {
            context: operation(method, "storage-operation"),
            target: ProviderTarget::Workload {
                realm_id: binding().realm_id,
                workload_id: binding().workload_id,
            },
            expected_configuration_fingerprint: fingerprint(1),
            input,
        }
    }

    fn call_context<'a>(
        operation: &'a ProviderOperationContext,
        deadline_ms: u32,
        cancelled: bool,
    ) -> ProviderCallContext<'a> {
        ProviderCallContext {
            operation,
            peer_role: EndpointRole::RealmController,
            service: ServicePackage::ProviderV2,
            monotonic_deadline_remaining_ms: deadline_ms,
            cancelled,
        }
    }

    fn handle_request(
        method: ProviderMethod,
        input: ProviderOperationInput,
        handle: &ProviderHandle,
        operation_id: &str,
    ) -> ProviderOperationRequest {
        ProviderOperationRequest {
            context: operation(method, operation_id),
            target: ProviderTarget::Handle {
                realm_id: binding().realm_id,
                workload_id: Some(binding().workload_id),
                handle_id: handle.handle_id.clone(),
                handle_generation: handle.resource_generation,
            },
            expected_configuration_fingerprint: fingerprint(1),
            input,
        }
    }

    async fn planned_and_ensured(
        provider: &LocalStorageProvider,
    ) -> (ProviderPlan, ProviderHandle) {
        let request = request(ProviderMethod::StoragePlan, ProviderOperationInput::NoInput);
        let plan_context = call_context(&request.context, 30_000, false);
        let plan = provider.plan(&plan_context, &request).await.expect("plan");
        let ensure_operation = operation(ProviderMethod::StorageEnsure, "storage-ensure");
        let ensure_context = call_context(&ensure_operation, 30_000, false);
        let handle = provider
            .ensure(&ensure_context, &plan)
            .await
            .expect("ensure");
        (plan, handle)
    }

    #[test]
    fn advertises_canonical_live_capabilities() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects);
        assert_eq!(provider.capabilities(), descriptor().capabilities);
        assert!(StorageLiveCapabilities::REQUIRED.is_complete());
    }

    #[test]
    fn factory_registers_and_rejects_wrong_descriptor_axis() {
        let effects = Arc::new(FakeEffects::default());
        let factory = Arc::new(LocalStorageFactory::with_clock(
            binding(),
            effects,
            Arc::new(TestClock),
        ));
        let descriptor = descriptor();
        let key = provider_factory_key();
        assert_eq!(key.provider_type, ProviderType::Storage);
        assert_eq!(key.implementation_id, implementation_id());

        let mut wrong_type = descriptor.clone();
        wrong_type.authority = ProviderAuthority::Display;
        assert!(matches!(
            factory.construct(&wrong_type),
            Err(FactoryError::Rejected)
        ));

        let mut wrong_implementation = descriptor.clone();
        wrong_implementation.implementation_id =
            ImplementationId::parse("other-storage").expect("implementation");
        assert!(matches!(
            factory.construct(&wrong_implementation),
            Err(FactoryError::Rejected)
        ));

        let mut builder = d2b_provider::ProviderRegistryBuilder::new(
            descriptor.registry_generation,
            fingerprint(12),
            NOW,
        );
        builder
            .register_factory(key, factory)
            .expect("register factory")
            .register_instance(descriptor.clone())
            .expect("register provider");
        let registry = builder.finish().expect("registry");
        assert_eq!(
            registry
                .instance(&descriptor.provider_id)
                .expect("instance")
                .descriptor(),
            descriptor
        );
    }

    #[tokio::test]
    async fn plan_and_ensure_preserve_nix_authority_and_idempotency() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let request = request(ProviderMethod::StoragePlan, ProviderOperationInput::NoInput);
        let context = call_context(&request.context, 30_000, false);
        let first = provider.plan(&context, &request).await.expect("plan");
        let second = provider
            .plan(&context, &request)
            .await
            .expect("repeat plan");
        assert_eq!(first, second);
        assert_eq!(effects.plan_calls.load(Ordering::SeqCst), 1);

        let ensure_operation = operation(ProviderMethod::StorageEnsure, "storage-ensure");
        let ensure_context = call_context(&ensure_operation, 30_000, false);
        let first_handle = provider
            .ensure(&ensure_context, &first)
            .await
            .expect("ensure");
        let second_handle = provider
            .ensure(&ensure_context, &first)
            .await
            .expect("repeat ensure");
        assert_eq!(first_handle, second_handle);
        assert_eq!(effects.ensure_calls.load(Ordering::SeqCst), 1);

        let captured = effects
            .last
            .lock()
            .expect("last request lock")
            .clone()
            .expect("request");
        assert_eq!(captured.topology.local_state_id, binding().local_state_id);
        assert_eq!(captured.topology.disk_set_id, binding().disk_set_id);
        assert_eq!(captured.topology.store_view_id, binding().store_view_id);
        assert_eq!(captured.topology.closure_sync_id, binding().closure_sync_id);
        assert_eq!(captured.topology.media_set_id, binding().media_set_id);
    }

    #[tokio::test]
    async fn wrong_input_and_cancellation_have_zero_effect() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let wrong = request(
            ProviderMethod::StoragePlan,
            ProviderOperationInput::StorageSnapshot {
                snapshot_id: d2b_contracts::v2_provider::StorageSnapshotId::parse("snapshot")
                    .expect("snapshot"),
            },
        );
        let wrong_context = call_context(&wrong.context, 30_000, false);
        assert!(provider.plan(&wrong_context, &wrong).await.is_err());

        let cancelled = request(
            ProviderMethod::StorageInspect,
            ProviderOperationInput::NoInput,
        );
        let cancelled_context = call_context(&cancelled.context, 30_000, true);
        let cancelled_error = provider
            .inspect(&cancelled_context, &cancelled)
            .await
            .expect_err("cancelled");
        assert_eq!(cancelled_error.kind, ProviderFailureKind::Cancelled);
        assert_eq!(effects.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_adoption_is_rejected_without_effect() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let (_, handle) = planned_and_ensured(&provider).await;
        let adoption = AdoptionRequest {
            context: operation(ProviderMethod::StorageAdopt, "storage-adopt"),
            handle: handle.clone(),
            expected_owner: handle.owner.clone(),
            expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
            expected_resource_generation: Generation::new(2).expect("stale"),
        };
        let context = call_context(&adoption.context, 30_000, false);
        let before = effects.calls.load(Ordering::SeqCst);
        let error = provider
            .adopt(&context, &adoption)
            .await
            .expect_err("stale adoption");
        assert_eq!(error.kind, ProviderFailureKind::AdoptionRejected);
        assert_eq!(effects.calls.load(Ordering::SeqCst), before);
        assert_eq!(effects.adopt_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn snapshot_forwards_only_typed_input_and_is_idempotent() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let (_, handle) = planned_and_ensured(&provider).await;
        let snapshot_id = StorageSnapshotId::parse("snapshot-one").expect("snapshot");
        let request = handle_request(
            ProviderMethod::StorageSnapshot,
            ProviderOperationInput::StorageSnapshot {
                snapshot_id: snapshot_id.clone(),
            },
            &handle,
            "storage-snapshot",
        );
        let context = call_context(&request.context, 30_000, false);
        let first = provider
            .snapshot(&context, &request)
            .await
            .expect("snapshot");
        let second = provider
            .snapshot(&context, &request)
            .await
            .expect("repeat snapshot");
        assert_eq!(first, second);
        assert_eq!(effects.snapshot_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            effects
                .last_snapshot
                .lock()
                .expect("snapshot lock")
                .as_ref(),
            Some(&snapshot_id)
        );

        let wrong = handle_request(
            ProviderMethod::StorageSnapshot,
            ProviderOperationInput::NoInput,
            &handle,
            "storage-snapshot-wrong",
        );
        let wrong_context = call_context(&wrong.context, 30_000, false);
        assert!(provider.snapshot(&wrong_context, &wrong).await.is_err());
        assert_eq!(effects.snapshot_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn inspect_adopt_and_destroy_use_the_bound_handle() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let (_, handle) = planned_and_ensured(&provider).await;

        let inspect = handle_request(
            ProviderMethod::StorageInspect,
            ProviderOperationInput::NoInput,
            &handle,
            "storage-inspect",
        );
        let inspect_context = call_context(&inspect.context, 30_000, false);
        let observation = provider
            .inspect(&inspect_context, &inspect)
            .await
            .expect("inspect");
        assert_eq!(observation.handle_id.as_ref(), Some(&handle.handle_id));

        let adoption = AdoptionRequest {
            context: operation(ProviderMethod::StorageAdopt, "storage-adopt-valid"),
            handle: handle.clone(),
            expected_owner: handle.owner.clone(),
            expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
            expected_resource_generation: handle.resource_generation,
        };
        let adopt_context = call_context(&adoption.context, 30_000, false);
        let adopted = provider
            .adopt(&adopt_context, &adoption)
            .await
            .expect("adopt");
        assert_eq!(adopted.adoption, AdoptionState::Adopted);

        let destroy = handle_request(
            ProviderMethod::StorageDestroy,
            ProviderOperationInput::NoInput,
            &handle,
            "storage-destroy",
        );
        let destroy_context = call_context(&destroy.context, 30_000, false);
        let first = provider
            .destroy(&destroy_context, &destroy)
            .await
            .expect("destroy");
        let second = provider
            .destroy(&destroy_context, &destroy)
            .await
            .expect("repeat destroy");
        assert_eq!(first, second);
        assert_eq!(effects.adopt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(effects.destroy_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ensure_deadline_is_ambiguous_and_bounded() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let request = request(ProviderMethod::StoragePlan, ProviderOperationInput::NoInput);
        let plan_context = call_context(&request.context, 30_000, false);
        let plan = provider.plan(&plan_context, &request).await.expect("plan");
        effects.delay_ms.store(25, Ordering::SeqCst);
        let ensure_operation = operation(ProviderMethod::StorageEnsure, "storage-ensure");
        let ensure_context = call_context(&ensure_operation, 1, false);
        let error = provider
            .ensure(&ensure_context, &plan)
            .await
            .expect_err("deadline");
        assert_eq!(error.kind, ProviderFailureKind::AmbiguousMutation);
        assert_eq!(effects.ensure_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn boundary_cannot_encode_paths_modes_acls_or_repair_instructions() {
        for forbidden in [
            "/var/lib/d2b/vms/work",
            "../work/disk.raw",
            "u:alice:rwx",
            "chmod 0770",
            "0750",
        ] {
            assert!(GeneratedStorageId::parse(forbidden).is_err(), "{forbidden}");
        }
        let rendered = format!(
            "{:?} {:?} {:?} {}",
            binding(),
            StorageSnapshotRequest {
                effect: StorageEffectRequest {
                    context: StorageEffectContext {
                        operation: operation(ProviderMethod::StorageSnapshot, "render").binding(),
                        scope: AuthorizedProviderScope::Workload {
                            realm_id: binding().realm_id,
                            workload_id: binding().workload_id,
                        },
                        principal: PrincipalRef::parse("storage-principal").expect("principal"),
                        authorization_decision_digest: fingerprint(9),
                        resource: provider(Arc::new(FakeEffects::default())).resource_binding(),
                        deadline_remaining_ms: 1,
                    },
                    topology: provider(Arc::new(FakeEffects::default())).topology(),
                    plan_id: None,
                    handle_id: None,
                },
                snapshot_id: StorageSnapshotId::parse("snapshot-secret").expect("snapshot"),
            },
            StorageEffectError::Rejected,
            StorageBuildError::ScopeMismatch
        );
        for forbidden in [
            "/var/lib",
            "disk.raw",
            "u:alice",
            "chmod",
            "0750",
            "snapshot-secret",
        ] {
            assert!(!rendered.contains(forbidden), "{rendered}");
        }
    }
}
