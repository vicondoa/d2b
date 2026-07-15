#![allow(clippy::result_large_err)]

use std::{error::Error, fmt, future::Future, sync::Arc, time::Duration};

use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_provider::{
        AdoptionRequest, AdoptionState, AuthorizedProviderScope, HandleOwner,
        MAX_SAFE_JSON_INTEGER, MutationReceipt, ObservationReason, ObservedLifecycleState,
        PROVIDER_SCHEMA_VERSION, PlannedResourceClass, Provider, ProviderCallContext,
        ProviderCapability, ProviderCapabilitySet, ProviderContractError, ProviderDescriptor,
        ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle, ProviderHandleKind,
        ProviderHealth, ProviderHealthReason, ProviderHealthState, ProviderMethod,
        ProviderObservation, ProviderOperationRequest, ProviderPlacement, ProviderPlan,
        ProviderRemediation, ProviderResult, RetryClass, RuntimeProvider,
    },
};
use d2b_provider::{CancellationToken, ProviderClock};
use d2b_provider_toolkit::ProviderValues;

use crate::{
    LocalRuntimeConfiguration, LocalRuntimeKind, RuntimeAdoptionControl, RuntimeAdoptionMismatch,
    RuntimeAdoptionOutcome, RuntimeControlContext, RuntimeControlError, RuntimeControlPort,
    RuntimeEnsureControl, RuntimeHealth, RuntimeMutationOutcome, RuntimeObservedState,
    RuntimeOperationControl, RuntimePlanDecision, RuntimeResourceIdentity,
};

pub const LIVE_RUNTIME_METHODS: [ProviderMethod; 7] = [
    ProviderMethod::RuntimePlan,
    ProviderMethod::RuntimeEnsure,
    ProviderMethod::RuntimeStart,
    ProviderMethod::RuntimeStop,
    ProviderMethod::RuntimeInspect,
    ProviderMethod::RuntimeAdopt,
    ProviderMethod::RuntimeDestroy,
];

pub fn live_runtime_capabilities() -> Result<ProviderCapabilitySet, ProviderContractError> {
    ProviderCapabilitySet::new(
        LIVE_RUNTIME_METHODS
            .into_iter()
            .map(ProviderCapability)
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRuntimeProviderBuildError {
    Contract(ProviderContractError),
    FactoryEntriesEmpty,
    FactoryEntryBoundExceeded,
    DuplicateProviderEntry,
    RuntimeKindMismatch,
    ProviderTypeMismatch,
    ImplementationMismatch,
    CapabilityMismatch,
    AuthorityMismatch,
    PlacementMismatch,
}

impl fmt::Display for LocalRuntimeProviderBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => {
                write!(
                    formatter,
                    "local runtime provider contract is invalid ({error})"
                )
            }
            Self::FactoryEntriesEmpty => {
                formatter.write_str("local runtime factory has no entries")
            }
            Self::FactoryEntryBoundExceeded => {
                formatter.write_str("local runtime factory entry bound exceeded")
            }
            Self::DuplicateProviderEntry => {
                formatter.write_str("duplicate local runtime provider entry")
            }
            Self::RuntimeKindMismatch => {
                formatter.write_str("local runtime factory entry has the wrong runtime kind")
            }
            Self::ProviderTypeMismatch => {
                formatter.write_str("local runtime descriptor has the wrong provider type")
            }
            Self::ImplementationMismatch => {
                formatter.write_str("local runtime implementation does not match configuration")
            }
            Self::CapabilityMismatch => {
                formatter.write_str("local runtime capabilities do not match live methods")
            }
            Self::AuthorityMismatch => {
                formatter.write_str("local runtime authority posture does not match implementation")
            }
            Self::PlacementMismatch => {
                formatter.write_str("local runtime placement does not match implementation")
            }
        }
    }
}

impl Error for LocalRuntimeProviderBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProviderContractError> for LocalRuntimeProviderBuildError {
    fn from(value: ProviderContractError) -> Self {
        Self::Contract(value)
    }
}

#[derive(Clone)]
pub struct LocalRuntimeProvider {
    descriptor: ProviderDescriptor,
    configuration: LocalRuntimeConfiguration,
    control: Arc<dyn RuntimeControlPort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for LocalRuntimeProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRuntimeProvider")
            .field("kind", &self.kind())
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

impl LocalRuntimeProvider {
    pub(crate) fn with_clock(
        descriptor: ProviderDescriptor,
        configuration: LocalRuntimeConfiguration,
        control: Arc<dyn RuntimeControlPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        descriptor.validate()?;
        let kind = configuration.kind();
        if descriptor.provider_type() != d2b_contracts::v2_identity::ProviderType::Runtime {
            return Err(LocalRuntimeProviderBuildError::ProviderTypeMismatch);
        }
        if descriptor.implementation_id.as_str() != kind.implementation_id() {
            return Err(LocalRuntimeProviderBuildError::ImplementationMismatch);
        }
        if descriptor.capabilities != live_runtime_capabilities()? {
            return Err(LocalRuntimeProviderBuildError::CapabilityMismatch);
        }
        if descriptor.authority
            != (d2b_contracts::v2_provider::ProviderAuthority::Runtime {
                posture: kind.authority_posture(),
            })
        {
            return Err(LocalRuntimeProviderBuildError::AuthorityMismatch);
        }
        let placement_matches = match kind {
            LocalRuntimeKind::CloudHypervisor | LocalRuntimeKind::QemuMedia => matches!(
                &descriptor.placement,
                ProviderPlacement::TrustedFirstPartyInProcess {
                    controller_role: EndpointRole::RealmController,
                    ..
                }
            ),
            LocalRuntimeKind::SystemdUser => matches!(
                &descriptor.placement,
                ProviderPlacement::UserAgent {
                    endpoint_role: EndpointRole::UserAgent,
                    service: ServicePackage::UserV2,
                    ..
                }
            ),
        };
        if !placement_matches {
            return Err(LocalRuntimeProviderBuildError::PlacementMismatch);
        }
        Ok(Self {
            descriptor,
            configuration,
            control,
            clock,
        })
    }

    pub fn kind(&self) -> LocalRuntimeKind {
        self.configuration.kind()
    }

    pub fn configuration(&self) -> &LocalRuntimeConfiguration {
        &self.configuration
    }

    fn current_time(&self) -> u64 {
        self.clock.now_unix_ms()
    }

    fn direct_failure(
        &self,
        context: &ProviderCallContext<'_>,
        now_unix_ms: u64,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry,
            provider_type: self.descriptor.provider_type(),
            binding: context.operation.binding(),
            correlation_id: context.operation.correlation_id.clone(),
            occurred_at_unix_ms: now_unix_ms.min(MAX_SAFE_JSON_INTEGER),
            reason,
            remediation,
        }
    }

    fn failure(
        &self,
        context: &ProviderCallContext<'_>,
        now_unix_ms: u64,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        if now_unix_ms <= MAX_SAFE_JSON_INTEGER
            && context
                .operation
                .validate(&self.descriptor, now_unix_ms)
                .is_ok()
            && let Ok(values) = ProviderValues::new(&self.descriptor, now_unix_ms)
            && let Ok(failure) = values.failure(context.operation, kind, retry, reason, remediation)
        {
            return failure;
        }
        self.direct_failure(context, now_unix_ms, kind, retry, reason, remediation)
    }

    fn contract_failure(
        &self,
        context: &ProviderCallContext<'_>,
        now_unix_ms: u64,
        error: ProviderContractError,
    ) -> ProviderFailure {
        let (kind, retry, reason, remediation) = match error {
            ProviderContractError::ScopeMismatch => (
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ),
            ProviderContractError::CapabilityMismatch
            | ProviderContractError::MissingRequiredCapability => (
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            ProviderContractError::RequestExpired => (
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ),
            ProviderContractError::AdoptionAmbiguous
            | ProviderContractError::AdoptionEvidenceMismatch
            | ProviderContractError::HandleBindingMismatch
            | ProviderContractError::OwnershipTransferInvalid => (
                ProviderFailureKind::AdoptionRejected,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::OperatorInteraction,
            ),
            _ => (
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
        };
        self.failure(context, now_unix_ms, kind, retry, reason, remediation)
    }

    fn placement_matches_call(&self, context: &ProviderCallContext<'_>) -> bool {
        match self.kind() {
            LocalRuntimeKind::CloudHypervisor | LocalRuntimeKind::QemuMedia => {
                context.peer_role == EndpointRole::RealmController
                    && context.service == ServicePackage::ProviderV2
            }
            LocalRuntimeKind::SystemdUser => {
                context.peer_role == EndpointRole::UserAgent
                    && context.service == ServicePackage::UserV2
            }
        }
    }

    fn preflight(
        &self,
        context: &ProviderCallContext<'_>,
        expected: ProviderMethod,
    ) -> ProviderResult<PreparedCall> {
        let now_unix_ms = self.current_time();
        if context.cancelled {
            return Err(self.failure(
                context,
                now_unix_ms,
                ProviderFailureKind::Cancelled,
                RetryClass::SameOperation,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.failure(
                context,
                now_unix_ms,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ));
        }
        if now_unix_ms > MAX_SAFE_JSON_INTEGER {
            return Err(self.direct_failure(
                context,
                now_unix_ms,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::HealthStale,
                ProviderRemediation::OperatorInteraction,
            ));
        }
        if let Err(error) = context.validate() {
            return Err(self.contract_failure(context, now_unix_ms, error));
        }
        if let Err(error) = context.operation.validate(&self.descriptor, now_unix_ms) {
            return Err(self.contract_failure(context, now_unix_ms, error));
        }
        if context.operation.method != expected {
            return Err(self.failure(
                context,
                now_unix_ms,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        if !self.placement_matches_call(context) {
            return Err(self.failure(
                context,
                now_unix_ms,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        let wall_remaining_ms = context
            .operation
            .expires_at_unix_ms
            .saturating_sub(now_unix_ms);
        let effective_deadline_remaining_ms =
            wall_remaining_ms.min(u64::from(context.monotonic_deadline_remaining_ms));
        if effective_deadline_remaining_ms == 0 {
            return Err(self.failure(
                context,
                now_unix_ms,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::Never,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::OperatorInteraction,
            ));
        }
        let effective_deadline_remaining_ms = u32::try_from(effective_deadline_remaining_ms)
            .map_err(|_| {
                self.failure(
                    context,
                    now_unix_ms,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::HealthStale,
                    ProviderRemediation::OperatorInteraction,
                )
            })?;
        Ok(PreparedCall {
            now_unix_ms,
            deadline: Duration::from_millis(u64::from(effective_deadline_remaining_ms)),
            effective_deadline_remaining_ms,
            cancellation: CancellationToken::new(),
        })
    }

    fn validate_operation_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
        now_unix_ms: u64,
        allow_handle: bool,
    ) -> ProviderResult<()> {
        if request.context != *context.operation {
            return Err(self.failure(
                context,
                now_unix_ms,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        if let Err(error) = request.validate_method(&self.descriptor, now_unix_ms, expected) {
            return Err(self.contract_failure(context, now_unix_ms, error));
        }
        let target_valid = match &request.target {
            d2b_contracts::v2_provider::ProviderTarget::Workload { .. } => true,
            d2b_contracts::v2_provider::ProviderTarget::Handle {
                workload_id: Some(_),
                ..
            } => allow_handle,
            d2b_contracts::v2_provider::ProviderTarget::Realm { .. }
            | d2b_contracts::v2_provider::ProviderTarget::Handle {
                workload_id: None, ..
            } => false,
        };
        if !target_valid {
            return Err(self.failure(
                context,
                now_unix_ms,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        Ok(())
    }

    fn validate_plan(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
        now_unix_ms: u64,
    ) -> ProviderResult<()> {
        let valid = plan.schema_version == PROVIDER_SCHEMA_VERSION
            && plan.binding == context.operation.binding()
            && plan.realm_id == *context.operation.scope.realm_id()
            && plan.workload_id.as_ref() == context.operation.scope.workload_id()
            && plan.method == ProviderMethod::RuntimePlan
            && plan.configuration_fingerprint == self.descriptor.configuration_schema_fingerprint
            && plan.created_at_unix_ms <= now_unix_ms
            && plan.expires_at_unix_ms > now_unix_ms
            && plan.resources.as_slice() == [PlannedResourceClass::WorkloadExecution];
        if valid {
            Ok(())
        } else {
            Err(self.failure(
                context,
                now_unix_ms,
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ))
        }
    }

    fn control_context(
        &self,
        context: &ProviderCallContext<'_>,
        prepared: &PreparedCall,
    ) -> RuntimeControlContext {
        RuntimeControlContext::new(
            self.kind(),
            context.operation.clone(),
            prepared.effective_deadline_remaining_ms,
            prepared.cancellation.clone(),
        )
    }

    async fn invoke_control<T, F>(
        &self,
        context: &ProviderCallContext<'_>,
        prepared: PreparedCall,
        mutation: bool,
        future: F,
    ) -> ProviderResult<T>
    where
        F: Future<Output = Result<T, RuntimeControlError>> + Send,
    {
        let mut cancellation_guard = ControlCancellationGuard::new(prepared.cancellation.clone());
        match tokio::time::timeout(prepared.deadline, future).await {
            Ok(result) => {
                cancellation_guard.disarm();
                match result {
                    Ok(value) => Ok(value),
                    Err(error) => Err(self.control_failure(context, error)),
                }
            }
            Err(_) => {
                drop(cancellation_guard);
                Err(self.control_failure(
                    context,
                    if mutation {
                        RuntimeControlError::CompletionAmbiguous
                    } else {
                        RuntimeControlError::DeadlineExpiredBeforeMutation
                    },
                ))
            }
        }
    }

    fn control_failure(
        &self,
        context: &ProviderCallContext<'_>,
        error: RuntimeControlError,
    ) -> ProviderFailure {
        let (kind, retry, reason, remediation) = match error {
            RuntimeControlError::InvalidRequest => (
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            RuntimeControlError::UnauthorizedScope => (
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ),
            RuntimeControlError::CancelledBeforeMutation => (
                ProviderFailureKind::Cancelled,
                RetryClass::SameOperation,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            RuntimeControlError::DeadlineExpiredBeforeMutation => (
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ),
            RuntimeControlError::Unavailable => (
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            RuntimeControlError::InvariantViolation => (
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::OperatorInteraction,
            ),
            RuntimeControlError::CompletionAmbiguous => (
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::HealthStale,
                ProviderRemediation::InspectProvider,
            ),
            RuntimeControlError::AdoptionRejected(mismatch) => {
                let (reason, remediation) = adoption_health(mismatch);
                (
                    ProviderFailureKind::AdoptionRejected,
                    RetryClass::Never,
                    reason,
                    remediation,
                )
            }
        };
        self.failure(
            context,
            self.current_time(),
            kind,
            retry,
            reason,
            remediation,
        )
    }

    fn invariant_failure(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            self.current_time(),
            ProviderFailureKind::InvariantViolation,
            RetryClass::Never,
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::OperatorInteraction,
        )
    }

    fn adoption_failure(
        &self,
        context: &ProviderCallContext<'_>,
        mismatch: RuntimeAdoptionMismatch,
    ) -> ProviderFailure {
        let (reason, remediation) = adoption_health(mismatch);
        self.failure(
            context,
            self.current_time(),
            ProviderFailureKind::AdoptionRejected,
            RetryClass::Never,
            reason,
            remediation,
        )
    }

    fn expected_owner(&self, scope: &AuthorizedProviderScope) -> HandleOwner {
        if self.kind().uses_user_agent() {
            HandleOwner::Provider {
                realm_id: scope.realm_id().clone(),
                provider_id: self.descriptor.provider_id.clone(),
            }
        } else {
            HandleOwner::RealmController {
                realm_id: scope.realm_id().clone(),
            }
        }
    }

    fn identity_mismatch(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        target: Option<&d2b_contracts::v2_provider::ProviderTarget>,
        identity: &RuntimeResourceIdentity,
    ) -> Option<RuntimeAdoptionMismatch> {
        if identity.kind() != self.kind() {
            return Some(RuntimeAdoptionMismatch::RuntimeKind);
        }
        if identity.provider_id() != &self.descriptor.provider_id {
            return Some(RuntimeAdoptionMismatch::ProviderIdentity);
        }
        if identity.provider_generation() != self.descriptor.registry_generation {
            return Some(RuntimeAdoptionMismatch::ProviderGeneration);
        }
        if identity.scope() != &operation.scope {
            return Some(RuntimeAdoptionMismatch::Scope);
        }
        if identity.configuration_fingerprint() != &self.descriptor.configuration_schema_fingerprint
        {
            return Some(RuntimeAdoptionMismatch::Configuration);
        }
        if identity.owner() != &self.expected_owner(&operation.scope) {
            return Some(RuntimeAdoptionMismatch::Owner);
        }
        if let Some(d2b_contracts::v2_provider::ProviderTarget::Handle {
            handle_id,
            handle_generation,
            ..
        }) = target
        {
            if identity.handle_id() != handle_id {
                return Some(RuntimeAdoptionMismatch::ProviderIdentity);
            }
            if identity.resource_generation() != *handle_generation {
                return Some(RuntimeAdoptionMismatch::ResourceGeneration);
            }
        }
        None
    }

    fn identity_from_handle(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        handle: &ProviderHandle,
    ) -> RuntimeResourceIdentity {
        RuntimeResourceIdentity::new(
            self.kind(),
            handle.provider_id.clone(),
            handle.provider_generation,
            operation.scope.clone(),
            handle.handle_id.clone(),
            handle.owner.clone(),
            handle.resource_generation,
            handle.configuration_fingerprint.clone(),
        )
    }

    fn values(&self, context: &ProviderCallContext<'_>) -> ProviderResult<ProviderValues> {
        ProviderValues::new(&self.descriptor, self.current_time())
            .map_err(|_| self.invariant_failure(context))
    }

    fn map_health(
        &self,
        context: &ProviderCallContext<'_>,
        health: RuntimeHealth,
    ) -> ProviderResult<ProviderHealth> {
        self.values(context)?
            .health(health.state(), health.reason(), health.remediation())
            .map_err(|_| self.invariant_failure(context))
    }

    fn map_observation(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        state: RuntimeObservedState,
        adoption: AdoptionState,
    ) -> ProviderResult<ProviderObservation> {
        let values = self.values(context)?;
        let handle = if let Some(identity) = state.identity() {
            if let Some(mismatch) =
                self.identity_mismatch(context.operation, Some(&request.target), identity)
            {
                return Err(self.adoption_failure(context, mismatch));
            }
            Some(
                values
                    .handle_from_request(
                        request,
                        identity.handle_id().clone(),
                        identity.owner().clone(),
                        identity.resource_generation(),
                        None,
                    )
                    .map_err(|_| self.invariant_failure(context))?,
            )
        } else {
            None
        };
        values
            .observation(
                context.operation,
                handle.as_ref(),
                state.lifecycle(),
                adoption,
                state.reason(),
                state.health().state(),
                state.health().reason(),
                state.health().remediation(),
            )
            .map_err(|_| self.invariant_failure(context))
    }

    fn map_adopted_observation(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
        state: RuntimeObservedState,
    ) -> ProviderResult<ProviderObservation> {
        let Some(identity) = state.identity() else {
            return Err(self.adoption_failure(context, RuntimeAdoptionMismatch::MissingEvidence));
        };
        if let Some(mismatch) = self.identity_mismatch(context.operation, None, identity) {
            return Err(self.adoption_failure(context, mismatch));
        }
        let expected = self.identity_from_handle(context.operation, &request.handle);
        if identity != &expected {
            return Err(self.adoption_failure(context, identity_difference(&expected, identity)));
        }
        self.values(context)?
            .observation(
                context.operation,
                Some(&request.handle),
                state.lifecycle(),
                AdoptionState::Adopted,
                state.reason(),
                state.health().state(),
                state.health().reason(),
                state.health().remediation(),
            )
            .map_err(|_| self.invariant_failure(context))
    }

    fn rejected_adoption_observation(
        &self,
        context: &ProviderCallContext<'_>,
        mismatch: RuntimeAdoptionMismatch,
    ) -> ProviderResult<ProviderObservation> {
        let (observation_reason, health_reason, remediation) = adoption_observation(mismatch);
        self.values(context)?
            .observation(
                context.operation,
                None,
                ObservedLifecycleState::Quarantined,
                AdoptionState::Rejected,
                observation_reason,
                ProviderHealthState::Failed,
                health_reason,
                remediation,
            )
            .map_err(|_| self.invariant_failure(context))
    }

    fn ambiguous_adoption_observation(
        &self,
        context: &ProviderCallContext<'_>,
    ) -> ProviderResult<ProviderObservation> {
        self.values(context)?
            .observation(
                context.operation,
                None,
                ObservedLifecycleState::Quarantined,
                AdoptionState::Ambiguous,
                ObservationReason::MultipleCandidates,
                ProviderHealthState::Failed,
                ProviderHealthReason::AdoptionAmbiguous,
                ProviderRemediation::OperatorInteraction,
            )
            .map_err(|_| self.invariant_failure(context))
    }

    async fn health_inner(
        &self,
        context: &ProviderCallContext<'_>,
    ) -> ProviderResult<ProviderHealth> {
        let prepared = self.preflight(context, context.operation.method)?;
        let control_context = self.control_context(context, &prepared);
        let health = self
            .invoke_control(
                context,
                prepared,
                false,
                self.control.health(control_context),
            )
            .await?;
        self.map_health(context, health)
    }

    async fn plan_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderPlan> {
        let prepared = self.preflight(context, ProviderMethod::RuntimePlan)?;
        self.validate_operation_request(
            context,
            request,
            ProviderMethod::RuntimePlan,
            prepared.now_unix_ms,
            false,
        )?;
        let control_request = RuntimeOperationControl::new(
            self.control_context(context, &prepared),
            request.target.clone(),
        );
        let decision: RuntimePlanDecision = self
            .invoke_control(context, prepared, false, self.control.plan(control_request))
            .await?;
        let resources = BoundedVec::new(vec![PlannedResourceClass::WorkloadExecution])
            .map_err(|_| self.invariant_failure(context))?;
        self.values(context)?
            .plan(
                request,
                decision.plan_id().clone(),
                decision.expires_at_unix_ms(),
                resources,
            )
            .map_err(|_| self.invariant_failure(context))
    }

    async fn ensure_inner(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
    ) -> ProviderResult<ProviderHandle> {
        let prepared = self.preflight(context, ProviderMethod::RuntimeEnsure)?;
        self.validate_plan(context, plan, prepared.now_unix_ms)?;
        let control_request =
            RuntimeEnsureControl::new(self.control_context(context, &prepared), plan.clone());
        let identity = self
            .invoke_control(
                context,
                prepared,
                true,
                self.control.ensure(control_request),
            )
            .await?;
        if let Some(mismatch) = self.identity_mismatch(context.operation, None, &identity) {
            return Err(self.adoption_failure(context, mismatch));
        }
        self.values(context)?
            .handle_from_plan(
                plan,
                identity.handle_id().clone(),
                identity.owner().clone(),
                identity.resource_generation(),
                None,
            )
            .map_err(|_| self.invariant_failure(context))
    }

    async fn operation_observation(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        method: ProviderMethod,
    ) -> ProviderResult<ProviderObservation> {
        let prepared = self.preflight(context, method)?;
        self.validate_operation_request(context, request, method, prepared.now_unix_ms, true)?;
        let control_request = RuntimeOperationControl::new(
            self.control_context(context, &prepared),
            request.target.clone(),
        );
        let mutation = method != ProviderMethod::RuntimeInspect;
        let future = match method {
            ProviderMethod::RuntimeStart => self.control.start(control_request),
            ProviderMethod::RuntimeStop => self.control.stop(control_request),
            ProviderMethod::RuntimeInspect => self.control.inspect(control_request),
            _ => return Err(self.invariant_failure(context)),
        };
        let state = self
            .invoke_control(context, prepared, mutation, future)
            .await?;
        let lifecycle_valid = match method {
            ProviderMethod::RuntimeStart => matches!(
                state.lifecycle(),
                ObservedLifecycleState::Ready | ObservedLifecycleState::Running
            ),
            ProviderMethod::RuntimeStop => state.lifecycle() == ObservedLifecycleState::Stopped,
            ProviderMethod::RuntimeInspect => true,
            _ => false,
        };
        if !lifecycle_valid {
            return Err(self.invariant_failure(context));
        }
        self.map_observation(context, request, state, AdoptionState::NotAttempted)
    }

    async fn adopt_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
    ) -> ProviderResult<ProviderObservation> {
        let prepared = self.preflight(context, ProviderMethod::RuntimeAdopt)?;
        if request.context != *context.operation {
            return Err(self.failure(
                context,
                prepared.now_unix_ms,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        if let Err(error) = request.validate(&self.descriptor, prepared.now_unix_ms) {
            return Err(self.contract_failure(context, prepared.now_unix_ms, error));
        }
        let handle_scope_matches = request.handle.kind == ProviderHandleKind::Runtime
            && request.handle.realm_id == *context.operation.scope.realm_id()
            && request.handle.workload_id.as_ref() == context.operation.scope.workload_id()
            && request.expected_configuration_fingerprint
                == self.descriptor.configuration_schema_fingerprint
            && request.expected_owner == self.expected_owner(&context.operation.scope)
            && request
                .handle
                .expires_at_unix_ms
                .is_none_or(|expiry| expiry > prepared.now_unix_ms);
        if !handle_scope_matches {
            return Err(self.adoption_failure(context, RuntimeAdoptionMismatch::Scope));
        }
        let expected = self.identity_from_handle(context.operation, &request.handle);
        let control_request =
            RuntimeAdoptionControl::new(self.control_context(context, &prepared), expected);
        let outcome = self
            .invoke_control(context, prepared, true, self.control.adopt(control_request))
            .await?;
        match outcome {
            RuntimeAdoptionOutcome::Adopted(state) => {
                self.map_adopted_observation(context, request, *state)
            }
            RuntimeAdoptionOutcome::Rejected(mismatch) => {
                self.rejected_adoption_observation(context, mismatch)
            }
            RuntimeAdoptionOutcome::Ambiguous => self.ambiguous_adoption_observation(context),
        }
    }

    async fn destroy_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<MutationReceipt> {
        let prepared = self.preflight(context, ProviderMethod::RuntimeDestroy)?;
        self.validate_operation_request(
            context,
            request,
            ProviderMethod::RuntimeDestroy,
            prepared.now_unix_ms,
            true,
        )?;
        let control_request = RuntimeOperationControl::new(
            self.control_context(context, &prepared),
            request.target.clone(),
        );
        let outcome: RuntimeMutationOutcome = self
            .invoke_control(
                context,
                prepared,
                true,
                self.control.destroy(control_request),
            )
            .await?;
        self.values(context)?
            .receipt(context.operation, outcome.state())
            .map_err(|_| self.invariant_failure(context))
    }
}

#[derive(Debug, Clone)]
struct PreparedCall {
    now_unix_ms: u64,
    deadline: Duration,
    effective_deadline_remaining_ms: u32,
    cancellation: CancellationToken,
}

struct ControlCancellationGuard {
    cancellation: CancellationToken,
    armed: bool,
}

impl ControlCancellationGuard {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ControlCancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.cancel();
        }
    }
}

fn adoption_health(
    mismatch: RuntimeAdoptionMismatch,
) -> (ProviderHealthReason, ProviderRemediation) {
    match mismatch {
        RuntimeAdoptionMismatch::ProviderGeneration
        | RuntimeAdoptionMismatch::ResourceGeneration => (
            ProviderHealthReason::GenerationMismatch,
            ProviderRemediation::ReplaceGeneration,
        ),
        RuntimeAdoptionMismatch::Configuration => (
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::RepairConfiguration,
        ),
        RuntimeAdoptionMismatch::RuntimeKind
        | RuntimeAdoptionMismatch::ProviderIdentity
        | RuntimeAdoptionMismatch::Scope
        | RuntimeAdoptionMismatch::Owner
        | RuntimeAdoptionMismatch::MissingEvidence => (
            ProviderHealthReason::IdentityMismatch,
            ProviderRemediation::OperatorInteraction,
        ),
    }
}

fn adoption_observation(
    mismatch: RuntimeAdoptionMismatch,
) -> (ObservationReason, ProviderHealthReason, ProviderRemediation) {
    let (health_reason, remediation) = adoption_health(mismatch);
    let observation_reason = match mismatch {
        RuntimeAdoptionMismatch::ProviderGeneration
        | RuntimeAdoptionMismatch::ResourceGeneration => ObservationReason::GenerationMismatch,
        RuntimeAdoptionMismatch::Configuration => ObservationReason::ConfigurationMismatch,
        RuntimeAdoptionMismatch::Owner => ObservationReason::OwnerMismatch,
        RuntimeAdoptionMismatch::MissingEvidence => ObservationReason::MissingEvidence,
        RuntimeAdoptionMismatch::RuntimeKind
        | RuntimeAdoptionMismatch::ProviderIdentity
        | RuntimeAdoptionMismatch::Scope => ObservationReason::IdentityMismatch,
    };
    (observation_reason, health_reason, remediation)
}

fn identity_difference(
    expected: &RuntimeResourceIdentity,
    observed: &RuntimeResourceIdentity,
) -> RuntimeAdoptionMismatch {
    if expected.kind() != observed.kind() {
        RuntimeAdoptionMismatch::RuntimeKind
    } else if expected.provider_id() != observed.provider_id()
        || expected.handle_id() != observed.handle_id()
    {
        RuntimeAdoptionMismatch::ProviderIdentity
    } else if expected.provider_generation() != observed.provider_generation() {
        RuntimeAdoptionMismatch::ProviderGeneration
    } else if expected.resource_generation() != observed.resource_generation() {
        RuntimeAdoptionMismatch::ResourceGeneration
    } else if expected.configuration_fingerprint() != observed.configuration_fingerprint() {
        RuntimeAdoptionMismatch::Configuration
    } else if expected.scope() != observed.scope() {
        RuntimeAdoptionMismatch::Scope
    } else if expected.owner() != observed.owner() {
        RuntimeAdoptionMismatch::Owner
    } else {
        RuntimeAdoptionMismatch::MissingEvidence
    }
}

impl Provider for LocalRuntimeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(self.health_inner(context))
    }
}

impl RuntimeProvider for LocalRuntimeProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn plan<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderPlan> {
        Box::pin(self.plan_inner(context, request))
    }

    fn ensure<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        plan: &'a ProviderPlan,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(self.ensure_inner(context, plan))
    }

    fn start<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.operation_observation(context, request, ProviderMethod::RuntimeStart))
    }

    fn stop<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.operation_observation(context, request, ProviderMethod::RuntimeStop))
    }

    fn inspect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.operation_observation(context, request, ProviderMethod::RuntimeInspect))
    }

    fn adopt<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a AdoptionRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.adopt_inner(context, request))
    }

    fn destroy<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(self.destroy_inner(context, request))
    }
}
