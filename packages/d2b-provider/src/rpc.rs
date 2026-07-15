#![allow(clippy::result_large_err)]

use std::{
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        AdoptionRequest, AudioProvider, CredentialLease, CredentialLeaseRequest,
        CredentialLeaseState, CredentialLeaseTransferPolicy, CredentialProvider, DeviceProvider,
        DisplayProvider, InfrastructureProvider, MAX_CREDENTIAL_OPERATION_CLASSES,
        MAX_PROVIDER_LEASE_LIFETIME_MS, MutationReceipt, NetworkProvider, ObservabilityProvider,
        PROVIDER_SCHEMA_VERSION, Provider, ProviderCallContext, ProviderCapabilitySet,
        ProviderDescriptor, ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle,
        ProviderHealth, ProviderHealthReason, ProviderMethod, ProviderObservation,
        ProviderOperationRequest, ProviderPlan, ProviderRemediation, ProviderResult, RetryClass,
        RuntimeProvider, StorageProvider, SubstrateProvider, TransportProvider,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub peer_role: EndpointRole,
    pub service: ServicePackage,
    pub provider_id: ProviderId,
    pub provider_type: ProviderType,
    pub provider_generation: d2b_contracts::v2_provider::Generation,
}

pub trait ProviderClock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemProviderClock;

impl ProviderClock for SystemProviderClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcOperation {
    Health,
    Capabilities,
    Method(ProviderMethod),
}

pub enum RpcPayload<'a> {
    None,
    Operation(&'a ProviderOperationRequest),
    Plan(&'a ProviderPlan),
    Adoption(&'a AdoptionRequest),
    LeaseRequest(&'a CredentialLeaseRequest),
    Lease(&'a CredentialLease),
}

impl fmt::Debug for RpcPayload<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::None => "RpcPayload::None",
            Self::Operation(_) => "RpcPayload::Operation(<redacted>)",
            Self::Plan(_) => "RpcPayload::Plan(<redacted>)",
            Self::Adoption(_) => "RpcPayload::Adoption(<redacted>)",
            Self::LeaseRequest(_) => "RpcPayload::LeaseRequest(<redacted>)",
            Self::Lease(_) => "RpcPayload::Lease(<redacted>)",
        })
    }
}

pub struct RpcCall<'a> {
    pub operation: RpcOperation,
    pub context: &'a ProviderCallContext<'a>,
    pub payload: RpcPayload<'a>,
}

impl fmt::Debug for RpcCall<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RpcCall")
            .field("operation", &self.operation)
            .field("payload", &self.payload)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub enum RpcResponse {
    Health(Box<ProviderHealth>),
    Capabilities(ProviderCapabilitySet),
    Plan(Box<ProviderPlan>),
    Handle(Box<ProviderHandle>),
    Observation(Box<ProviderObservation>),
    Mutation(Box<MutationReceipt>),
    Lease(Box<CredentialLease>),
}

impl fmt::Debug for RpcResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Health(_) => "RpcResponse::Health(<redacted>)",
            Self::Capabilities(_) => "RpcResponse::Capabilities(<redacted>)",
            Self::Plan(_) => "RpcResponse::Plan(<redacted>)",
            Self::Handle(_) => "RpcResponse::Handle(<redacted>)",
            Self::Observation(_) => "RpcResponse::Observation(<redacted>)",
            Self::Mutation(_) => "RpcResponse::Mutation(<redacted>)",
            Self::Lease(_) => "RpcResponse::Lease(<redacted>)",
        })
    }
}

#[async_trait]
pub trait AuthenticatedProviderRpc: Send + Sync {
    fn session_identity(&self) -> SessionIdentity;
    async fn invoke(&self, call: RpcCall<'_>) -> ProviderResult<RpcResponse>;
}

pub struct RpcProviderProxy {
    descriptor: ProviderDescriptor,
    rpc: Arc<dyn AuthenticatedProviderRpc>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for RpcProviderProxy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RpcProviderProxy")
            .field("provider_type", &self.descriptor.provider_type())
            .field("generation", &self.descriptor.registry_generation)
            .finish_non_exhaustive()
    }
}

impl RpcProviderProxy {
    pub fn new(
        descriptor: ProviderDescriptor,
        rpc: Arc<dyn AuthenticatedProviderRpc>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, d2b_contracts::v2_provider::ProviderContractError> {
        descriptor.validate()?;
        Ok(Self {
            descriptor,
            rpc,
            clock,
        })
    }

    fn failure(
        &self,
        context: &ProviderCallContext<'_>,
        kind: ProviderFailureKind,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry: match kind {
                ProviderFailureKind::AmbiguousMutation => RetryClass::AfterObservation,
                _ => RetryClass::Never,
            },
            provider_type: self.descriptor.provider_type(),
            binding: context.operation.binding(),
            correlation_id: context.operation.correlation_id.clone(),
            occurred_at_unix_ms: self.clock.now_unix_ms(),
            reason,
            remediation,
        }
    }

    fn preflight(
        &self,
        context: &ProviderCallContext<'_>,
        expected: ProviderMethod,
    ) -> ProviderResult<()> {
        if context.cancelled {
            return Err(self.failure(
                context,
                ProviderFailureKind::Cancelled,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.failure(
                context,
                ProviderFailureKind::DeadlineExpired,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.validate().is_err()
            || context
                .operation
                .validate(&self.descriptor, self.clock.now_unix_ms())
                .is_err()
            || context.operation.method != expected
        {
            return Err(self.failure(
                context,
                ProviderFailureKind::InvalidRequest,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        let identity = self.rpc.session_identity();
        let placement_matches = self
            .descriptor
            .placement
            .agent_binding()
            .is_some_and(|binding| {
                binding.agent_generation == identity.provider_generation
                    && identity.peer_role == EndpointRole::ProviderAgent
                    && identity.service == ServicePackage::ProviderV2
            });
        if identity.provider_id != self.descriptor.provider_id
            || identity.provider_type != self.descriptor.provider_type()
            || identity.provider_generation != self.descriptor.registry_generation
            || !placement_matches
        {
            return Err(self.failure(
                context,
                ProviderFailureKind::UnauthorizedScope,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        Ok(())
    }

    fn validate_health(
        &self,
        context: &ProviderCallContext<'_>,
        health: ProviderHealth,
    ) -> ProviderResult<ProviderHealth> {
        if health.validate().is_err()
            || health.provider_id != self.descriptor.provider_id
            || health.registry_generation != self.descriptor.registry_generation
        {
            Err(self.response_mismatch(context))
        } else {
            Ok(health)
        }
    }

    fn validate_observation(
        &self,
        context: &ProviderCallContext<'_>,
        observation: ProviderObservation,
    ) -> ProviderResult<ProviderObservation> {
        if observation.validate().is_err()
            || observation.provider_id != self.descriptor.provider_id
            || observation.provider_generation != self.descriptor.registry_generation
            || observation.realm_id != *context.operation.scope.realm_id()
            || observation.workload_id.as_ref() != context.operation.scope.workload_id()
        {
            Err(self.response_mismatch(context))
        } else {
            Ok(observation)
        }
    }

    fn validate_handle(
        &self,
        context: &ProviderCallContext<'_>,
        handle: ProviderHandle,
    ) -> ProviderResult<ProviderHandle> {
        if handle.validate().is_err()
            || handle.provider_id != self.descriptor.provider_id
            || handle.provider_generation != self.descriptor.registry_generation
            || handle.realm_id != *context.operation.scope.realm_id()
            || handle.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
        {
            Err(self.response_mismatch(context))
        } else {
            Ok(handle)
        }
    }

    fn validate_mutation(
        &self,
        context: &ProviderCallContext<'_>,
        receipt: MutationReceipt,
    ) -> ProviderResult<MutationReceipt> {
        if receipt.validate().is_err() || receipt.binding != context.operation.binding() {
            Err(self.response_mismatch(context))
        } else {
            Ok(receipt)
        }
    }

    fn validate_lease(
        &self,
        context: &ProviderCallContext<'_>,
        lease: CredentialLease,
    ) -> ProviderResult<CredentialLease> {
        if lease.credential_provider_id != self.descriptor.provider_id
            || lease.credential_provider_generation != self.descriptor.registry_generation
        {
            Err(self.response_mismatch(context))
        } else {
            Ok(lease)
        }
    }

    fn validate_plan_input(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
    ) -> ProviderResult<()> {
        let now = self.clock.now_unix_ms();
        if plan.schema_version != PROVIDER_SCHEMA_VERSION
            || plan.binding.provider_id != self.descriptor.provider_id
            || plan.binding.provider_generation != self.descriptor.registry_generation
            || plan.realm_id != *context.operation.scope.realm_id()
            || plan.workload_id.as_ref() != context.operation.scope.workload_id()
            || plan.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || plan.method.provider_type() != self.descriptor.provider_type()
            || plan.created_at_unix_ms > now
            || plan.expires_at_unix_ms <= now
        {
            Err(self.response_mismatch(context))
        } else {
            Ok(())
        }
    }

    fn validate_lease_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &CredentialLeaseRequest,
    ) -> ProviderResult<()> {
        let now = self.clock.now_unix_ms();
        let binding = self.descriptor.placement.agent_binding();
        if request.context != *context.operation
            || binding.as_ref() != Some(&request.agent_binding)
            || request.consumer_provider_id == self.descriptor.provider_id
            || request.allowed_operations.is_empty()
            || request.allowed_operations.len() > MAX_CREDENTIAL_OPERATION_CLASSES
            || request
                .allowed_operations
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || request.requested_expiry_unix_ms <= now
            || request.requested_expiry_unix_ms - now > MAX_PROVIDER_LEASE_LIFETIME_MS
        {
            Err(self.response_mismatch(context))
        } else {
            Ok(())
        }
    }

    fn validate_acquired_lease(
        &self,
        context: &ProviderCallContext<'_>,
        request: &CredentialLeaseRequest,
        lease: CredentialLease,
    ) -> ProviderResult<CredentialLease> {
        let now = self.clock.now_unix_ms();
        if lease.credential_provider_id != self.descriptor.provider_id
            || lease.consumer_provider_id != request.consumer_provider_id
            || lease.agent_binding != request.agent_binding
            || lease.allowed_operations != request.allowed_operations
            || lease.credential_provider_generation != self.descriptor.registry_generation
            || lease.issued_at_unix_ms > now
            || lease.expires_at_unix_ms <= now
            || lease.expires_at_unix_ms > request.requested_expiry_unix_ms
            || lease.state != CredentialLeaseState::Active
            || lease.transfer_policy != CredentialLeaseTransferPolicy::Forbidden
            || lease.revoked_at_unix_ms.is_some()
        {
            Err(self.response_mismatch(context))
        } else {
            Ok(lease)
        }
    }

    fn response_mismatch(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::InvariantViolation,
            ProviderHealthReason::CapabilityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    async fn call(
        &self,
        context: &ProviderCallContext<'_>,
        method: ProviderMethod,
        payload: RpcPayload<'_>,
    ) -> ProviderResult<RpcResponse> {
        self.preflight(context, method)?;
        self.rpc
            .invoke(RpcCall {
                operation: RpcOperation::Method(method),
                context,
                payload,
            })
            .await
    }

    async fn call_operation_plan(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        method: ProviderMethod,
    ) -> ProviderResult<ProviderPlan> {
        if request
            .validate_method(&self.descriptor, self.clock.now_unix_ms(), method)
            .is_err()
        {
            return Err(self.response_mismatch(context));
        }
        match self
            .call(context, method, RpcPayload::Operation(request))
            .await?
        {
            RpcResponse::Plan(plan) if plan.validate(request, self.clock.now_unix_ms()).is_ok() => {
                Ok(*plan)
            }
            _ => Err(self.response_mismatch(context)),
        }
    }

    async fn call_plan_handle(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
        method: ProviderMethod,
    ) -> ProviderResult<ProviderHandle> {
        self.validate_plan_input(context, plan)?;
        match self.call(context, method, RpcPayload::Plan(plan)).await? {
            RpcResponse::Handle(handle) => self.validate_handle(context, *handle),
            _ => Err(self.response_mismatch(context)),
        }
    }

    async fn call_operation_handle(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        method: ProviderMethod,
    ) -> ProviderResult<ProviderHandle> {
        if request
            .validate_method(&self.descriptor, self.clock.now_unix_ms(), method)
            .is_err()
        {
            return Err(self.response_mismatch(context));
        }
        match self
            .call(context, method, RpcPayload::Operation(request))
            .await?
        {
            RpcResponse::Handle(handle) => self.validate_handle(context, *handle),
            _ => Err(self.response_mismatch(context)),
        }
    }

    async fn call_operation_observation(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        method: ProviderMethod,
    ) -> ProviderResult<ProviderObservation> {
        if request
            .validate_method(&self.descriptor, self.clock.now_unix_ms(), method)
            .is_err()
        {
            return Err(self.response_mismatch(context));
        }
        match self
            .call(context, method, RpcPayload::Operation(request))
            .await?
        {
            RpcResponse::Observation(observation) => {
                self.validate_observation(context, *observation)
            }
            _ => Err(self.response_mismatch(context)),
        }
    }

    async fn call_adoption(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
        method: ProviderMethod,
    ) -> ProviderResult<ProviderObservation> {
        if request
            .validate(&self.descriptor, self.clock.now_unix_ms())
            .is_err()
        {
            return Err(self.response_mismatch(context));
        }
        match self
            .call(context, method, RpcPayload::Adoption(request))
            .await?
        {
            RpcResponse::Observation(observation) => {
                self.validate_observation(context, *observation)
            }
            _ => Err(self.response_mismatch(context)),
        }
    }

    async fn call_operation_mutation(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        method: ProviderMethod,
    ) -> ProviderResult<MutationReceipt> {
        if request
            .validate_method(&self.descriptor, self.clock.now_unix_ms(), method)
            .is_err()
        {
            return Err(self.response_mismatch(context));
        }
        match self
            .call(context, method, RpcPayload::Operation(request))
            .await?
        {
            RpcResponse::Mutation(receipt) => self.validate_mutation(context, *receipt),
            _ => Err(self.response_mismatch(context)),
        }
    }

    async fn call_plan_mutation(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
        method: ProviderMethod,
    ) -> ProviderResult<MutationReceipt> {
        self.validate_plan_input(context, plan)?;
        match self.call(context, method, RpcPayload::Plan(plan)).await? {
            RpcResponse::Mutation(receipt) => self.validate_mutation(context, *receipt),
            _ => Err(self.response_mismatch(context)),
        }
    }
}

impl Provider for RpcProviderProxy {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            self.preflight(context, context.operation.method)?;
            match self
                .rpc
                .invoke(RpcCall {
                    operation: RpcOperation::Health,
                    context,
                    payload: RpcPayload::None,
                })
                .await?
            {
                RpcResponse::Health(health) => self.validate_health(context, *health),
                _ => Err(self.response_mismatch(context)),
            }
        })
    }
}

macro_rules! capabilities {
    () => {
        fn capabilities(&self) -> ProviderCapabilitySet {
            self.descriptor.capabilities.clone()
        }
    };
}

macro_rules! operation_plan {
    ($name:ident, $method:ident) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            request: &'a ProviderOperationRequest,
        ) -> ProviderFuture<'a, ProviderPlan> {
            Box::pin(self.call_operation_plan(context, request, ProviderMethod::$method))
        }
    };
}

macro_rules! plan_handle {
    ($name:ident, $method:ident) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            plan: &'a ProviderPlan,
        ) -> ProviderFuture<'a, ProviderHandle> {
            Box::pin(self.call_plan_handle(context, plan, ProviderMethod::$method))
        }
    };
}

macro_rules! operation_handle {
    ($name:ident, $method:ident) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            request: &'a ProviderOperationRequest,
        ) -> ProviderFuture<'a, ProviderHandle> {
            Box::pin(self.call_operation_handle(context, request, ProviderMethod::$method))
        }
    };
}

macro_rules! operation_observation {
    ($name:ident, $method:ident) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            request: &'a ProviderOperationRequest,
        ) -> ProviderFuture<'a, ProviderObservation> {
            Box::pin(self.call_operation_observation(context, request, ProviderMethod::$method))
        }
    };
}

macro_rules! adoption {
    ($name:ident, $method:ident) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            request: &'a AdoptionRequest,
        ) -> ProviderFuture<'a, ProviderObservation> {
            Box::pin(self.call_adoption(context, request, ProviderMethod::$method))
        }
    };
}

macro_rules! operation_mutation {
    ($name:ident, $method:ident) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            request: &'a ProviderOperationRequest,
        ) -> ProviderFuture<'a, MutationReceipt> {
            Box::pin(self.call_operation_mutation(context, request, ProviderMethod::$method))
        }
    };
}

macro_rules! plan_mutation {
    ($name:ident, $method:ident) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            plan: &'a ProviderPlan,
        ) -> ProviderFuture<'a, MutationReceipt> {
            Box::pin(self.call_plan_mutation(context, plan, ProviderMethod::$method))
        }
    };
}

impl RuntimeProvider for RpcProviderProxy {
    capabilities!();
    operation_plan!(plan, RuntimePlan);
    plan_handle!(ensure, RuntimeEnsure);
    operation_observation!(start, RuntimeStart);
    operation_observation!(stop, RuntimeStop);
    operation_observation!(inspect, RuntimeInspect);
    adoption!(adopt, RuntimeAdopt);
    operation_mutation!(destroy, RuntimeDestroy);
}

impl InfrastructureProvider for RpcProviderProxy {
    capabilities!();
    operation_plan!(plan, InfrastructurePlan);
    plan_handle!(apply, InfrastructureApply);
    operation_observation!(set_power_state, InfrastructureSetPowerState);
    operation_observation!(inspect, InfrastructureInspect);
    adoption!(adopt, InfrastructureAdopt);
    operation_handle!(bootstrap_binding, InfrastructureBootstrapBinding);
    operation_mutation!(destroy, InfrastructureDestroy);
}

impl TransportProvider for RpcProviderProxy {
    capabilities!();
    operation_handle!(connect, TransportConnect);
    operation_handle!(listen, TransportListen);
    operation_handle!(issue_binding, TransportIssueBinding);
    operation_mutation!(revoke_binding, TransportRevokeBinding);
    operation_observation!(inspect, TransportInspect);
}

impl SubstrateProvider for RpcProviderProxy {
    capabilities!();
    operation_observation!(check, SubstrateCheck);
    operation_plan!(plan_remediation, SubstratePlanRemediation);
    plan_mutation!(apply, SubstrateApply);
}

impl CredentialProvider for RpcProviderProxy {
    fn status<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.call_operation_observation(
            context,
            request,
            ProviderMethod::CredentialStatus,
        ))
    }

    fn acquire_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a CredentialLeaseRequest,
    ) -> ProviderFuture<'a, CredentialLease> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialAcquireLease)?;
            self.validate_lease_request(context, request)?;
            match self
                .rpc
                .invoke(RpcCall {
                    operation: RpcOperation::Method(ProviderMethod::CredentialAcquireLease),
                    context,
                    payload: RpcPayload::LeaseRequest(request),
                })
                .await?
            {
                RpcResponse::Lease(lease) => self.validate_acquired_lease(context, request, *lease),
                _ => Err(self.response_mismatch(context)),
            }
        })
    }

    fn refresh_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        lease: &'a CredentialLease,
    ) -> ProviderFuture<'a, CredentialLease> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialRefreshLease)?;
            match self
                .rpc
                .invoke(RpcCall {
                    operation: RpcOperation::Method(ProviderMethod::CredentialRefreshLease),
                    context,
                    payload: RpcPayload::Lease(lease),
                })
                .await?
            {
                RpcResponse::Lease(lease) => self.validate_lease(context, *lease),
                _ => Err(self.response_mismatch(context)),
            }
        })
    }

    fn revoke_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        lease: &'a CredentialLease,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move {
            self.preflight(context, ProviderMethod::CredentialRevokeLease)?;
            match self
                .rpc
                .invoke(RpcCall {
                    operation: RpcOperation::Method(ProviderMethod::CredentialRevokeLease),
                    context,
                    payload: RpcPayload::Lease(lease),
                })
                .await?
            {
                RpcResponse::Mutation(receipt) => self.validate_mutation(context, *receipt),
                _ => Err(self.response_mismatch(context)),
            }
        })
    }
}

impl DisplayProvider for RpcProviderProxy {
    capabilities!();
    operation_handle!(open, DisplayOpen);
    operation_observation!(inspect, DisplayInspect);
    adoption!(adopt, DisplayAdopt);
    operation_mutation!(close, DisplayClose);
}

impl NetworkProvider for RpcProviderProxy {
    capabilities!();
    operation_plan!(plan, NetworkPlan);
    plan_handle!(ensure, NetworkEnsure);
    operation_observation!(inspect, NetworkInspect);
    adoption!(adopt, NetworkAdopt);
    operation_mutation!(release, NetworkRelease);
}

impl StorageProvider for RpcProviderProxy {
    capabilities!();
    operation_plan!(plan, StoragePlan);
    plan_handle!(ensure, StorageEnsure);
    operation_observation!(inspect, StorageInspect);
    adoption!(adopt, StorageAdopt);
    operation_handle!(snapshot, StorageSnapshot);
    operation_mutation!(destroy, StorageDestroy);
}

impl DeviceProvider for RpcProviderProxy {
    capabilities!();
    operation_plan!(plan_attach, DevicePlanAttach);
    plan_handle!(attach, DeviceAttach);
    operation_observation!(inspect, DeviceInspect);
    adoption!(adopt, DeviceAdopt);
    operation_mutation!(detach, DeviceDetach);
}

impl AudioProvider for RpcProviderProxy {
    capabilities!();
    operation_handle!(open, AudioOpen);
    operation_observation!(set_state, AudioSetState);
    operation_observation!(inspect, AudioInspect);
    adoption!(adopt, AudioAdopt);
    operation_mutation!(close, AudioClose);
}

impl ObservabilityProvider for RpcProviderProxy {
    capabilities!();
    operation_observation!(status, ObservabilityStatus);
    operation_observation!(query, ObservabilityQuery);
    operation_handle!(subscribe, ObservabilitySubscribe);
    operation_mutation!(export, ObservabilityExport);
}
