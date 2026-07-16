//! Explicitly non-production Azure VM infrastructure provider scaffold.
//!
//! The canonical provider surface always denies lifecycle dispatch. Explicit
//! conformance construction and direct methods use only the in-process fake
//! SDK; production registration remains typed unavailable.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{error::Error, fmt};

use std::sync::Arc;

use d2b_contracts::{
    v2_component_session::BoundedVec,
    v2_identity::{ProviderId, ProviderType, RealmId},
    v2_provider::{
        AdoptionRequest, Fingerprint, Generation, HandleId, HandleOwner,
        MAX_PROVIDER_REQUEST_LIFETIME_MS, MAX_SAFE_JSON_INTEGER, MutationReceipt, MutationState,
        ObservationReason, ObservedLifecycleState, OwnershipTransfer, PROVIDER_SCHEMA_VERSION,
        PlanId, PlannedResourceClass, ProviderCallContext, ProviderFailure, ProviderFailureKind,
        ProviderHandle, ProviderHandleKind, ProviderHealth, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderObservation, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlan, ProviderRemediation, RetryClass,
    },
};
use {
    d2b_azure_vm_fake_sdk::{
        ApplyDisposition, BindingMaterialError, BootstrapBinding, FakeAzureVmSdk, FakeSdkError,
        FakeSdkErrorKind, InfrastructureBindingFingerprint, InfrastructureBindingMaterial,
        InfrastructureHandle, PowerState, ResourceGeneration, ResourceId, SdkCallContext,
    },
    d2b_contracts::v2_provider::{AdoptionState, InfrastructurePowerState},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldAvailability {
    Unavailable,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmInfrastructureScaffoldDescriptor {
    provider_id: ProviderId,
    registry_generation: Generation,
    configuration_fingerprint: Fingerprint,
    realm_id: RealmId,
}

impl AzureVmInfrastructureScaffoldDescriptor {
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

impl fmt::Debug for AzureVmInfrastructureScaffoldDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmInfrastructureScaffoldDescriptor")
            .field("availability", &self.availability())
            .field("provider_type", &ProviderType::Infrastructure)
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
            Self::InvalidClock => "Azure VM infrastructure scaffold clock is invalid",
        })
    }
}

impl Error for ScaffoldConstructionError {}

pub struct AzureVmInfrastructureScaffold {
    descriptor: AzureVmInfrastructureScaffoldDescriptor,
    now_unix_ms: u64,
    sdk: Arc<FakeAzureVmSdk>,
}

impl fmt::Debug for AzureVmInfrastructureScaffold {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmInfrastructureScaffold")
            .field("availability", &self.descriptor.availability())
            .field("provider_type", &ProviderType::Infrastructure)
            .finish_non_exhaustive()
    }
}

impl AzureVmInfrastructureScaffold {
    #[cfg(test)]
    const DIRECT_METHODS: &'static [ProviderMethod] = &[
        ProviderMethod::InfrastructurePlan,
        ProviderMethod::InfrastructureApply,
        ProviderMethod::InfrastructureSetPowerState,
        ProviderMethod::InfrastructureInspect,
        ProviderMethod::InfrastructureAdopt,
        ProviderMethod::InfrastructureBootstrapBinding,
        ProviderMethod::InfrastructureDestroy,
    ];

    pub fn new_for_conformance(
        descriptor: AzureVmInfrastructureScaffoldDescriptor,
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

    pub fn descriptor(&self) -> &AzureVmInfrastructureScaffoldDescriptor {
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
            provider_type: ProviderType::Infrastructure,
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
            && operation.provider_type == ProviderType::Infrastructure
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
        if expected.provider_type() != ProviderType::Infrastructure
            || context.operation.method.provider_type() != ProviderType::Infrastructure
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
        if context.operation != &request.context || !infrastructure_input_matches(request, expected)
        {
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

    fn validate_handle_request(
        &self,
        request: &ProviderOperationRequest,
        handle: &AzureVmInfrastructureHandle,
    ) -> Result<(), ProviderFailure> {
        self.validate_bound_handle(&request.context, handle)?;
        let matches = match &request.target {
            d2b_contracts::v2_provider::ProviderTarget::Handle {
                realm_id,
                workload_id,
                handle_id,
                handle_generation,
            } => {
                realm_id == &handle.provider.realm_id
                    && workload_id.is_none()
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

    fn validate_bound_handle(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        handle: &AzureVmInfrastructureHandle,
    ) -> Result<(), ProviderFailure> {
        let valid = handle.provider.validate().is_ok()
            && handle.provider.kind == ProviderHandleKind::Infrastructure
            && handle.provider.provider_id == self.descriptor.provider_id
            && handle.provider.provider_generation == self.descriptor.registry_generation
            && handle.provider.configuration_fingerprint
                == self.descriptor.configuration_fingerprint
            && handle.provider.workload_id.is_none()
            && matches!(
                &handle.provider.owner,
                HandleOwner::Provider {
                    realm_id,
                    provider_id,
                } if realm_id == &handle.provider.realm_id
                    && provider_id == &handle.provider.provider_id
            )
            && handle.provider.resource_generation.get() == handle.sdk.generation().get()
            && handle.binding.verifies(
                &infrastructure_binding_material(&handle.provider)
                    .map_err(|_| self.invalid_bound_handle(context))?,
                handle.sdk,
            );
        if valid {
            Ok(())
        } else {
            Err(self.invalid_bound_handle(context))
        }
    }

    fn invalid_bound_handle(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::AdoptionRejected,
            RetryClass::Never,
            ProviderHealthReason::IdentityMismatch,
            ProviderRemediation::InspectProvider,
        )
    }

    fn sdk_context(
        context: &ProviderCallContext<'_>,
        handle: InfrastructureHandle,
    ) -> Result<SdkCallContext, FakeSdkErrorKind> {
        Ok(SdkCallContext::infrastructure(
            operation_key(context.operation.idempotency_key.as_str())?,
            handle,
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

    pub async fn plan_create(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> Result<AzureVmInfrastructurePlan, ProviderFailure> {
        self.validate_request(context, request, ProviderMethod::InfrastructurePlan)?;
        let resources =
            BoundedVec::new(vec![PlannedResourceClass::Infrastructure]).map_err(|_| {
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
            plan_id: PlanId::parse("azure-vm-infrastructure-plan").map_err(|_| {
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
        let desired = InfrastructureHandle::new(
            resource_id(request.context.request_digest.as_str())
                .map_err(|kind| self.local_sdk_failure(context.operation, kind))?,
            ResourceGeneration::new(1).map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })?,
        );
        Ok(AzureVmInfrastructurePlan { plan, desired })
    }

    fn validate_apply_plan(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &AzureVmInfrastructurePlan,
    ) -> Result<(), ProviderFailure> {
        let expected_desired = InfrastructureHandle::new(
            resource_id(plan.plan.binding.request_digest.as_str())
                .map_err(|_| self.invalid_plan(context.operation))?,
            ResourceGeneration::new(1).map_err(|_| self.invalid_plan(context.operation))?,
        );
        let valid = plan.plan.schema_version == d2b_contracts::v2_provider::PROVIDER_SCHEMA_VERSION
            && plan.plan.binding == context.operation.binding()
            && plan.plan.realm_id == *context.operation.scope.realm_id()
            && plan.plan.workload_id.is_none()
            && context.operation.scope.workload_id().is_none()
            && plan.plan.method == ProviderMethod::InfrastructurePlan
            && plan.plan.resources.as_slice() == [PlannedResourceClass::Infrastructure]
            && plan.plan.configuration_fingerprint == self.descriptor.configuration_fingerprint
            && plan.plan.created_at_unix_ms <= self.now_unix_ms
            && plan.plan.created_at_unix_ms < plan.plan.expires_at_unix_ms
            && plan.plan.expires_at_unix_ms > self.now_unix_ms
            && plan.plan.expires_at_unix_ms <= context.operation.expires_at_unix_ms
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
            kind: ProviderHandleKind::Infrastructure,
            provider_id: self.descriptor.provider_id.clone(),
            realm_id: plan.realm_id.clone(),
            workload_id: None,
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

    pub async fn create(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &AzureVmInfrastructurePlan,
    ) -> Result<AzureVmInfrastructureHandle, ProviderFailure> {
        self.validate_call(context, ProviderMethod::InfrastructureApply)?;
        self.validate_apply_plan(context, plan)?;
        let sdk_context = Self::sdk_context(context, plan.desired)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let mutation = self
            .sdk
            .create_infrastructure(&sdk_context, plan.desired, PowerState::Stopped)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        let provider_handle = self.handle_from_plan(
            context.operation,
            &plan.plan,
            handle_id(
                "azure-vm-infrastructure",
                mutation.handle().identity().get(),
            )
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
        let sdk_handle = mutation.handle();
        let binding = InfrastructureBindingFingerprint::compute(
            &infrastructure_binding_material(&provider_handle).map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })?,
            sdk_handle,
        );
        Ok(AzureVmInfrastructureHandle {
            provider: provider_handle,
            sdk: sdk_handle,
            binding,
        })
    }

    pub async fn set_power_state_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        handle: &AzureVmInfrastructureHandle,
    ) -> Result<ProviderObservation, ProviderFailure> {
        self.validate_request(
            context,
            request,
            ProviderMethod::InfrastructureSetPowerState,
        )?;
        self.validate_handle_request(request, handle)?;
        let ProviderOperationInput::InfrastructurePowerState { state } = &request.input else {
            return Err(self.deny_unavailable_dispatch(context.operation));
        };
        let power = match state {
            InfrastructurePowerState::Running => PowerState::Running,
            InfrastructurePowerState::Stopped => PowerState::Stopped,
        };
        let sdk_context = Self::sdk_context(context, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let observation = self
            .sdk
            .set_power_state(&sdk_context, handle.sdk, power)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        self.observation(
            context.operation,
            handle,
            power_lifecycle(observation.power_state()),
            AdoptionState::NotAttempted,
        )
    }

    pub async fn inspect_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        handle: &AzureVmInfrastructureHandle,
    ) -> Result<ProviderObservation, ProviderFailure> {
        self.validate_request(context, request, ProviderMethod::InfrastructureInspect)?;
        self.validate_handle_request(request, handle)?;
        let sdk_context = Self::sdk_context(context, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let observation = self
            .sdk
            .inspect_infrastructure(&sdk_context, handle.sdk)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        self.observation(
            context.operation,
            handle,
            power_lifecycle(observation.power_state()),
            AdoptionState::NotAttempted,
        )
    }

    pub async fn adopt_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
        handle: &AzureVmInfrastructureHandle,
    ) -> Result<ProviderObservation, ProviderFailure> {
        self.validate_call(context, ProviderMethod::InfrastructureAdopt)?;
        self.validate_bound_handle(context.operation, handle)?;
        if context.operation != &request.context
            || request.handle != handle.provider
            || request.handle.validate().is_err()
            || request.handle.provider_id != self.descriptor.provider_id
            || request.handle.provider_generation != self.descriptor.registry_generation
            || request.handle.realm_id != *context.operation.scope.realm_id()
            || context.operation.scope.workload_id().is_some()
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
        let sdk_context = Self::sdk_context(context, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let observation = self
            .sdk
            .adopt_infrastructure(&sdk_context, handle.sdk)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        self.observation(
            context.operation,
            handle,
            power_lifecycle(observation.power_state()),
            AdoptionState::Adopted,
        )
    }

    pub async fn bootstrap_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        handle: &AzureVmInfrastructureHandle,
    ) -> Result<AzureVmBootstrapBinding, ProviderFailure> {
        self.validate_request(
            context,
            request,
            ProviderMethod::InfrastructureBootstrapBinding,
        )?;
        self.validate_handle_request(request, handle)?;
        let sdk_context = Self::sdk_context(context, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let binding = self
            .sdk
            .bootstrap_binding(&sdk_context, handle.sdk)
            .await
            .map_err(|error| self.sdk_failure(context.operation, error))?;
        Ok(AzureVmBootstrapBinding {
            infrastructure: handle.clone(),
            binding,
        })
    }

    pub async fn delete_direct(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        handle: &AzureVmInfrastructureHandle,
    ) -> Result<MutationReceipt, ProviderFailure> {
        self.validate_request(context, request, ProviderMethod::InfrastructureDestroy)?;
        self.validate_handle_request(request, handle)?;
        let sdk_context = Self::sdk_context(context, handle.sdk)
            .map_err(|kind| self.local_sdk_failure(context.operation, kind))?;
        let result = self
            .sdk
            .delete_infrastructure(&sdk_context, handle.sdk)
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
        handle: &AzureVmInfrastructureHandle,
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
            workload_id: None,
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

    fn local_sdk_failure(
        &self,
        context: &d2b_contracts::v2_provider::ProviderOperationContext,
        kind: FakeSdkErrorKind,
    ) -> ProviderFailure {
        self.sdk_failure_kind(context, kind)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmInfrastructurePlan {
    plan: ProviderPlan,
    desired: InfrastructureHandle,
}

impl AzureVmInfrastructurePlan {
    pub fn provider_plan(&self) -> &ProviderPlan {
        &self.plan
    }
}

impl fmt::Debug for AzureVmInfrastructurePlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmInfrastructurePlan")
            .field("provider_plan", &self.plan)
            .field("desired", &"<opaque>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmInfrastructureHandle {
    provider: ProviderHandle,
    sdk: InfrastructureHandle,
    binding: InfrastructureBindingFingerprint,
}

impl AzureVmInfrastructureHandle {
    pub fn provider_handle(&self) -> &ProviderHandle {
        &self.provider
    }

    pub const fn sdk_handle(&self) -> InfrastructureHandle {
        self.sdk
    }

    pub const fn binding_fingerprint(&self) -> InfrastructureBindingFingerprint {
        self.binding
    }
}

impl fmt::Debug for AzureVmInfrastructureHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmInfrastructureHandle")
            .field("provider_handle", &self.provider)
            .field("sdk_handle", &"<opaque>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmBootstrapBinding {
    infrastructure: AzureVmInfrastructureHandle,
    binding: BootstrapBinding,
}

impl AzureVmBootstrapBinding {
    pub fn infrastructure(&self) -> &AzureVmInfrastructureHandle {
        &self.infrastructure
    }

    pub const fn binding(&self) -> BootstrapBinding {
        self.binding
    }
}

impl fmt::Debug for AzureVmBootstrapBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmBootstrapBinding")
            .field("infrastructure", &"<opaque>")
            .field("binding", &self.binding)
            .finish()
    }
}

fn infrastructure_input_matches(
    request: &ProviderOperationRequest,
    method: ProviderMethod,
) -> bool {
    matches!(
        (method, &request.input),
        (
            ProviderMethod::InfrastructureSetPowerState,
            ProviderOperationInput::InfrastructurePowerState { .. }
        ) | (
            ProviderMethod::InfrastructureBootstrapBinding,
            ProviderOperationInput::TransportBinding { .. }
        ) | (
            ProviderMethod::InfrastructurePlan
                | ProviderMethod::InfrastructureInspect
                | ProviderMethod::InfrastructureDestroy,
            ProviderOperationInput::NoInput
        )
    )
}

fn power_lifecycle(state: PowerState) -> ObservedLifecycleState {
    match state {
        PowerState::Running => ObservedLifecycleState::Running,
        PowerState::Stopped => ObservedLifecycleState::Stopped,
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

fn resource_id(value: &str) -> Result<ResourceId, FakeSdkErrorKind> {
    ResourceId::new(bounded_hash(value))
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
