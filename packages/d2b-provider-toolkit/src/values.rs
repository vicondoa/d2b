use std::fmt;

use d2b_contracts::{
    v2_component_session::BoundedVec,
    v2_identity::{RealmId, WorkloadId},
    v2_provider::{
        AdoptionState, Generation, HandleId, HandleOwner, MAX_PROVIDER_PLAN_RESOURCES,
        MAX_SAFE_JSON_INTEGER, MutationReceipt, MutationState, ObservationReason,
        ObservedLifecycleState, OperationBinding, OwnershipTransfer, PROVIDER_SCHEMA_VERSION,
        PlanId, PlannedResourceClass, ProviderContractError, ProviderDescriptor, ProviderFailure,
        ProviderFailureKind, ProviderHandle, ProviderHandleKind, ProviderHealth,
        ProviderHealthReason, ProviderHealthState, ProviderObservation, ProviderOperationContext,
        ProviderOperationRequest, ProviderPlan, ProviderRemediation, RetryClass,
    },
};

#[derive(Clone)]
pub struct ProviderValues {
    descriptor: ProviderDescriptor,
    now_unix_ms: u64,
}

impl fmt::Debug for ProviderValues {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderValues")
            .field("provider_type", &self.descriptor.provider_type())
            .field("provider_generation", &self.descriptor.registry_generation)
            .field("now_unix_ms", &self.now_unix_ms)
            .finish_non_exhaustive()
    }
}

impl ProviderValues {
    pub fn new(
        descriptor: &ProviderDescriptor,
        now_unix_ms: u64,
    ) -> Result<Self, ProviderContractError> {
        descriptor.validate()?;
        if now_unix_ms > MAX_SAFE_JSON_INTEGER {
            return Err(ProviderContractError::BoundExceeded);
        }
        Ok(Self {
            descriptor: descriptor.clone(),
            now_unix_ms,
        })
    }

    pub fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    pub const fn now_unix_ms(&self) -> u64 {
        self.now_unix_ms
    }

    pub fn provider_owner(&self, realm_id: &RealmId) -> HandleOwner {
        HandleOwner::Provider {
            realm_id: realm_id.clone(),
            provider_id: self.descriptor.provider_id.clone(),
        }
    }

    pub fn health(
        &self,
        state: ProviderHealthState,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> Result<ProviderHealth, ProviderContractError> {
        let health = ProviderHealth {
            provider_id: self.descriptor.provider_id.clone(),
            registry_generation: self.descriptor.registry_generation,
            observed_at_unix_ms: self.now_unix_ms,
            state,
            reason,
            remediation,
        };
        health.validate()?;
        Ok(health)
    }

    pub fn plan(
        &self,
        request: &ProviderOperationRequest,
        plan_id: PlanId,
        expires_at_unix_ms: u64,
        resources: BoundedVec<PlannedResourceClass, 0, MAX_PROVIDER_PLAN_RESOURCES>,
    ) -> Result<ProviderPlan, ProviderContractError> {
        request.validate(&self.descriptor, self.now_unix_ms)?;
        let plan = ProviderPlan {
            schema_version: PROVIDER_SCHEMA_VERSION,
            plan_id,
            binding: request.context.binding(),
            realm_id: request.target.realm_id().clone(),
            workload_id: request.target.workload_id().cloned(),
            method: request.context.method,
            configuration_fingerprint: request.expected_configuration_fingerprint.clone(),
            created_at_unix_ms: self.now_unix_ms,
            expires_at_unix_ms,
            resources,
        };
        plan.validate(request, self.now_unix_ms)?;
        Ok(plan)
    }

    pub fn handle_from_request(
        &self,
        request: &ProviderOperationRequest,
        handle_id: HandleId,
        owner: HandleOwner,
        resource_generation: Generation,
        expires_at_unix_ms: Option<u64>,
    ) -> Result<ProviderHandle, ProviderContractError> {
        request.validate(&self.descriptor, self.now_unix_ms)?;
        self.handle(
            request.context.binding(),
            request.target.realm_id().clone(),
            request.target.workload_id().cloned(),
            request.expected_configuration_fingerprint.clone(),
            handle_id,
            owner,
            resource_generation,
            expires_at_unix_ms,
        )
    }

    pub fn handle_from_plan(
        &self,
        plan: &ProviderPlan,
        handle_id: HandleId,
        owner: HandleOwner,
        resource_generation: Generation,
        expires_at_unix_ms: Option<u64>,
    ) -> Result<ProviderHandle, ProviderContractError> {
        if plan.schema_version != PROVIDER_SCHEMA_VERSION
            || plan.binding.provider_id != self.descriptor.provider_id
            || plan.binding.provider_generation != self.descriptor.registry_generation
            || plan.method.provider_type() != self.descriptor.provider_type()
            || plan.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || plan.created_at_unix_ms > self.now_unix_ms
            || plan.expires_at_unix_ms <= self.now_unix_ms
        {
            return Err(ProviderContractError::OperationBindingMismatch);
        }
        self.handle(
            plan.binding.clone(),
            plan.realm_id.clone(),
            plan.workload_id.clone(),
            plan.configuration_fingerprint.clone(),
            handle_id,
            owner,
            resource_generation,
            expires_at_unix_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn handle(
        &self,
        binding: OperationBinding,
        realm_id: RealmId,
        workload_id: Option<WorkloadId>,
        configuration_fingerprint: d2b_contracts::v2_provider::Fingerprint,
        handle_id: HandleId,
        owner: HandleOwner,
        resource_generation: Generation,
        expires_at_unix_ms: Option<u64>,
    ) -> Result<ProviderHandle, ProviderContractError> {
        let handle = ProviderHandle {
            schema_version: PROVIDER_SCHEMA_VERSION,
            handle_id,
            kind: handle_kind(self.descriptor.provider_type()),
            provider_id: self.descriptor.provider_id.clone(),
            realm_id,
            workload_id,
            owner,
            provider_generation: self.descriptor.registry_generation,
            resource_generation,
            configuration_fingerprint,
            created_by: binding,
            created_at_unix_ms: self.now_unix_ms,
            expires_at_unix_ms,
            ownership_transfer: OwnershipTransfer::Stationary {
                ownership_epoch: Generation::new(1)?,
            },
        };
        handle.validate()?;
        Ok(handle)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn observation(
        &self,
        context: &ProviderOperationContext,
        handle: Option<&ProviderHandle>,
        lifecycle: ObservedLifecycleState,
        adoption: AdoptionState,
        reason: ObservationReason,
        health_state: ProviderHealthState,
        health_reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> Result<ProviderObservation, ProviderContractError> {
        context.validate(&self.descriptor, self.now_unix_ms)?;
        if let Some(handle) = handle {
            handle.validate()?;
            if handle.provider_id != self.descriptor.provider_id
                || handle.provider_generation != self.descriptor.registry_generation
                || &handle.realm_id != context.scope.realm_id()
                || handle.workload_id.as_ref() != context.scope.workload_id()
                || handle.configuration_fingerprint
                    != self.descriptor.configuration_schema_fingerprint
            {
                return Err(ProviderContractError::HandleBindingMismatch);
            }
        }
        let observation = ProviderObservation {
            provider_id: self.descriptor.provider_id.clone(),
            provider_generation: self.descriptor.registry_generation,
            realm_id: context.scope.realm_id().clone(),
            workload_id: context.scope.workload_id().cloned(),
            handle_id: handle.map(|value| value.handle_id.clone()),
            resource_generation: handle.map(|value| value.resource_generation),
            observed_at_unix_ms: self.now_unix_ms,
            lifecycle,
            adoption,
            reason,
            health: self.health(health_state, health_reason, remediation)?,
        };
        observation.validate()?;
        Ok(observation)
    }

    pub fn receipt(
        &self,
        context: &ProviderOperationContext,
        state: MutationState,
    ) -> Result<MutationReceipt, ProviderContractError> {
        context.validate(&self.descriptor, self.now_unix_ms)?;
        let receipt = MutationReceipt {
            binding: context.binding(),
            state,
            observed_at_unix_ms: self.now_unix_ms,
            observation_required_before_retry: state == MutationState::CompletionAmbiguous,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn failure(
        &self,
        context: &ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> Result<ProviderFailure, ProviderContractError> {
        context.validate(&self.descriptor, self.now_unix_ms)?;
        let failure = ProviderFailure {
            kind,
            retry,
            provider_type: self.descriptor.provider_type(),
            binding: context.binding(),
            correlation_id: context.correlation_id.clone(),
            occurred_at_unix_ms: self.now_unix_ms,
            reason,
            remediation,
        };
        failure.validate_against(&self.descriptor)?;
        Ok(failure)
    }
}

const fn handle_kind(
    provider_type: d2b_contracts::v2_identity::ProviderType,
) -> ProviderHandleKind {
    match provider_type {
        d2b_contracts::v2_identity::ProviderType::Runtime => ProviderHandleKind::Runtime,
        d2b_contracts::v2_identity::ProviderType::Infrastructure => {
            ProviderHandleKind::Infrastructure
        }
        d2b_contracts::v2_identity::ProviderType::Transport => ProviderHandleKind::Transport,
        d2b_contracts::v2_identity::ProviderType::Display => ProviderHandleKind::Display,
        d2b_contracts::v2_identity::ProviderType::Network => ProviderHandleKind::Network,
        d2b_contracts::v2_identity::ProviderType::Storage => ProviderHandleKind::Storage,
        d2b_contracts::v2_identity::ProviderType::Device => ProviderHandleKind::Device,
        d2b_contracts::v2_identity::ProviderType::Audio => ProviderHandleKind::Audio,
        d2b_contracts::v2_identity::ProviderType::Substrate
        | d2b_contracts::v2_identity::ProviderType::Credential
        | d2b_contracts::v2_identity::ProviderType::Observability => {
            ProviderHandleKind::Observation
        }
    }
}
