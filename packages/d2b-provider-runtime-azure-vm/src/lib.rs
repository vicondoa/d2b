//! Explicitly non-production Azure VM workload runtime provider scaffold.
//!
//! Runtime operations require an already-bound opaque infrastructure handle.
//! Canonical dispatch and production registration remain unavailable; explicit
//! conformance APIs exercise workload-only behavior through the fake SDK.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{error::Error, fmt};

use std::sync::Arc;

use d2b_contracts::{
    v2_identity::ProviderType,
    v2_provider::{
        AdoptionRequest, MutationReceipt, Provider, ProviderCallContext, ProviderCapabilitySet,
        ProviderDescriptor, ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle,
        ProviderHealth, ProviderHealthReason, ProviderHealthState, ProviderMethod,
        ProviderObservation, ProviderOperationRequest, ProviderPlan, ProviderRemediation,
        RetryClass, RuntimeProvider,
    },
};
use d2b_provider::ProviderInstance;
use {
    d2b_azure_vm_fake_sdk::{
        ApplyDisposition, DeploymentHandle, DeploymentState, FakeAzureVmSdk, FakeSdkError,
        FakeSdkErrorKind, InfrastructureHandle, ResourceGeneration, ResourceId, SdkCallContext,
    },
    d2b_contracts::{
        v2_component_session::BoundedVec,
        v2_provider::{
            AdoptionState, Generation, HandleId, MutationState, ObservationReason,
            ObservedLifecycleState, PlannedResourceClass, ProviderCapability, ProviderHandleKind,
            ProviderOperationInput,
        },
    },
    d2b_provider_toolkit::ProviderValues,
};

const IMPLEMENTATION_ID: &str = "azure-vm";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldUnavailable {
    CapabilityUnavailable,
}

impl fmt::Display for ScaffoldUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Azure VM runtime capability is unavailable")
    }
}

impl Error for ScaffoldUnavailable {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldConstructionError {
    InvalidDescriptor,
    WrongAuthority,
    WrongImplementation,
    CapabilityInventoryMismatch,
    InvalidClock,
}

impl fmt::Display for ScaffoldConstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor => "Azure VM runtime descriptor is invalid",
            Self::WrongAuthority => "Azure VM runtime descriptor has the wrong authority",
            Self::WrongImplementation => "Azure VM runtime descriptor has the wrong implementation",
            Self::CapabilityInventoryMismatch => {
                "Azure VM runtime descriptor has the wrong method inventory"
            }
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
}

impl fmt::Display for InfrastructureBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidHandle => "opaque infrastructure handle is invalid",
            Self::WrongHandleKind => "opaque infrastructure handle has the wrong authority",
            Self::GenerationMismatch => "opaque infrastructure handle generation mismatch",
        })
    }
}

impl Error for InfrastructureBindingError {}

pub struct AzureVmRuntimeProvider {
    descriptor: ProviderDescriptor,
    now_unix_ms: u64,
    sdk: Arc<FakeAzureVmSdk>,
}

impl fmt::Debug for AzureVmRuntimeProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmRuntimeProvider")
            .field("production_available", &Self::PRODUCTION_AVAILABLE)
            .field("provider_type", &ProviderType::Runtime)
            .finish_non_exhaustive()
    }
}

impl AzureVmRuntimeProvider {
    pub const PRODUCTION_AVAILABLE: bool = false;
    pub const LIVE_PRODUCTION_CAPABILITIES: &'static [ProviderMethod] = &[];
    pub const CONTRACT_METHODS: &'static [ProviderMethod] = &[
        ProviderMethod::RuntimePlan,
        ProviderMethod::RuntimeEnsure,
        ProviderMethod::RuntimeStart,
        ProviderMethod::RuntimeStop,
        ProviderMethod::RuntimeInspect,
        ProviderMethod::RuntimeAdopt,
        ProviderMethod::RuntimeDestroy,
    ];

    pub fn production_registration() -> Result<ProviderInstance, ScaffoldUnavailable> {
        Err(ScaffoldUnavailable::CapabilityUnavailable)
    }

    pub fn new_for_conformance(
        descriptor: ProviderDescriptor,
        sdk: Arc<FakeAzureVmSdk>,
        now_unix_ms: u64,
    ) -> Result<Self, ScaffoldConstructionError> {
        descriptor
            .validate()
            .map_err(|_| ScaffoldConstructionError::InvalidDescriptor)?;
        if descriptor.provider_type() != ProviderType::Runtime {
            return Err(ScaffoldConstructionError::WrongAuthority);
        }
        if descriptor.implementation_id.as_str() != IMPLEMENTATION_ID {
            return Err(ScaffoldConstructionError::WrongImplementation);
        }
        if descriptor.capabilities != contract_capabilities() {
            return Err(ScaffoldConstructionError::CapabilityInventoryMismatch);
        }
        if !(1_001..=d2b_contracts::v2_provider::MAX_SAFE_JSON_INTEGER).contains(&now_unix_ms) {
            return Err(ScaffoldConstructionError::InvalidClock);
        }
        Ok(Self {
            descriptor,
            now_unix_ms,
            sdk,
        })
    }

    pub fn conformance_instance(self: Arc<Self>) -> ProviderInstance {
        ProviderInstance::Runtime(self)
    }

    fn capability_unavailable(
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

    fn values(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> Result<ProviderValues, ProviderFailure> {
        ProviderValues::new(&self.descriptor, self.now_unix_ms).map_err(|_| {
            self.failure(
                context,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            )
        })
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
            return Err(self.capability_unavailable(context.operation));
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
        context
            .operation
            .validate(&self.descriptor, self.now_unix_ms)
            .map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvalidRequest,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
    ) -> Result<(), ProviderFailure> {
        self.validate_call(context, expected)?;
        if context.operation != &request.context || !runtime_input_matches(request, expected) {
            return Err(self.capability_unavailable(context.operation));
        }
        request
            .validate_method(&self.descriptor, self.now_unix_ms, expected)
            .map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvalidRequest,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })
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
                == self.descriptor.configuration_schema_fingerprint
            && handle.provider.workload_id.is_some()
            && handle.provider.resource_generation.get() == handle.sdk.generation().get()
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
        let values = self.values(context.operation)?;
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
        let plan = values
            .plan(
                request,
                d2b_contracts::v2_provider::PlanId::parse("azure-vm-runtime-plan").map_err(
                    |_| {
                        self.failure(
                            context.operation,
                            ProviderFailureKind::InvariantViolation,
                            RetryClass::Never,
                            ProviderHealthReason::ConfigurationMismatch,
                            ProviderRemediation::RepairConfiguration,
                        )
                    },
                )?,
                expires_at_unix_ms,
                resources,
            )
            .map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvalidRequest,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })?;
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

    pub async fn deploy(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &AzureVmRuntimePlan,
    ) -> Result<AzureVmRuntimeHandle, ProviderFailure> {
        self.validate_call(context, ProviderMethod::RuntimeEnsure)?;
        if plan.plan.method != ProviderMethod::RuntimePlan
            || plan.plan.binding.provider_id != self.descriptor.provider_id
            || plan.plan.binding.provider_generation != self.descriptor.registry_generation
            || plan.plan.configuration_fingerprint
                != self.descriptor.configuration_schema_fingerprint
            || plan.plan.expires_at_unix_ms <= self.now_unix_ms
            || plan.infrastructure.validate().is_err()
            || plan.desired.infrastructure() != plan.infrastructure.sdk
        {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        let sdk_context = Self::sdk_context(context, plan.infrastructure.sdk, plan.desired)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let mutation = self
            .sdk
            .deploy_runtime(&sdk_context, plan.desired)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        let values = self.values(context.operation)?;
        let provider_handle = values
            .handle_from_plan(
                &plan.plan,
                handle_id("azure-vm-runtime", mutation.handle().identity().get())
                    .map_err(|kind| self.local_sdk_failure(context.operation, kind))?,
                values.provider_owner(&plan.plan.realm_id),
                Generation::new(mutation.handle().generation().get()).map_err(|_| {
                    self.failure(
                        context.operation,
                        ProviderFailureKind::InvariantViolation,
                        RetryClass::Never,
                        ProviderHealthReason::GenerationMismatch,
                        ProviderRemediation::ReplaceGeneration,
                    )
                })?,
                None,
            )
            .map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })?;
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
            || request
                .validate(&self.descriptor, self.now_unix_ms)
                .is_err()
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
        self.values(context.operation)?
            .receipt(
                context.operation,
                match result.disposition() {
                    ApplyDisposition::Applied => MutationState::Applied,
                    ApplyDisposition::AlreadyApplied => MutationState::AlreadyApplied,
                },
            )
            .map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })
    }

    fn observation(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        handle: &AzureVmRuntimeHandle,
        lifecycle: ObservedLifecycleState,
        adoption: AdoptionState,
    ) -> Result<ProviderObservation, ProviderFailure> {
        self.values(context)?
            .observation(
                context,
                Some(&handle.provider),
                lifecycle,
                adoption,
                ObservationReason::None,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            )
            .map_err(|_| {
                self.failure(
                    context,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })
    }
}

impl Provider for AzureVmRuntimeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            self.values(context.operation)?
                .health(
                    ProviderHealthState::Unavailable,
                    ProviderHealthReason::HealthStale,
                    ProviderRemediation::InspectProvider,
                )
                .map_err(|_| self.capability_unavailable(context.operation))
        })
    }
}

macro_rules! denied_dispatch {
    ($name:ident, $request:ty, $result:ty) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            _request: &'a $request,
        ) -> ProviderFuture<'a, $result> {
            Box::pin(async move { Err(self.capability_unavailable(context.operation)) })
        }
    };
}

impl RuntimeProvider for AzureVmRuntimeProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    denied_dispatch!(plan, ProviderOperationRequest, ProviderPlan);
    denied_dispatch!(ensure, ProviderPlan, ProviderHandle);
    denied_dispatch!(start, ProviderOperationRequest, ProviderObservation);
    denied_dispatch!(stop, ProviderOperationRequest, ProviderObservation);
    denied_dispatch!(inspect, ProviderOperationRequest, ProviderObservation);
    denied_dispatch!(adopt, AdoptionRequest, ProviderObservation);
    denied_dispatch!(destroy, ProviderOperationRequest, MutationReceipt);
}

#[derive(Clone, PartialEq, Eq)]
pub struct BoundInfrastructureHandle {
    provider: ProviderHandle,
    sdk: InfrastructureHandle,
}

impl BoundInfrastructureHandle {
    pub fn new(
        provider: ProviderHandle,
        sdk: InfrastructureHandle,
    ) -> Result<Self, InfrastructureBindingError> {
        let value = Self { provider, sdk };
        value.validate()?;
        Ok(value)
    }

    pub fn provider_handle(&self) -> &ProviderHandle {
        &self.provider
    }

    pub const fn sdk_handle(&self) -> InfrastructureHandle {
        self.sdk
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

fn contract_capabilities() -> ProviderCapabilitySet {
    ProviderCapabilitySet::new(
        AzureVmRuntimeProvider::CONTRACT_METHODS
            .iter()
            .copied()
            .map(ProviderCapability)
            .collect(),
    )
    .unwrap_or_else(|_| unreachable!())
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

#[cfg(test)]
mod tests;
