#![allow(clippy::result_large_err)]

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use d2b_contracts::v2_provider::{
    ProviderFailure, ProviderFailureKind, ProviderHealthReason, ProviderMethod,
    ProviderRemediation, ProviderResult, RetryClass,
};
use d2b_provider::{
    AuthenticatedProviderRpc, ProviderClock, ProviderInstance, RpcCall, RpcOperation, RpcPayload,
    RpcResponse, SessionIdentity,
};

use crate::ToolkitError;

pub struct ProviderAgentAdapter {
    instance: ProviderInstance,
    identity: SessionIdentity,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for ProviderAgentAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderAgentAdapter")
            .field("provider_type", &self.instance.provider_type())
            .field("generation", &self.identity.provider_generation)
            .finish_non_exhaustive()
    }
}

impl ProviderAgentAdapter {
    pub fn new(
        instance: ProviderInstance,
        identity: SessionIdentity,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, ToolkitError> {
        let descriptor = instance.descriptor();
        descriptor
            .validate()
            .map_err(|_| ToolkitError::DescriptorInvalid)?;
        if instance.capabilities() != descriptor.capabilities {
            return Err(ToolkitError::CapabilityMismatch);
        }
        let binding = descriptor
            .placement
            .agent_binding()
            .ok_or(ToolkitError::DescriptorInvalid)?;
        if identity.provider_id != descriptor.provider_id
            || identity.provider_type != descriptor.provider_type()
            || identity.provider_generation != descriptor.registry_generation
            || identity.peer_role
                != d2b_contracts::v2_component_session::EndpointRole::ProviderAgent
            || identity.service != d2b_contracts::v2_component_session::ServicePackage::ProviderV2
            || binding.agent_generation != identity.provider_generation
        {
            return Err(ToolkitError::DescriptorInvalid);
        }
        Ok(Self {
            instance,
            identity,
            clock,
        })
    }

    pub fn descriptor(&self) -> d2b_contracts::v2_provider::ProviderDescriptor {
        self.instance.descriptor()
    }

    pub fn published_capabilities(&self) -> d2b_contracts::v2_provider::ProviderCapabilitySet {
        self.instance.capabilities()
    }

    fn failure(
        &self,
        call: &RpcCall<'_>,
        kind: ProviderFailureKind,
        reason: ProviderHealthReason,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry: RetryClass::Never,
            provider_type: self.instance.provider_type(),
            binding: call.context.operation.binding(),
            correlation_id: call.context.operation.correlation_id.clone(),
            occurred_at_unix_ms: self.clock.now_unix_ms(),
            reason,
            remediation: ProviderRemediation::RepairConfiguration,
        }
    }

    fn validate_call(&self, call: &RpcCall<'_>) -> ProviderResult<()> {
        let descriptor = self.instance.descriptor();
        call.context.validate().map_err(|_| {
            self.failure(
                call,
                ProviderFailureKind::InvalidRequest,
                ProviderHealthReason::CapabilityMismatch,
            )
        })?;
        call.context
            .operation
            .validate(&descriptor, self.clock.now_unix_ms())
            .map_err(|_| {
                self.failure(
                    call,
                    ProviderFailureKind::UnauthorizedScope,
                    ProviderHealthReason::IdentityMismatch,
                )
            })?;
        if let RpcOperation::Method(method) = call.operation
            && (!self.instance.capabilities().contains_method(method)
                || call.context.operation.method != method)
        {
            return Err(self.failure(
                call,
                ProviderFailureKind::CapabilityDenied,
                ProviderHealthReason::CapabilityMismatch,
            ));
        }
        Ok(())
    }
}

macro_rules! dispatch_operation {
    ($provider:expr, $method:ident, $context:expr, $payload:expr, $response:ident) => {
        match $payload {
            RpcPayload::Operation(request) => Some(
                $provider
                    .$method($context, request)
                    .await
                    .map(|value| RpcResponse::$response(Box::new(value))),
            ),
            _ => None,
        }
    };
}

macro_rules! dispatch_plan {
    ($provider:expr, $method:ident, $context:expr, $payload:expr, $response:ident) => {
        match $payload {
            RpcPayload::Plan(plan) => Some(
                $provider
                    .$method($context, plan)
                    .await
                    .map(|value| RpcResponse::$response(Box::new(value))),
            ),
            _ => None,
        }
    };
}

macro_rules! dispatch_adoption {
    ($provider:expr, $method:ident, $context:expr, $payload:expr) => {
        match $payload {
            RpcPayload::Adoption(request) => Some(
                $provider
                    .$method($context, request)
                    .await
                    .map(|value| RpcResponse::Observation(Box::new(value))),
            ),
            _ => None,
        }
    };
}

#[async_trait]
impl AuthenticatedProviderRpc for ProviderAgentAdapter {
    fn session_identity(&self) -> SessionIdentity {
        self.identity.clone()
    }

    async fn invoke(&self, call: RpcCall<'_>) -> ProviderResult<RpcResponse> {
        self.validate_call(&call)?;
        match call.operation {
            RpcOperation::Health => self
                .instance
                .provider()
                .health(call.context)
                .await
                .map(|value| RpcResponse::Health(Box::new(value))),
            RpcOperation::Capabilities => {
                Ok(RpcResponse::Capabilities(self.instance.capabilities()))
            }
            RpcOperation::Method(method) => {
                let result = match (&self.instance, method) {
                    (ProviderInstance::Runtime(provider), ProviderMethod::RuntimePlan) => {
                        dispatch_operation!(provider, plan, call.context, call.payload, Plan)
                    }
                    (ProviderInstance::Runtime(provider), ProviderMethod::RuntimeEnsure) => {
                        dispatch_plan!(provider, ensure, call.context, call.payload, Handle)
                    }
                    (ProviderInstance::Runtime(provider), ProviderMethod::RuntimeStart) => {
                        dispatch_operation!(
                            provider,
                            start,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Runtime(provider), ProviderMethod::RuntimeStop) => {
                        dispatch_operation!(provider, stop, call.context, call.payload, Observation)
                    }
                    (ProviderInstance::Runtime(provider), ProviderMethod::RuntimeInspect) => {
                        dispatch_operation!(
                            provider,
                            inspect,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Runtime(provider), ProviderMethod::RuntimeAdopt) => {
                        dispatch_adoption!(provider, adopt, call.context, call.payload)
                    }
                    (ProviderInstance::Runtime(provider), ProviderMethod::RuntimeDestroy) => {
                        dispatch_operation!(provider, destroy, call.context, call.payload, Mutation)
                    }
                    (
                        ProviderInstance::Infrastructure(provider),
                        ProviderMethod::InfrastructurePlan,
                    ) => dispatch_operation!(provider, plan, call.context, call.payload, Plan),
                    (
                        ProviderInstance::Infrastructure(provider),
                        ProviderMethod::InfrastructureApply,
                    ) => dispatch_plan!(provider, apply, call.context, call.payload, Handle),
                    (
                        ProviderInstance::Infrastructure(provider),
                        ProviderMethod::InfrastructureSetPowerState,
                    ) => dispatch_operation!(
                        provider,
                        set_power_state,
                        call.context,
                        call.payload,
                        Observation
                    ),
                    (
                        ProviderInstance::Infrastructure(provider),
                        ProviderMethod::InfrastructureInspect,
                    ) => dispatch_operation!(
                        provider,
                        inspect,
                        call.context,
                        call.payload,
                        Observation
                    ),
                    (
                        ProviderInstance::Infrastructure(provider),
                        ProviderMethod::InfrastructureAdopt,
                    ) => dispatch_adoption!(provider, adopt, call.context, call.payload),
                    (
                        ProviderInstance::Infrastructure(provider),
                        ProviderMethod::InfrastructureBootstrapBinding,
                    ) => dispatch_operation!(
                        provider,
                        bootstrap_binding,
                        call.context,
                        call.payload,
                        Handle
                    ),
                    (
                        ProviderInstance::Infrastructure(provider),
                        ProviderMethod::InfrastructureDestroy,
                    ) => {
                        dispatch_operation!(provider, destroy, call.context, call.payload, Mutation)
                    }
                    (ProviderInstance::Transport(provider), ProviderMethod::TransportConnect) => {
                        dispatch_operation!(provider, connect, call.context, call.payload, Handle)
                    }
                    (ProviderInstance::Transport(provider), ProviderMethod::TransportListen) => {
                        dispatch_operation!(provider, listen, call.context, call.payload, Handle)
                    }
                    (
                        ProviderInstance::Transport(provider),
                        ProviderMethod::TransportIssueBinding,
                    ) => dispatch_operation!(
                        provider,
                        issue_binding,
                        call.context,
                        call.payload,
                        Handle
                    ),
                    (
                        ProviderInstance::Transport(provider),
                        ProviderMethod::TransportRevokeBinding,
                    ) => dispatch_operation!(
                        provider,
                        revoke_binding,
                        call.context,
                        call.payload,
                        Mutation
                    ),
                    (ProviderInstance::Transport(provider), ProviderMethod::TransportInspect) => {
                        dispatch_operation!(
                            provider,
                            inspect,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Substrate(provider), ProviderMethod::SubstrateCheck) => {
                        dispatch_operation!(
                            provider,
                            check,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (
                        ProviderInstance::Substrate(provider),
                        ProviderMethod::SubstratePlanRemediation,
                    ) => dispatch_operation!(
                        provider,
                        plan_remediation,
                        call.context,
                        call.payload,
                        Plan
                    ),
                    (ProviderInstance::Substrate(provider), ProviderMethod::SubstrateApply) => {
                        dispatch_plan!(provider, apply, call.context, call.payload, Mutation)
                    }
                    (ProviderInstance::Credential(provider), ProviderMethod::CredentialStatus) => {
                        dispatch_operation!(
                            provider,
                            status,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (
                        ProviderInstance::Credential(provider),
                        ProviderMethod::CredentialAcquireLease,
                    ) => match call.payload {
                        RpcPayload::LeaseRequest(request) => Some(
                            provider
                                .acquire_lease(call.context, request)
                                .await
                                .map(|value| RpcResponse::Lease(Box::new(value))),
                        ),
                        _ => None,
                    },
                    (
                        ProviderInstance::Credential(provider),
                        ProviderMethod::CredentialRefreshLease,
                    ) => match call.payload {
                        RpcPayload::Lease(lease) => Some(
                            provider
                                .refresh_lease(call.context, lease)
                                .await
                                .map(|value| RpcResponse::Lease(Box::new(value))),
                        ),
                        _ => None,
                    },
                    (
                        ProviderInstance::Credential(provider),
                        ProviderMethod::CredentialRevokeLease,
                    ) => match call.payload {
                        RpcPayload::Lease(lease) => Some(
                            provider
                                .revoke_lease(call.context, lease)
                                .await
                                .map(|value| RpcResponse::Mutation(Box::new(value))),
                        ),
                        _ => None,
                    },
                    (ProviderInstance::Display(provider), ProviderMethod::DisplayOpen) => {
                        dispatch_operation!(provider, open, call.context, call.payload, Handle)
                    }
                    (ProviderInstance::Display(provider), ProviderMethod::DisplayInspect) => {
                        dispatch_operation!(
                            provider,
                            inspect,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Display(provider), ProviderMethod::DisplayAdopt) => {
                        dispatch_adoption!(provider, adopt, call.context, call.payload)
                    }
                    (ProviderInstance::Display(provider), ProviderMethod::DisplayClose) => {
                        dispatch_operation!(provider, close, call.context, call.payload, Mutation)
                    }
                    (ProviderInstance::Network(provider), ProviderMethod::NetworkPlan) => {
                        dispatch_operation!(provider, plan, call.context, call.payload, Plan)
                    }
                    (ProviderInstance::Network(provider), ProviderMethod::NetworkEnsure) => {
                        dispatch_plan!(provider, ensure, call.context, call.payload, Handle)
                    }
                    (ProviderInstance::Network(provider), ProviderMethod::NetworkInspect) => {
                        dispatch_operation!(
                            provider,
                            inspect,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Network(provider), ProviderMethod::NetworkAdopt) => {
                        dispatch_adoption!(provider, adopt, call.context, call.payload)
                    }
                    (ProviderInstance::Network(provider), ProviderMethod::NetworkRelease) => {
                        dispatch_operation!(provider, release, call.context, call.payload, Mutation)
                    }
                    (ProviderInstance::Storage(provider), ProviderMethod::StoragePlan) => {
                        dispatch_operation!(provider, plan, call.context, call.payload, Plan)
                    }
                    (ProviderInstance::Storage(provider), ProviderMethod::StorageEnsure) => {
                        dispatch_plan!(provider, ensure, call.context, call.payload, Handle)
                    }
                    (ProviderInstance::Storage(provider), ProviderMethod::StorageInspect) => {
                        dispatch_operation!(
                            provider,
                            inspect,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Storage(provider), ProviderMethod::StorageAdopt) => {
                        dispatch_adoption!(provider, adopt, call.context, call.payload)
                    }
                    (ProviderInstance::Storage(provider), ProviderMethod::StorageSnapshot) => {
                        dispatch_operation!(provider, snapshot, call.context, call.payload, Handle)
                    }
                    (ProviderInstance::Storage(provider), ProviderMethod::StorageDestroy) => {
                        dispatch_operation!(provider, destroy, call.context, call.payload, Mutation)
                    }
                    (ProviderInstance::Device(provider), ProviderMethod::DevicePlanAttach) => {
                        dispatch_operation!(provider, plan_attach, call.context, call.payload, Plan)
                    }
                    (ProviderInstance::Device(provider), ProviderMethod::DeviceAttach) => {
                        dispatch_plan!(provider, attach, call.context, call.payload, Handle)
                    }
                    (ProviderInstance::Device(provider), ProviderMethod::DeviceInspect) => {
                        dispatch_operation!(
                            provider,
                            inspect,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Device(provider), ProviderMethod::DeviceAdopt) => {
                        dispatch_adoption!(provider, adopt, call.context, call.payload)
                    }
                    (ProviderInstance::Device(provider), ProviderMethod::DeviceDetach) => {
                        dispatch_operation!(provider, detach, call.context, call.payload, Mutation)
                    }
                    (ProviderInstance::Audio(provider), ProviderMethod::AudioOpen) => {
                        dispatch_operation!(provider, open, call.context, call.payload, Handle)
                    }
                    (ProviderInstance::Audio(provider), ProviderMethod::AudioSetState) => {
                        dispatch_operation!(
                            provider,
                            set_state,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Audio(provider), ProviderMethod::AudioInspect) => {
                        dispatch_operation!(
                            provider,
                            inspect,
                            call.context,
                            call.payload,
                            Observation
                        )
                    }
                    (ProviderInstance::Audio(provider), ProviderMethod::AudioAdopt) => {
                        dispatch_adoption!(provider, adopt, call.context, call.payload)
                    }
                    (ProviderInstance::Audio(provider), ProviderMethod::AudioClose) => {
                        dispatch_operation!(provider, close, call.context, call.payload, Mutation)
                    }
                    (
                        ProviderInstance::Observability(provider),
                        ProviderMethod::ObservabilityStatus,
                    ) => dispatch_operation!(
                        provider,
                        status,
                        call.context,
                        call.payload,
                        Observation
                    ),
                    (
                        ProviderInstance::Observability(provider),
                        ProviderMethod::ObservabilityQuery,
                    ) => dispatch_operation!(
                        provider,
                        query,
                        call.context,
                        call.payload,
                        Observation
                    ),
                    (
                        ProviderInstance::Observability(provider),
                        ProviderMethod::ObservabilitySubscribe,
                    ) => {
                        dispatch_operation!(provider, subscribe, call.context, call.payload, Handle)
                    }
                    (
                        ProviderInstance::Observability(provider),
                        ProviderMethod::ObservabilityExport,
                    ) => {
                        dispatch_operation!(provider, export, call.context, call.payload, Mutation)
                    }
                    _ => None,
                };
                result.unwrap_or_else(|| {
                    Err(self.failure(
                        &call,
                        ProviderFailureKind::CapabilityDenied,
                        ProviderHealthReason::CapabilityMismatch,
                    ))
                })
            }
        }
    }
}
