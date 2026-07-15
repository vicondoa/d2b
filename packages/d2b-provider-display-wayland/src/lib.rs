//! Wayland display provider implementation boundary.
//!
//! The provider never accepts compositor socket paths or process arguments.
//! Generated role and endpoint identifiers are injected at construction and
//! forwarded to an async semantic effect port implemented by the owning realm
//! controller.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{collections::BTreeMap, error::Error, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType, RealmId, RoleId, WorkloadId},
    v2_provider::{
        AdoptionRequest, AdoptionState, AuthorizedProviderScope, Fingerprint, Generation, HandleId,
        HandleOwner, IdempotencyKey, ImplementationId, MutationReceipt, MutationState,
        ObservationReason, ObservedLifecycleState, OperationBinding, PlanId, PrincipalRef,
        Provider, ProviderCallContext, ProviderCapabilitySet, ProviderDescriptor,
        ProviderFactoryKey, ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle,
        ProviderHealth, ProviderHealthReason, ProviderHealthState, ProviderMethod,
        ProviderObservation, ProviderOperationRequest, ProviderPlacement, ProviderRemediation,
        ProviderResult, ProviderTarget, RetryClass,
    },
};
use d2b_provider::{
    DisplayProvider, FactoryError, ProviderClock, ProviderFactory, ProviderInstance,
    SystemProviderClock,
};
use d2b_provider_toolkit::ProviderValues;
use tokio::sync::Mutex;

const MAX_TRACKED_OPERATIONS: usize = 128;
pub const IMPLEMENTATION_ID: &str = "wayland";

pub fn implementation_id() -> ImplementationId {
    ImplementationId::parse(IMPLEMENTATION_ID)
        .unwrap_or_else(|_| unreachable!("wayland implementation ID is valid"))
}

pub fn provider_factory_key() -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Display,
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
        formatter.write_str("invalid opaque display identifier")
    }
}

impl Error for OpaqueIdError {}

opaque_id!(
    DisplayEndpointId,
    "An opaque generated display endpoint identifier."
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayLiveCapabilities {
    pub wayland: bool,
    pub cross_domain: bool,
    pub waypipe: bool,
    pub proxy: bool,
    pub authorization: bool,
}

impl DisplayLiveCapabilities {
    pub const REQUIRED: Self = Self {
        wayland: true,
        cross_domain: true,
        waypipe: true,
        proxy: true,
        authorization: true,
    };

    const fn is_complete(self) -> bool {
        self.wayland && self.cross_domain && self.waypipe && self.proxy && self.authorization
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WaylandDisplayBinding {
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub owner_role_id: RoleId,
    pub wayland_endpoint_id: DisplayEndpointId,
    pub cross_domain_endpoint_id: DisplayEndpointId,
    pub waypipe_endpoint_id: DisplayEndpointId,
    pub proxy_endpoint_id: DisplayEndpointId,
    pub resource_generation: Generation,
}

impl fmt::Debug for WaylandDisplayBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WaylandDisplayBinding")
            .field("resource_generation", &self.resource_generation)
            .finish_non_exhaustive()
    }
}

impl WaylandDisplayBinding {
    fn owner(&self) -> HandleOwner {
        HandleOwner::WorkloadRole {
            realm_id: self.realm_id.clone(),
            workload_id: self.workload_id.clone(),
            role_id: self.owner_role_id.clone(),
        }
    }

    fn endpoints_are_distinct(&self) -> bool {
        let endpoints = [
            &self.wayland_endpoint_id,
            &self.cross_domain_endpoint_id,
            &self.waypipe_endpoint_id,
            &self.proxy_endpoint_id,
        ];
        endpoints
            .iter()
            .enumerate()
            .all(|(index, endpoint)| endpoints[index + 1..].iter().all(|other| endpoint != other))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DisplayResourceBinding {
    pub provider_id: ProviderId,
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub owner: HandleOwner,
    pub provider_generation: Generation,
    pub resource_generation: Generation,
    pub configuration_fingerprint: Fingerprint,
}

impl fmt::Debug for DisplayResourceBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayResourceBinding")
            .field("provider_generation", &self.provider_generation)
            .field("resource_generation", &self.resource_generation)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DisplayTopology {
    pub owner_role_id: RoleId,
    pub wayland_endpoint_id: DisplayEndpointId,
    pub cross_domain_endpoint_id: DisplayEndpointId,
    pub waypipe_endpoint_id: DisplayEndpointId,
    pub proxy_endpoint_id: DisplayEndpointId,
}

impl fmt::Debug for DisplayTopology {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayTopology")
            .field("endpoint_count", &4)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct DisplayEffectContext {
    pub operation: OperationBinding,
    pub scope: AuthorizedProviderScope,
    pub principal: PrincipalRef,
    pub authorization_decision_digest: Fingerprint,
    pub resource: DisplayResourceBinding,
    pub deadline_remaining_ms: u32,
}

impl fmt::Debug for DisplayEffectContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayEffectContext")
            .field("provider_generation", &self.operation.provider_generation)
            .field("resource", &self.resource)
            .field("deadline_remaining_ms", &self.deadline_remaining_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct DisplayEffectRequest {
    pub context: DisplayEffectContext,
    pub topology: DisplayTopology,
    pub handle_id: Option<HandleId>,
}

impl fmt::Debug for DisplayEffectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayEffectRequest")
            .field("context", &self.context)
            .field("topology", &self.topology)
            .field("has_handle", &self.handle_id.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DisplayEffectPlan {
    pub plan_id: PlanId,
    pub resource: DisplayResourceBinding,
}

impl fmt::Debug for DisplayEffectPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayEffectPlan")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DisplayEffectHandle {
    pub handle_id: HandleId,
    pub resource: DisplayResourceBinding,
}

impl fmt::Debug for DisplayEffectHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayEffectHandle")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayEffectHealth {
    Healthy,
    Degraded,
    Unavailable,
    Failed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DisplayEffectObservation {
    pub resource: DisplayResourceBinding,
    pub lifecycle: ObservedLifecycleState,
    pub health: DisplayEffectHealth,
}

impl fmt::Debug for DisplayEffectObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayEffectObservation")
            .field("resource", &self.resource)
            .field("lifecycle", &self.lifecycle)
            .field("health", &self.health)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayAdoptionRejection {
    IdentityMismatch,
    ConfigurationMismatch,
    GenerationMismatch,
    OwnerMismatch,
    MissingEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayAdoptionState {
    Adopted { lifecycle: ObservedLifecycleState },
    Rejected(DisplayAdoptionRejection),
    Ambiguous,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DisplayAdoptionOutcome {
    pub resource: DisplayResourceBinding,
    pub state: DisplayAdoptionState,
}

impl fmt::Debug for DisplayAdoptionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayAdoptionOutcome")
            .field("resource", &self.resource)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DisplayMutationOutcome {
    pub resource: DisplayResourceBinding,
    pub state: MutationState,
}

impl fmt::Debug for DisplayMutationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayMutationOutcome")
            .field("resource", &self.resource)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayEffectError {
    Unavailable,
    Rejected,
    Ambiguous,
    Cancelled,
}

impl fmt::Display for DisplayEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "display effect unavailable",
            Self::Rejected => "display effect rejected",
            Self::Ambiguous => "display effect completion ambiguous",
            Self::Cancelled => "display effect cancelled",
        })
    }
}

impl Error for DisplayEffectError {}

#[async_trait]
pub trait DisplayEffectPort: Send + Sync {
    fn live_capabilities(&self) -> DisplayLiveCapabilities;

    async fn health(
        &self,
        request: &DisplayEffectRequest,
    ) -> Result<DisplayEffectHealth, DisplayEffectError>;

    async fn plan(
        &self,
        request: &DisplayEffectRequest,
    ) -> Result<DisplayEffectPlan, DisplayEffectError>;

    async fn ensure(
        &self,
        request: &DisplayEffectRequest,
        plan: &DisplayEffectPlan,
    ) -> Result<DisplayEffectHandle, DisplayEffectError>;

    async fn inspect(
        &self,
        request: &DisplayEffectRequest,
    ) -> Result<DisplayEffectObservation, DisplayEffectError>;

    async fn adopt(
        &self,
        request: &DisplayEffectRequest,
    ) -> Result<DisplayAdoptionOutcome, DisplayEffectError>;

    async fn destroy(
        &self,
        request: &DisplayEffectRequest,
    ) -> Result<DisplayMutationOutcome, DisplayEffectError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBuildError {
    InvalidDescriptor,
    WrongProviderType,
    WrongImplementation,
    WrongPlacement,
    ScopeMismatch,
    DuplicateEndpoint,
    MissingLiveCapability,
}

impl fmt::Display for DisplayBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor => "invalid display provider descriptor",
            Self::WrongProviderType => "descriptor is not a display provider",
            Self::WrongImplementation => "display implementation is not wayland",
            Self::WrongPlacement => "wayland display provider must run in the owning controller",
            Self::ScopeMismatch => "display binding is outside the configured provider scope",
            Self::DuplicateEndpoint => "display endpoint identifiers must be distinct",
            Self::MissingLiveCapability => "required live wayland capability is unavailable",
        })
    }
}

impl Error for DisplayBuildError {}

#[derive(Clone)]
pub struct WaylandDisplayFactory {
    binding: WaylandDisplayBinding,
    effects: Arc<dyn DisplayEffectPort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for WaylandDisplayFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WaylandDisplayFactory")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl WaylandDisplayFactory {
    pub fn new(binding: WaylandDisplayBinding, effects: Arc<dyn DisplayEffectPort>) -> Self {
        Self::with_clock(binding, effects, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        binding: WaylandDisplayBinding,
        effects: Arc<dyn DisplayEffectPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Self {
        Self {
            binding,
            effects,
            clock,
        }
    }
}

impl ProviderFactory for WaylandDisplayFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Display
            || descriptor.implementation_id != implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let provider = WaylandDisplayProvider::with_clock(
            descriptor.clone(),
            self.binding.clone(),
            self.effects.clone(),
            self.clock.clone(),
        )
        .map_err(|error| match error {
            DisplayBuildError::MissingLiveCapability => FactoryError::Unavailable,
            _ => FactoryError::Rejected,
        })?;
        Ok(ProviderInstance::Display(Arc::new(provider)))
    }
}

#[derive(Clone)]
enum CachedResult {
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
            Self::Handle {
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

pub struct WaylandDisplayProvider {
    descriptor: ProviderDescriptor,
    binding: WaylandDisplayBinding,
    effects: Arc<dyn DisplayEffectPort>,
    clock: Arc<dyn ProviderClock>,
    state: Mutex<ProviderState>,
}

impl fmt::Debug for WaylandDisplayProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WaylandDisplayProvider")
            .field("descriptor", &self.descriptor)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl WaylandDisplayProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        binding: WaylandDisplayBinding,
        effects: Arc<dyn DisplayEffectPort>,
    ) -> Result<Self, DisplayBuildError> {
        Self::with_clock(descriptor, binding, effects, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        binding: WaylandDisplayBinding,
        effects: Arc<dyn DisplayEffectPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, DisplayBuildError> {
        descriptor
            .validate()
            .map_err(|_| DisplayBuildError::InvalidDescriptor)?;
        if descriptor.provider_type() != ProviderType::Display {
            return Err(DisplayBuildError::WrongProviderType);
        }
        if descriptor.implementation_id != implementation_id() {
            return Err(DisplayBuildError::WrongImplementation);
        }
        if !matches!(
            descriptor.placement,
            ProviderPlacement::TrustedFirstPartyInProcess { .. }
        ) {
            return Err(DisplayBuildError::WrongPlacement);
        }
        if descriptor.placement.realm_id() != &binding.realm_id {
            return Err(DisplayBuildError::ScopeMismatch);
        }
        if !binding.endpoints_are_distinct() {
            return Err(DisplayBuildError::DuplicateEndpoint);
        }
        if !effects.live_capabilities().is_complete() {
            return Err(DisplayBuildError::MissingLiveCapability);
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

    fn resource_binding(&self) -> DisplayResourceBinding {
        DisplayResourceBinding {
            provider_id: self.descriptor.provider_id.clone(),
            realm_id: self.binding.realm_id.clone(),
            workload_id: self.binding.workload_id.clone(),
            owner: self.binding.owner(),
            provider_generation: self.descriptor.registry_generation,
            resource_generation: self.binding.resource_generation,
            configuration_fingerprint: self.descriptor.configuration_schema_fingerprint.clone(),
        }
    }

    fn topology(&self) -> DisplayTopology {
        DisplayTopology {
            owner_role_id: self.binding.owner_role_id.clone(),
            wayland_endpoint_id: self.binding.wayland_endpoint_id.clone(),
            cross_domain_endpoint_id: self.binding.cross_domain_endpoint_id.clone(),
            waypipe_endpoint_id: self.binding.waypipe_endpoint_id.clone(),
            proxy_endpoint_id: self.binding.proxy_endpoint_id.clone(),
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
            provider_type: ProviderType::Display,
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
            return Err(self.effect_failure(context.operation, DisplayEffectError::Cancelled));
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

    fn effect_request(
        &self,
        context: &ProviderCallContext<'_>,
        handle_id: Option<HandleId>,
    ) -> DisplayEffectRequest {
        DisplayEffectRequest {
            context: DisplayEffectContext {
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
            handle_id,
        }
    }

    fn effect_failure(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        error: DisplayEffectError,
    ) -> ProviderFailure {
        match error {
            DisplayEffectError::Unavailable => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
            DisplayEffectError::Rejected => self.failure(
                operation,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            DisplayEffectError::Ambiguous => self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
            DisplayEffectError::Cancelled => self.failure(
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
        future: impl std::future::Future<Output = Result<T, DisplayEffectError>>,
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
        resource: &DisplayResourceBinding,
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
                ..
            } if *handle_generation == self.binding.resource_generation => {
                Ok(Some(handle_id.clone()))
            }
            ProviderTarget::Handle { .. } => Err(self.invalid_request(operation)),
            ProviderTarget::Workload { .. } if required => Err(self.invalid_request(operation)),
            ProviderTarget::Workload { .. } => Ok(None),
            ProviderTarget::Realm { .. } => Err(self.invalid_request(operation)),
        }
    }

    fn health_fields(
        health: DisplayEffectHealth,
    ) -> (
        ProviderHealthState,
        ProviderHealthReason,
        ProviderRemediation,
    ) {
        match health {
            DisplayEffectHealth::Healthy => (
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            DisplayEffectHealth::Degraded => (
                ProviderHealthState::Degraded,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
            DisplayEffectHealth::Unavailable => (
                ProviderHealthState::Unavailable,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            DisplayEffectHealth::Failed => (
                ProviderHealthState::Failed,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
        }
    }

    fn adoption_fields(
        state: DisplayAdoptionState,
    ) -> (
        ObservedLifecycleState,
        AdoptionState,
        ObservationReason,
        ProviderHealthState,
        ProviderHealthReason,
        ProviderRemediation,
    ) {
        match state {
            DisplayAdoptionState::Adopted { lifecycle } => (
                lifecycle,
                AdoptionState::Adopted,
                ObservationReason::None,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            DisplayAdoptionState::Rejected(reason) => {
                let (observation, health, remediation) = match reason {
                    DisplayAdoptionRejection::IdentityMismatch
                    | DisplayAdoptionRejection::OwnerMismatch
                    | DisplayAdoptionRejection::MissingEvidence => (
                        ObservationReason::IdentityMismatch,
                        ProviderHealthReason::IdentityMismatch,
                        ProviderRemediation::ReEnrollPeer,
                    ),
                    DisplayAdoptionRejection::ConfigurationMismatch => (
                        ObservationReason::ConfigurationMismatch,
                        ProviderHealthReason::ConfigurationMismatch,
                        ProviderRemediation::RepairConfiguration,
                    ),
                    DisplayAdoptionRejection::GenerationMismatch => (
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
            DisplayAdoptionState::Ambiguous => (
                ObservedLifecycleState::Quarantined,
                AdoptionState::Ambiguous,
                ObservationReason::MultipleCandidates,
                ProviderHealthState::Failed,
                ProviderHealthReason::AdoptionAmbiguous,
                ProviderRemediation::OperatorInteraction,
            ),
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
        let request = self.effect_request(context, None);
        let health = self
            .run_effect(context, false, self.effects.health(&request))
            .await?;
        let (state, reason, remediation) = Self::health_fields(health);
        self.values(context.operation)?
            .health(state, reason, remediation)
            .map_err(|_| self.invalid_request(context.operation))
    }

    async fn open_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderHandle> {
        self.validate_request(context, request, ProviderMethod::DisplayOpen)?;
        if !matches!(request.target, ProviderTarget::Workload { .. }) {
            return Err(self.invalid_request(context.operation));
        }

        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::DisplayOpen,
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

        let effect_request = self.effect_request(context, None);
        let plan = self
            .run_effect(context, false, self.effects.plan(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &plan.resource)?;
        let ensured = self
            .run_effect(context, true, self.effects.ensure(&effect_request, &plan))
            .await?;
        self.validate_effect_binding(context.operation, &ensured.resource)?;
        let handle = self
            .values(context.operation)?
            .handle_from_request(
                request,
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
                method: ProviderMethod::DisplayOpen,
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
        self.validate_request(context, request, ProviderMethod::DisplayInspect)?;
        let handle_id = self.target_handle_id(context.operation, &request.target, false)?;
        let handle = self
            .known_handle(context.operation, handle_id.as_ref())
            .await?;
        let effect_request = self.effect_request(context, handle_id);
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
        self.validate_call(context, ProviderMethod::DisplayAdopt)?;
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

        let effect_request = self.effect_request(context, Some(request.handle.handle_id.clone()));
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

    async fn close_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<MutationReceipt> {
        self.validate_request(context, request, ProviderMethod::DisplayClose)?;
        let handle_id = self
            .target_handle_id(context.operation, &request.target, true)?
            .ok_or_else(|| self.invalid_request(context.operation))?;
        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::DisplayClose,
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
        let effect_request = self.effect_request(context, Some(handle_id));
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
                method: ProviderMethod::DisplayClose,
                digest: context.operation.request_digest.clone(),
                value: Box::new(receipt.clone()),
            },
        );
        Ok(receipt)
    }
}

impl Provider for WaylandDisplayProvider {
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

impl DisplayProvider for WaylandDisplayProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn open<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { self.open_inner(context, request).await })
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

    fn close<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move { self.close_inner(context, request).await })
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
            ProviderOperationContext,
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
        inspect_calls: AtomicUsize,
        adopt_calls: AtomicUsize,
        destroy_calls: AtomicUsize,
        delay_ms: AtomicU64,
        last: StdMutex<Option<DisplayEffectRequest>>,
    }

    impl FakeEffects {
        fn record(&self, request: &DisplayEffectRequest) {
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
    impl DisplayEffectPort for FakeEffects {
        fn live_capabilities(&self) -> DisplayLiveCapabilities {
            DisplayLiveCapabilities::REQUIRED
        }

        async fn health(
            &self,
            request: &DisplayEffectRequest,
        ) -> Result<DisplayEffectHealth, DisplayEffectError> {
            self.record(request);
            Ok(DisplayEffectHealth::Healthy)
        }

        async fn plan(
            &self,
            request: &DisplayEffectRequest,
        ) -> Result<DisplayEffectPlan, DisplayEffectError> {
            self.record(request);
            self.delay().await;
            Ok(DisplayEffectPlan {
                plan_id: PlanId::parse("display-plan").expect("plan id"),
                resource: request.context.resource.clone(),
            })
        }

        async fn ensure(
            &self,
            request: &DisplayEffectRequest,
            _plan: &DisplayEffectPlan,
        ) -> Result<DisplayEffectHandle, DisplayEffectError> {
            self.record(request);
            Ok(DisplayEffectHandle {
                handle_id: HandleId::parse("display-handle").expect("handle id"),
                resource: request.context.resource.clone(),
            })
        }

        async fn inspect(
            &self,
            request: &DisplayEffectRequest,
        ) -> Result<DisplayEffectObservation, DisplayEffectError> {
            self.record(request);
            self.inspect_calls.fetch_add(1, Ordering::SeqCst);
            Ok(DisplayEffectObservation {
                resource: request.context.resource.clone(),
                lifecycle: ObservedLifecycleState::Ready,
                health: DisplayEffectHealth::Healthy,
            })
        }

        async fn adopt(
            &self,
            request: &DisplayEffectRequest,
        ) -> Result<DisplayAdoptionOutcome, DisplayEffectError> {
            self.record(request);
            self.adopt_calls.fetch_add(1, Ordering::SeqCst);
            Ok(DisplayAdoptionOutcome {
                resource: request.context.resource.clone(),
                state: DisplayAdoptionState::Adopted {
                    lifecycle: ObservedLifecycleState::Ready,
                },
            })
        }

        async fn destroy(
            &self,
            request: &DisplayEffectRequest,
        ) -> Result<DisplayMutationOutcome, DisplayEffectError> {
            self.record(request);
            self.destroy_calls.fetch_add(1, Ordering::SeqCst);
            Ok(DisplayMutationOutcome {
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
        let realm_id = RealmId::parse(short_id('a')).expect("realm");
        ProviderDescriptor {
            schema_version: PROVIDER_SCHEMA_VERSION,
            provider_id: ProviderId::parse(short_id('b')).expect("provider"),
            authority: ProviderAuthority::Display,
            implementation_id: implementation_id(),
            api_version: ProviderApiVersion::V2,
            capabilities: ProviderCapabilitySet::new(vec![
                ProviderCapability(ProviderMethod::DisplayOpen),
                ProviderCapability(ProviderMethod::DisplayInspect),
                ProviderCapability(ProviderMethod::DisplayAdopt),
                ProviderCapability(ProviderMethod::DisplayClose),
            ])
            .expect("capabilities"),
            configuration_schema_fingerprint: fingerprint(1),
            configured_scope_digest: fingerprint(2),
            registry_generation: Generation::new(1).expect("generation"),
            placement: ProviderPlacement::TrustedFirstPartyInProcess {
                realm_id,
                controller_role: EndpointRole::RealmController,
            },
        }
    }

    fn binding() -> WaylandDisplayBinding {
        WaylandDisplayBinding {
            realm_id: RealmId::parse(short_id('a')).expect("realm"),
            workload_id: WorkloadId::parse(short_id('c')).expect("workload"),
            owner_role_id: RoleId::parse(short_id('d')).expect("role"),
            wayland_endpoint_id: DisplayEndpointId::parse("wayland-endpoint").expect("endpoint"),
            cross_domain_endpoint_id: DisplayEndpointId::parse("cross-domain-endpoint")
                .expect("endpoint"),
            waypipe_endpoint_id: DisplayEndpointId::parse("waypipe-endpoint").expect("endpoint"),
            proxy_endpoint_id: DisplayEndpointId::parse("proxy-endpoint").expect("endpoint"),
            resource_generation: Generation::new(1).expect("generation"),
        }
    }

    fn provider(effects: Arc<FakeEffects>) -> WaylandDisplayProvider {
        WaylandDisplayProvider::with_clock(descriptor(), binding(), effects, Arc::new(TestClock))
            .expect("provider")
    }

    fn request(
        method: ProviderMethod,
        input: d2b_contracts::v2_provider::ProviderOperationInput,
    ) -> ProviderOperationRequest {
        let descriptor = descriptor();
        let binding = binding();
        ProviderOperationRequest {
            context: ProviderOperationContext {
                schema_version: PROVIDER_SCHEMA_VERSION,
                operation_id: d2b_contracts::v2_provider::OperationId::parse("display-operation")
                    .expect("operation"),
                idempotency_key: IdempotencyKey::parse("display-idempotency").expect("idempotency"),
                request_digest: fingerprint(3),
                scope: AuthorizedProviderScope::Workload {
                    realm_id: binding.realm_id.clone(),
                    workload_id: binding.workload_id.clone(),
                },
                principal: PrincipalRef::parse("display-principal").expect("principal"),
                provider_id: descriptor.provider_id,
                provider_type: ProviderType::Display,
                provider_generation: Generation::new(1).expect("generation"),
                capability: ProviderCapability(method),
                method,
                policy_epoch: Generation::new(1).expect("generation"),
                authorization_decision_digest: fingerprint(4),
                issued_at_unix_ms: NOW - 1_000,
                expires_at_unix_ms: NOW + 60_000,
                correlation_id: d2b_contracts::v2_provider::CorrelationId::parse(
                    "display-correlation",
                )
                .expect("correlation"),
                trace_id: fingerprint(5),
            },
            target: ProviderTarget::Workload {
                realm_id: binding.realm_id,
                workload_id: binding.workload_id,
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

    fn request_with_id(
        method: ProviderMethod,
        input: d2b_contracts::v2_provider::ProviderOperationInput,
        operation_id: &str,
    ) -> ProviderOperationRequest {
        let mut request = request(method, input);
        request.context.operation_id =
            d2b_contracts::v2_provider::OperationId::parse(operation_id).expect("operation");
        request.context.idempotency_key =
            IdempotencyKey::parse(format!("{operation_id}-idempotency")).expect("idempotency");
        request.context.request_digest = fingerprint(match method {
            ProviderMethod::DisplayOpen => 3,
            ProviderMethod::DisplayInspect => 6,
            ProviderMethod::DisplayAdopt => 7,
            ProviderMethod::DisplayClose => 8,
            _ => 9,
        });
        request
    }

    #[test]
    fn advertises_only_live_canonical_capabilities() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects);
        assert_eq!(provider.capabilities(), descriptor().capabilities);

        struct MissingCapability;
        #[async_trait]
        impl DisplayEffectPort for MissingCapability {
            fn live_capabilities(&self) -> DisplayLiveCapabilities {
                DisplayLiveCapabilities {
                    waypipe: false,
                    ..DisplayLiveCapabilities::REQUIRED
                }
            }
            async fn health(
                &self,
                _: &DisplayEffectRequest,
            ) -> Result<DisplayEffectHealth, DisplayEffectError> {
                unreachable!()
            }
            async fn plan(
                &self,
                _: &DisplayEffectRequest,
            ) -> Result<DisplayEffectPlan, DisplayEffectError> {
                unreachable!()
            }
            async fn ensure(
                &self,
                _: &DisplayEffectRequest,
                _: &DisplayEffectPlan,
            ) -> Result<DisplayEffectHandle, DisplayEffectError> {
                unreachable!()
            }
            async fn inspect(
                &self,
                _: &DisplayEffectRequest,
            ) -> Result<DisplayEffectObservation, DisplayEffectError> {
                unreachable!()
            }
            async fn adopt(
                &self,
                _: &DisplayEffectRequest,
            ) -> Result<DisplayAdoptionOutcome, DisplayEffectError> {
                unreachable!()
            }
            async fn destroy(
                &self,
                _: &DisplayEffectRequest,
            ) -> Result<DisplayMutationOutcome, DisplayEffectError> {
                unreachable!()
            }
        }
        assert!(matches!(
            WaylandDisplayProvider::new(descriptor(), binding(), Arc::new(MissingCapability)),
            Err(DisplayBuildError::MissingLiveCapability)
        ));
    }

    #[test]
    fn factory_registers_and_rejects_wrong_descriptor_axis() {
        let effects = Arc::new(FakeEffects::default());
        let factory = Arc::new(WaylandDisplayFactory::with_clock(
            binding(),
            effects,
            Arc::new(TestClock),
        ));
        let descriptor = descriptor();
        let key = provider_factory_key();
        assert_eq!(key.provider_type, ProviderType::Display);
        assert_eq!(key.implementation_id, implementation_id());

        let mut wrong_type = descriptor.clone();
        wrong_type.authority = ProviderAuthority::Network;
        assert!(matches!(
            factory.construct(&wrong_type),
            Err(FactoryError::Rejected)
        ));

        let mut wrong_implementation = descriptor.clone();
        wrong_implementation.implementation_id =
            ImplementationId::parse("other-display").expect("implementation");
        assert!(matches!(
            factory.construct(&wrong_implementation),
            Err(FactoryError::Rejected)
        ));

        let mut builder = d2b_provider::ProviderRegistryBuilder::new(
            descriptor.registry_generation,
            fingerprint(11),
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
    async fn open_forwards_only_authorized_generated_ids_and_is_idempotent() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let request = request(
            ProviderMethod::DisplayOpen,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
        );
        let context = call_context(&request.context, 30_000, false);
        let first = provider.open(&context, &request).await.expect("open");
        let second = provider.open(&context, &request).await.expect("repeat");
        assert_eq!(first, second);
        assert_eq!(effects.calls.load(Ordering::SeqCst), 2);

        let captured = effects
            .last
            .lock()
            .expect("last request lock")
            .clone()
            .expect("captured request");
        assert_eq!(captured.topology.owner_role_id, binding().owner_role_id);
        assert_eq!(
            captured.topology.proxy_endpoint_id,
            binding().proxy_endpoint_id
        );
        assert_eq!(
            captured.context.authorization_decision_digest,
            fingerprint(4)
        );
    }

    #[tokio::test]
    async fn wrong_input_and_cancelled_call_have_zero_effect() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let wrong = request(
            ProviderMethod::DisplayOpen,
            d2b_contracts::v2_provider::ProviderOperationInput::StorageSnapshot {
                snapshot_id: d2b_contracts::v2_provider::StorageSnapshotId::parse("snapshot")
                    .expect("snapshot"),
            },
        );
        let wrong_context = call_context(&wrong.context, 30_000, false);
        assert!(provider.open(&wrong_context, &wrong).await.is_err());

        let cancelled = request(
            ProviderMethod::DisplayInspect,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
        );
        let cancelled_context = call_context(&cancelled.context, 30_000, true);
        let cancelled_error = provider
            .inspect(&cancelled_context, &cancelled)
            .await
            .expect_err("cancelled");
        assert_eq!(cancelled_error.kind, ProviderFailureKind::Cancelled);

        let mut wrong_scope = request_with_id(
            ProviderMethod::DisplayOpen,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
            "display-wrong-scope",
        );
        wrong_scope.context.scope = AuthorizedProviderScope::Workload {
            realm_id: binding().realm_id,
            workload_id: WorkloadId::parse(short_id('e')).expect("workload"),
        };
        let wrong_scope_context = call_context(&wrong_scope.context, 30_000, false);
        assert!(
            provider
                .open(&wrong_scope_context, &wrong_scope)
                .await
                .is_err()
        );
        assert_eq!(effects.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_adoption_is_rejected_before_effect() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let open_request = request(
            ProviderMethod::DisplayOpen,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
        );
        let open_context = call_context(&open_request.context, 30_000, false);
        let handle = provider
            .open(&open_context, &open_request)
            .await
            .expect("open");
        let mut adopt_operation = request(
            ProviderMethod::DisplayAdopt,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
        )
        .context;
        adopt_operation.operation_id =
            d2b_contracts::v2_provider::OperationId::parse("display-adopt").expect("operation");
        let adoption = AdoptionRequest {
            context: adopt_operation,
            handle: handle.clone(),
            expected_owner: handle.owner.clone(),
            expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
            expected_resource_generation: Generation::new(2).expect("stale generation"),
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
    async fn inspect_adopt_and_close_use_the_authorized_handle() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let open = request_with_id(
            ProviderMethod::DisplayOpen,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
            "display-open-valid",
        );
        let open_context = call_context(&open.context, 30_000, false);
        let handle = provider.open(&open_context, &open).await.expect("open");

        let mut inspect = request_with_id(
            ProviderMethod::DisplayInspect,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
            "display-inspect",
        );
        inspect.target = ProviderTarget::Handle {
            realm_id: binding().realm_id,
            workload_id: Some(binding().workload_id),
            handle_id: handle.handle_id.clone(),
            handle_generation: handle.resource_generation,
        };
        let inspect_context = call_context(&inspect.context, 30_000, false);
        let observation = provider
            .inspect(&inspect_context, &inspect)
            .await
            .expect("inspect");
        assert_eq!(observation.handle_id.as_ref(), Some(&handle.handle_id));

        let adoption = AdoptionRequest {
            context: request_with_id(
                ProviderMethod::DisplayAdopt,
                d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
                "display-adopt-valid",
            )
            .context,
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

        let mut close = request_with_id(
            ProviderMethod::DisplayClose,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
            "display-close",
        );
        close.target = ProviderTarget::Handle {
            realm_id: binding().realm_id,
            workload_id: Some(binding().workload_id),
            handle_id: handle.handle_id.clone(),
            handle_generation: handle.resource_generation,
        };
        let close_context = call_context(&close.context, 30_000, false);
        let first = provider.close(&close_context, &close).await.expect("close");
        let second = provider
            .close(&close_context, &close)
            .await
            .expect("repeat close");
        assert_eq!(first, second);
        assert_eq!(effects.inspect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(effects.adopt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(effects.destroy_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn deadline_bounds_read_only_plan_before_mutation() {
        let effects = Arc::new(FakeEffects::default());
        effects.delay_ms.store(25, Ordering::SeqCst);
        let provider = provider(effects.clone());
        let request = request(
            ProviderMethod::DisplayOpen,
            d2b_contracts::v2_provider::ProviderOperationInput::NoInput,
        );
        let context = call_context(&request.context, 1, false);
        let error = provider
            .open(&context, &request)
            .await
            .expect_err("deadline");
        assert_eq!(error.kind, ProviderFailureKind::DeadlineExpired);
        assert_eq!(effects.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn public_values_debug_and_errors_cannot_carry_paths_or_acl_text() {
        assert!(DisplayEndpointId::parse("/run/user/1000/wayland-1").is_err());
        assert!(DisplayEndpointId::parse("u:alice:rwx").is_err());
        let rendered = format!(
            "{:?} {:?} {}",
            binding(),
            DisplayEffectError::Rejected,
            DisplayBuildError::ScopeMismatch
        );
        for forbidden in ["/run/", "wayland-1", "u:alice:rwx"] {
            assert!(!rendered.contains(forbidden), "{rendered}");
        }
    }
}
