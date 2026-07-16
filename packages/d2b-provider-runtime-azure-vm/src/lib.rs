//! Explicitly non-production Azure VM workload runtime provider scaffold.
//!
//! Runtime operations require an already-bound opaque infrastructure handle.
//! Canonical dispatch and production registration remain unavailable; explicit
//! conformance APIs exercise workload-only behavior through the fake SDK.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{error::Error, fmt};

use std::sync::Arc;

use d2b_azure_vm_fake_sdk::{
    ApplyDisposition, BindingMaterialError, DeploymentHandle, DeploymentState, FakeAzureVmSdk,
    FakeSdkError, FakeSdkErrorKind, InfrastructureBindingFingerprint,
    InfrastructureBindingMaterial, InfrastructureHandle, ResourceGeneration, ResourceId,
    SdkCallContext,
};
use d2b_contracts::{
    v2_component_session::BoundedVec,
    v2_identity::{ProviderId, ProviderType, RealmId},
    v2_provider::{
        AdoptionRequest, AdoptionState, Fingerprint, Generation, HandleId, HandleOwner,
        MAX_PROVIDER_REQUEST_LIFETIME_MS, MAX_SAFE_JSON_INTEGER, MutationReceipt, MutationState,
        ObservationReason, ObservedLifecycleState, OwnershipTransfer, PROVIDER_SCHEMA_VERSION,
        PlanId, PlannedResourceClass, ProviderCallContext, ProviderFailure, ProviderFailureKind,
        ProviderHandle, ProviderHandleKind, ProviderHealth, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderObservation, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlan, ProviderRemediation, RetryClass,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldAvailability {
    Unavailable,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmRuntimeScaffoldDescriptor {
    provider_id: ProviderId,
    registry_generation: Generation,
    configuration_fingerprint: Fingerprint,
    realm_id: RealmId,
}

impl AzureVmRuntimeScaffoldDescriptor {
    pub fn new(
        provider_id: ProviderId,
        registry_generation: Generation,
        configuration_fingerprint: Fingerprint,
        realm_id: RealmId,
    ) -> Self {
        Self {
            provider_id,
            registry_generation,
            configuration_fingerprint,
            realm_id,
        }
    }

    pub const fn availability(&self) -> ScaffoldAvailability {
        ScaffoldAvailability::Unavailable
    }

    pub const fn advertised_capabilities(&self) -> &'static [ProviderMethod] {
        &[]
    }

    pub const fn is_registerable(&self) -> bool {
        false
    }
}

impl fmt::Debug for AzureVmRuntimeScaffoldDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmRuntimeScaffoldDescriptor")
            .field("availability", &self.availability())
            .field("provider_type", &ProviderType::Runtime)
            .field("registry_generation", &self.registry_generation)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldConstructionError {
    InvalidClock,
}

impl fmt::Display for ScaffoldConstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidClock => "Azure VM runtime scaffold clock is invalid",
        })
    }
}

impl Error for ScaffoldConstructionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfrastructureBindingError {
    InvalidHandle,
    WrongHandleKind,
    GenerationMismatch,
    BindingMismatch,
}

impl fmt::Display for InfrastructureBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidHandle => "opaque infrastructure handle is invalid",
            Self::WrongHandleKind => "opaque infrastructure handle has the wrong authority",
            Self::GenerationMismatch => "opaque infrastructure handle generation mismatch",
            Self::BindingMismatch => "opaque infrastructure handle binding mismatch",
        })
    }
}

impl Error for InfrastructureBindingError {}

pub struct AzureVmRuntimeScaffold {
    descriptor: AzureVmRuntimeScaffoldDescriptor,
    now_unix_ms: u64,
    sdk: Arc<FakeAzureVmSdk>,
}

impl fmt::Debug for AzureVmRuntimeScaffold {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmRuntimeScaffold")
            .field("availability", &self.descriptor.availability())
            .field("provider_type", &ProviderType::Runtime)
            .finish_non_exhaustive()
    }
}

impl AzureVmRuntimeScaffold {
    #[cfg(test)]
    const DIRECT_METHODS: &'static [ProviderMethod] = &[
        ProviderMethod::RuntimePlan,
        ProviderMethod::RuntimeEnsure,
        ProviderMethod::RuntimeStart,
        ProviderMethod::RuntimeStop,
        ProviderMethod::RuntimeInspect,
        ProviderMethod::RuntimeAdopt,
        ProviderMethod::RuntimeDestroy,
    ];

    pub fn new_for_conformance(
        descriptor: AzureVmRuntimeScaffoldDescriptor,
        sdk: Arc<FakeAzureVmSdk>,
        now_unix_ms: u64,
    ) -> Result<Self, ScaffoldConstructionError> {
        if !(1_001..=d2b_contracts::v2_provider::MAX_SAFE_JSON_INTEGER).contains(&now_unix_ms) {
            return Err(ScaffoldConstructionError::InvalidClock);
        }
        Ok(Self {
            descriptor,
            now_unix_ms,
            sdk,
        })
    }

    pub fn descriptor(&self) -> &AzureVmRuntimeScaffoldDescriptor {
        &self.descriptor
    }

    pub fn deny_unavailable_dispatch(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::CapabilityDenied,
            RetryClass::Never,
            ProviderHealthReason::CapabilityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn failure(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Runtime,
            binding: context.binding(),
            correlation_id: context.correlation_id.clone(),
            occurred_at_unix_ms: self.now_unix_ms,
            reason,
            remediation,
        }
    }

    fn invalid_request(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn operation_matches_scaffold(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        expected: ProviderMethod,
    ) -> bool {
        operation.schema_version == PROVIDER_SCHEMA_VERSION
            && operation.provider_id == self.descriptor.provider_id
            && operation.provider_type == ProviderType::Runtime
            && operation.provider_generation == self.descriptor.registry_generation
            && operation.method == expected
            && operation.capability.0 == expected
            && operation.scope.realm_id() == &self.descriptor.realm_id
            && operation.issued_at_unix_ms <= MAX_SAFE_JSON_INTEGER
            && operation.expires_at_unix_ms <= MAX_SAFE_JSON_INTEGER
            && operation.expires_at_unix_ms > operation.issued_at_unix_ms
            && operation.expires_at_unix_ms - operation.issued_at_unix_ms
                <= MAX_PROVIDER_REQUEST_LIFETIME_MS
            && self.now_unix_ms < operation.expires_at_unix_ms
    }

    fn validate_call(
        &self,
        context: &ProviderCallContext<'_>,
        expected: ProviderMethod,
    ) -> Result<(), ProviderFailure> {
        if expected.provider_type() != ProviderType::Runtime
            || context.operation.method.provider_type() != ProviderType::Runtime
            || context.operation.method != expected
        {
            return Err(self.deny_unavailable_dispatch(context.operation));
        }
        if context.cancelled {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::Never,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ));
        }
        context.validate().map_err(|_| {
            self.failure(
                context.operation,
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            )
        })?;
        if self.operation_matches_scaffold(context.operation, expected) {
            Ok(())
        } else {
            Err(self.invalid_request(context.operation))
        }
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
    ) -> Result<(), ProviderFailure> {
        self.validate_call(context, expected)?;
        if context.operation != &request.context || !runtime_input_matches(request, expected) {
            return Err(self.deny_unavailable_dispatch(context.operation));
        }
        if request.target.realm_id() == request.context.scope.realm_id()
            && request.target.workload_id() == request.context.scope.workload_id()
            && request.expected_configuration_fingerprint
                == self.descriptor.configuration_fingerprint
        {
            Ok(())
        } else {
            Err(self.invalid_request(context.operation))
        }
    }

    fn validate_infrastructure_target(
        &self,
        request: &ProviderOperationRequest,
        infrastructure: &BoundInfrastructureHandle,
    ) -> Result<(), ProviderFailure> {
        let matches = match &request.target {
            d2b_contracts::v2_provider::ProviderTarget::Handle {
                realm_id,
                workload_id: Some(workload_id),
                handle_id,
                handle_generation,
            } => {
                realm_id == &infrastructure.provider.realm_id
                    && request.context.scope.workload_id() == Some(workload_id)
                    && handle_id == &infrastructure.provider.handle_id
                    && *handle_generation == infrastructure.provider.resource_generation
            }
            _ => false,
        };
        if matches && infrastructure.validate().is_ok() {
            Ok(())
        } else {
            Err(self.failure(
                &request.context,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::RepairConfiguration,
            ))
        }
    }

    fn validate_runtime_handle(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        handle: &AzureVmRuntimeHandle,
    ) -> Result<(), ProviderFailure> {
        let valid = handle.provider.validate().is_ok()
            && handle.provider.kind == ProviderHandleKind::Runtime
            && handle.provider.provider_id == self.descriptor.provider_id
            && handle.provider.provider_generation == self.descriptor.registry_generation
            && handle.provider.configuration_fingerprint
                == self.descriptor.configuration_fingerprint
            && handle.provider.workload_id.is_some()
            && handle.provider.resource_generation.get() == handle.sdk.generation().get()
            && handle.infrastructure.validate().is_ok()
            && handle.infrastructure.sdk == handle.sdk.infrastructure();
        if valid {
            Ok(())
        } else {
            Err(self.failure(
                context,
                ProviderFailureKind::AdoptionRejected,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::InspectProvider,
            ))
        }
    }

    fn validate_runtime_request(
        &self,
        request: &ProviderOperationRequest,
        handle: &AzureVmRuntimeHandle,
    ) -> Result<(), ProviderFailure> {
        self.validate_runtime_handle(&request.context, handle)?;
        let matches = match &request.target {
            d2b_contracts::v2_provider::ProviderTarget::Handle {
                realm_id,
                workload_id,
                handle_id,
                handle_generation,
            } => {
                realm_id == &handle.provider.realm_id
                    && workload_id.as_ref() == handle.provider.workload_id.as_ref()
                    && handle_id == &handle.provider.handle_id
                    && *handle_generation == handle.provider.resource_generation
            }
            _ => false,
        };
        if matches {
            Ok(())
        } else {
            Err(self.failure(
                &request.context,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::RepairConfiguration,
            ))
        }
    }

    fn sdk_context(
        context: &ProviderCallContext<'_>,
        infrastructure: InfrastructureHandle,
        deployment: DeploymentHandle,
    ) -> Result<SdkCallContext, FakeSdkErrorKind> {
        Ok(SdkCallContext::runtime(
            operation_key(context.operation.idempotency_key.as_str())?,
            infrastructure,
            deployment,
            context.monotonic_deadline_remaining_ms,
        ))
    }

    fn sdk_failure(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        error: FakeSdkError,
    ) -> ProviderFailure {
        self.sdk_failure_kind(context, error.kind())
    }

    fn sdk_failure_kind(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        error: FakeSdkErrorKind,
    ) -> ProviderFailure {
        let (kind, retry, reason, remediation) = match error {
            FakeSdkErrorKind::Cancelled => (
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
            FakeSdkErrorKind::DeadlineExpired => (
                ProviderFailureKind::DeadlineExpired,
                RetryClass::Never,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ),
            FakeSdkErrorKind::Unavailable => (
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthStale,
                ProviderRemediation::InspectProvider,
            ),
            FakeSdkErrorKind::NotFound | FakeSdkErrorKind::IdentityMismatch => (
                ProviderFailureKind::AdoptionRejected,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::InspectProvider,
            ),
            FakeSdkErrorKind::GenerationMismatch => (
                ProviderFailureKind::AdoptionRejected,
                RetryClass::Never,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ),
            FakeSdkErrorKind::AuthorityDenied => (
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            FakeSdkErrorKind::IdempotencyConflict
            | FakeSdkErrorKind::OutcomeMismatch
            | FakeSdkErrorKind::BoundExceeded
            | FakeSdkErrorKind::StateUnavailable => (
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
        };
        self.failure(context, kind, retry, reason, remediation)
    }

    fn local_sdk_failure(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        kind: FakeSdkErrorKind,
    ) -> ProviderFailure {
        self.sdk_failure_kind(context, kind)
    }

    pub async fn plan_deployment(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        infrastructure: &BoundInfrastructureHandle,
    ) -> Result<AzureVmRuntimePlan, ProviderFailure> {
        self.validate_request(context, request, ProviderMethod::RuntimePlan)?;
        self.validate_infrastructure_target(request, infrastructure)?;
        let resources =
            BoundedVec::new(vec![PlannedResourceClass::WorkloadExecution]).map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })?;
        let expires_at_unix_ms = self
            .now_unix_ms
            .saturating_add(30_000)
            .min(request.context.expires_at_unix_ms);
        let plan = ProviderPlan {
            schema_version: PROVIDER_SCHEMA_VERSION,
            plan_id: PlanId::parse("azure-vm-runtime-plan").map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })?,
            binding: request.context.binding(),
            realm_id: request.target.realm_id().clone(),
            workload_id: request.target.workload_id().cloned(),
            method: request.context.method,
            configuration_fingerprint: request.expected_configuration_fingerprint.clone(),
            created_at_unix_ms: self.now_unix_ms,
            expires_at_unix_ms,
            resources,
        };
        plan.validate(request, self.now_unix_ms)
            .map_err(|_| self.invalid_request(context.operation))?;
        let desired = DeploymentHandle::new(
            infrastructure.sdk,
            ResourceId::new(bounded_hash(request.context.request_digest.as_str()))
                .map_err(|kind| self.local_sdk_failure(context.operation, kind))?,
            ResourceGeneration::new(1)
                .map_err(|kind| self.local_sdk_failure(context.operation, kind))?,
        );
        Ok(AzureVmRuntimePlan {
            plan,
            infrastructure: infrastructure.clone(),
            desired,
        })
    }

    fn validate_ensure_plan(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &AzureVmRuntimePlan,
    ) -> Result<(), ProviderFailure> {
        let expected_desired = DeploymentHandle::new(
            plan.infrastructure.sdk,
            ResourceId::new(bounded_hash(plan.plan.binding.request_digest.as_str()))
                .map_err(|_| self.invalid_plan(context.operation))?,
            ResourceGeneration::new(1).map_err(|_| self.invalid_plan(context.operation))?,
        );
        let valid = plan.plan.schema_version == d2b_contracts::v2_provider::PROVIDER_SCHEMA_VERSION
            && plan.plan.binding == context.operation.binding()
            && plan.plan.realm_id == *context.operation.scope.realm_id()
            && plan.plan.workload_id.as_ref() == context.operation.scope.workload_id()
            && plan.plan.workload_id.is_some()
            && plan.plan.method == ProviderMethod::RuntimePlan
            && plan.plan.resources.as_slice() == [PlannedResourceClass::WorkloadExecution]
            && plan.plan.configuration_fingerprint == self.descriptor.configuration_fingerprint
            && plan.plan.created_at_unix_ms <= self.now_unix_ms
            && plan.plan.created_at_unix_ms < plan.plan.expires_at_unix_ms
            && plan.plan.expires_at_unix_ms > self.now_unix_ms
            && plan.plan.expires_at_unix_ms <= context.operation.expires_at_unix_ms
            && plan.infrastructure.validate().is_ok()
            && plan.infrastructure.provider.realm_id == plan.plan.realm_id
            && plan
                .infrastructure
                .provider
                .expires_at_unix_ms
                .is_none_or(|expiry| expiry > self.now_unix_ms)
            && plan.desired == expected_desired;
        if valid {
            Ok(())
        } else {
            Err(self.invalid_plan(context.operation))
        }
    }

    fn invalid_plan(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn handle_from_plan(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        plan: &ProviderPlan,
        handle_id: HandleId,
        resource_generation: Generation,
    ) -> Result<ProviderHandle, ProviderFailure> {
        let handle = ProviderHandle {
            schema_version: PROVIDER_SCHEMA_VERSION,
            handle_id,
            kind: ProviderHandleKind::Runtime,
            provider_id: self.descriptor.provider_id.clone(),
            realm_id: plan.realm_id.clone(),
            workload_id: plan.workload_id.clone(),
            owner: HandleOwner::Provider {
                realm_id: plan.realm_id.clone(),
                provider_id: self.descriptor.provider_id.clone(),
            },
            provider_generation: self.descriptor.registry_generation,
            resource_generation,
            configuration_fingerprint: self.descriptor.configuration_fingerprint.clone(),
            created_by: plan.binding.clone(),
            created_at_unix_ms: self.now_unix_ms,
            expires_at_unix_ms: None,
            ownership_transfer: OwnershipTransfer::Stationary {
                ownership_epoch: Generation::new(1).map_err(|_| self.invalid_plan(context))?,
            },
        };
        handle.validate().map_err(|_| self.invalid_plan(context))?;
        Ok(handle)
    }

    pub async fn deploy(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &AzureVmRuntimePlan,
    ) -> Result<AzureVmRuntimeHandle, ProviderFailure> {
        self.validate_call(context, ProviderMethod::RuntimeEnsure)?;
        self.validate_ensure_plan(context, plan)?;
        let sdk_context = Self::sdk_context(context, plan.infrastructure.sdk, plan.desired)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let mutation = self
            .sdk
            .deploy_runtime(&sdk_context, plan.desired)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        let provider_handle = self.handle_from_plan(
            context.operation,
            &plan.plan,
            handle_id("azure-vm-runtime", mutation.handle().identity().get())
                .map_err(|kind| self.local_sdk_failure(context.operation, kind))?,
            Generation::new(mutation.handle().generation().get()).map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::GenerationMismatch,
                    ProviderRemediation::ReplaceGeneration,
                )
            })?,
        )?;
        Ok(AzureVmRuntimeHandle {
            provider: provider_handle,
            infrastructure: plan.infrastructure.clone(),
            sdk: mutation.handle(),
        })
    }

    pub async fn start_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        handle: &AzureVmRuntimeHandle,
    ) -> Result<ProviderObservation, ProviderFailure> {
        self.validate_request(context, request, ProviderMethod::RuntimeStart)?;
        self.validate_runtime_request(request, handle)?;
        let sdk_context = Self::sdk_context(context, handle.infrastructure.sdk, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let observation = self
            .sdk
            .start_runtime(&sdk_context, handle.sdk)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        self.observation(
            context.operation,
            handle,
            deployment_lifecycle(observation.state()),
            AdoptionState::NotAttempted,
        )
    }

    pub async fn stop_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        handle: &AzureVmRuntimeHandle,
    ) -> Result<ProviderObservation, ProviderFailure> {
        self.validate_request(context, request, ProviderMethod::RuntimeStop)?;
        self.validate_runtime_request(request, handle)?;
        let sdk_context = Self::sdk_context(context, handle.infrastructure.sdk, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let observation = self
            .sdk
            .stop_runtime(&sdk_context, handle.sdk)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        self.observation(
            context.operation,
            handle,
            deployment_lifecycle(observation.state()),
            AdoptionState::NotAttempted,
        )
    }

    pub async fn inspect_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        handle: &AzureVmRuntimeHandle,
    ) -> Result<ProviderObservation, ProviderFailure> {
        self.validate_request(context, request, ProviderMethod::RuntimeInspect)?;
        self.validate_runtime_request(request, handle)?;
        let sdk_context = Self::sdk_context(context, handle.infrastructure.sdk, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let observation = self
            .sdk
            .inspect_runtime(&sdk_context, handle.sdk)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        self.observation(
            context.operation,
            handle,
            deployment_lifecycle(observation.state()),
            AdoptionState::NotAttempted,
        )
    }

    pub async fn adopt_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
        handle: &AzureVmRuntimeHandle,
    ) -> Result<ProviderObservation, ProviderFailure> {
        self.validate_call(context, ProviderMethod::RuntimeAdopt)?;
        self.validate_runtime_handle(context.operation, handle)?;
        if context.operation != &request.context
            || request.handle != handle.provider
            || request.handle.validate().is_err()
            || request.handle.provider_id != self.descriptor.provider_id
            || request.handle.provider_generation != self.descriptor.registry_generation
            || request.handle.realm_id != *context.operation.scope.realm_id()
            || request.handle.workload_id.as_ref() != context.operation.scope.workload_id()
            || request.handle.owner != request.expected_owner
            || request.handle.configuration_fingerprint
                != request.expected_configuration_fingerprint
            || request.handle.resource_generation != request.expected_resource_generation
        {
            let reason =
                if request.expected_resource_generation != handle.provider.resource_generation {
                    ProviderHealthReason::GenerationMismatch
                } else {
                    ProviderHealthReason::IdentityMismatch
                };
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::AdoptionRejected,
                RetryClass::Never,
                reason,
                ProviderRemediation::InspectProvider,
            ));
        }
        let sdk_context = Self::sdk_context(context, handle.infrastructure.sdk, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let observation = self
            .sdk
            .adopt_runtime(&sdk_context, handle.sdk)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        self.observation(
            context.operation,
            handle,
            deployment_lifecycle(observation.state()),
            AdoptionState::Adopted,
        )
    }

    pub async fn remove_deployment_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        handle: &AzureVmRuntimeHandle,
    ) -> Result<MutationReceipt, ProviderFailure> {
        self.validate_request(context, request, ProviderMethod::RuntimeDestroy)?;
        self.validate_runtime_request(request, handle)?;
        let sdk_context = Self::sdk_context(context, handle.infrastructure.sdk, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let result = self
            .sdk
            .remove_runtime_deployment(&sdk_context, handle.sdk)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        let receipt = MutationReceipt {
            binding: context.operation.binding(),
            state: match result.disposition() {
                ApplyDisposition::Applied => MutationState::Applied,
                ApplyDisposition::AlreadyApplied => MutationState::AlreadyApplied,
            },
            observed_at_unix_ms: self.now_unix_ms,
            observation_required_before_retry: false,
        };
        receipt.validate().map_err(|_| {
            self.failure(
                context.operation,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            )
        })?;
        Ok(receipt)
    }

    fn observation(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        handle: &AzureVmRuntimeHandle,
        lifecycle: ObservedLifecycleState,
        adoption: AdoptionState,
    ) -> Result<ProviderObservation, ProviderFailure> {
        let health = ProviderHealth {
            provider_id: self.descriptor.provider_id.clone(),
            registry_generation: self.descriptor.registry_generation,
            observed_at_unix_ms: self.now_unix_ms,
            state: ProviderHealthState::Healthy,
            reason: ProviderHealthReason::None,
            remediation: ProviderRemediation::None,
        };
        let observation = ProviderObservation {
            provider_id: self.descriptor.provider_id.clone(),
            provider_generation: self.descriptor.registry_generation,
            realm_id: context.scope.realm_id().clone(),
            workload_id: context.scope.workload_id().cloned(),
            handle_id: Some(handle.provider.handle_id.clone()),
            resource_generation: Some(handle.provider.resource_generation),
            observed_at_unix_ms: self.now_unix_ms,
            lifecycle,
            adoption,
            reason: ObservationReason::None,
            health,
        };
        observation.validate().map_err(|_| {
            self.failure(
                context,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            )
        })?;
        Ok(observation)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BoundInfrastructureHandle {
    provider: ProviderHandle,
    sdk: InfrastructureHandle,
    binding: InfrastructureBindingFingerprint,
}

impl BoundInfrastructureHandle {
    pub fn new(
        provider: ProviderHandle,
        sdk: InfrastructureHandle,
        binding: InfrastructureBindingFingerprint,
    ) -> Result<Self, InfrastructureBindingError> {
        let value = Self {
            provider,
            sdk,
            binding,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn provider_handle(&self) -> &ProviderHandle {
        &self.provider
    }

    pub const fn sdk_handle(&self) -> InfrastructureHandle {
        self.sdk
    }

    pub const fn binding_fingerprint(&self) -> InfrastructureBindingFingerprint {
        self.binding
    }

    fn validate(&self) -> Result<(), InfrastructureBindingError> {
        self.provider
            .validate()
            .map_err(|_| InfrastructureBindingError::InvalidHandle)?;
        if self.provider.kind != ProviderHandleKind::Infrastructure
            || self.provider.workload_id.is_some()
        {
            return Err(InfrastructureBindingError::WrongHandleKind);
        }
        if self.provider.resource_generation.get() != self.sdk.generation().get() {
            return Err(InfrastructureBindingError::GenerationMismatch);
        }
        if !matches!(
            &self.provider.owner,
            HandleOwner::Provider {
                realm_id,
                provider_id,
            } if realm_id == &self.provider.realm_id && provider_id == &self.provider.provider_id
        ) {
            return Err(InfrastructureBindingError::BindingMismatch);
        }
        let material = infrastructure_binding_material(&self.provider)
            .map_err(|_| InfrastructureBindingError::InvalidHandle)?;
        if !self.binding.verifies(&material, self.sdk) {
            return Err(InfrastructureBindingError::BindingMismatch);
        }
        Ok(())
    }
}

impl fmt::Debug for BoundInfrastructureHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundInfrastructureHandle")
            .field("provider_handle", &self.provider)
            .field("sdk_handle", &"<opaque>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmRuntimePlan {
    plan: ProviderPlan,
    infrastructure: BoundInfrastructureHandle,
    desired: DeploymentHandle,
}

impl AzureVmRuntimePlan {
    pub fn provider_plan(&self) -> &ProviderPlan {
        &self.plan
    }

    pub fn infrastructure(&self) -> &BoundInfrastructureHandle {
        &self.infrastructure
    }
}

impl fmt::Debug for AzureVmRuntimePlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmRuntimePlan")
            .field("provider_plan", &self.plan)
            .field("infrastructure", &"<opaque>")
            .field("desired", &"<opaque>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmRuntimeHandle {
    provider: ProviderHandle,
    infrastructure: BoundInfrastructureHandle,
    sdk: DeploymentHandle,
}

impl AzureVmRuntimeHandle {
    pub fn provider_handle(&self) -> &ProviderHandle {
        &self.provider
    }

    pub fn infrastructure(&self) -> &BoundInfrastructureHandle {
        &self.infrastructure
    }

    pub const fn sdk_handle(&self) -> DeploymentHandle {
        self.sdk
    }
}

impl fmt::Debug for AzureVmRuntimeHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmRuntimeHandle")
            .field("provider_handle", &self.provider)
            .field("infrastructure", &"<opaque>")
            .field("sdk_handle", &"<opaque>")
            .finish()
    }
}

fn runtime_input_matches(request: &ProviderOperationRequest, method: ProviderMethod) -> bool {
    matches!(
        (method, &request.input),
        (
            ProviderMethod::RuntimePlan
                | ProviderMethod::RuntimeStart
                | ProviderMethod::RuntimeStop
                | ProviderMethod::RuntimeInspect
                | ProviderMethod::RuntimeDestroy,
            ProviderOperationInput::NoInput
        )
    )
}

fn deployment_lifecycle(state: DeploymentState) -> ObservedLifecycleState {
    match state {
        DeploymentState::Running => ObservedLifecycleState::Running,
        DeploymentState::Stopped => ObservedLifecycleState::Stopped,
    }
}

fn bounded_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash % (d2b_contracts::v2_provider::MAX_SAFE_JSON_INTEGER - 1)) + 1
}

fn operation_key(value: &str) -> Result<d2b_azure_vm_fake_sdk::OperationKey, FakeSdkErrorKind> {
    d2b_azure_vm_fake_sdk::OperationKey::new(bounded_hash(value))
}

fn handle_id(prefix: &str, identity: u64) -> Result<HandleId, FakeSdkErrorKind> {
    HandleId::parse(format!("{prefix}-{identity:x}")).map_err(|_| FakeSdkErrorKind::BoundExceeded)
}

fn infrastructure_binding_material(
    handle: &ProviderHandle,
) -> Result<InfrastructureBindingMaterial<'_>, BindingMaterialError> {
    InfrastructureBindingMaterial::new(
        handle.schema_version,
        handle.provider_id.as_str(),
        handle.handle_id.as_str(),
        handle.realm_id.as_str(),
        handle.provider_generation.get(),
        handle.resource_generation.get(),
        handle.configuration_fingerprint.as_str(),
    )
}

#[cfg(test)]
mod tests;
