#![allow(clippy::result_large_err)]

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{RequestId, ServicePackage, SessionErrorCode},
    v2_identity::{ProviderType, RealmId, RoleId, WorkloadId},
    v2_provider::{
        AdoptionRequest, AuthorizedProviderScope, CorrelationId, CredentialLease,
        CredentialLeaseRequest, Fingerprint, IdempotencyKey, MutationState, ObservedLifecycleState,
        OperationId, PROVIDER_SCHEMA_VERSION, PrincipalRef, ProviderCapability, ProviderDescriptor,
        ProviderFailure, ProviderFailureKind, ProviderHandle, ProviderHealth, ProviderHealthState,
        ProviderMethod, ProviderObservation, ProviderOperationContext, ProviderOperationRequest,
        ProviderPlan, ProviderTarget, RetryClass,
    },
    v2_services::{
        ServiceContractError, StrictWireMessage, common, provider_audio_ttrpc,
        provider_credential_ttrpc, provider_device_ttrpc, provider_display_ttrpc,
        provider_infrastructure_ttrpc, provider_network_ttrpc, provider_observability_ttrpc,
        provider_operation_input, provider_runtime_ttrpc, provider_storage_ttrpc,
        provider_substrate_ttrpc, provider_transport_ttrpc, provider_type,
    },
};
use d2b_provider::{
    ProviderClock, ProviderInstance, RpcCall, RpcOperation, RpcPayload, RpcResponse,
    SessionIdentity, provider_capabilities_are_dispatchable, provider_inspection_method,
    provider_method_is_dispatchable,
};
use d2b_session::{
    Cancellation, ComponentSessionDriver, DeadlineBudget, OwnedAttachment, SessionDriverHandle,
    SessionError,
};
use protobuf::{Enum, EnumOrUnknown, MessageField};
use tokio::sync::Notify;

use crate::{ProviderAgentAdapter, ToolkitError};

#[derive(Default)]
struct ObjectStore {
    plans: BTreeMap<(u64, String), ProviderPlan>,
    handles: BTreeMap<(u64, String), ProviderHandle>,
    leases: BTreeMap<(u64, String), CredentialLease>,
}

const MAX_SESSION_PLANS: usize = 256;
const MAX_SESSION_HANDLES: usize = 1_024;
const MAX_SESSION_LEASES: usize = 1_024;
const MAX_AGENT_IN_FLIGHT: usize = 64;

pub struct GeneratedProviderServiceServer {
    adapter: Arc<ProviderAgentAdapter>,
    driver: Arc<dyn ComponentSessionDriver>,
    clock: Arc<dyn ProviderClock>,
    descriptor: ProviderDescriptor,
    objects: Mutex<ObjectStore>,
    accepting: AtomicBool,
    in_flight: AtomicUsize,
    idle: Notify,
}

impl std::fmt::Debug for GeneratedProviderServiceServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedProviderServiceServer")
            .field("provider_type", &self.descriptor.provider_type())
            .field("generation", &self.descriptor.registry_generation)
            .finish_non_exhaustive()
    }
}

impl GeneratedProviderServiceServer {
    pub fn from_session_handle(
        instance: ProviderInstance,
        driver: SessionDriverHandle,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, ToolkitError> {
        Self::new(instance, Arc::new(driver), clock)
    }

    pub fn new(
        instance: ProviderInstance,
        driver: Arc<dyn ComponentSessionDriver>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, ToolkitError> {
        let descriptor = instance.descriptor();
        if descriptor.placement.agent_binding().is_none() || driver.generation() == 0 {
            return Err(ToolkitError::DescriptorInvalid);
        }
        if !provider_capabilities_are_dispatchable(&instance.capabilities()) {
            return Err(ToolkitError::CapabilityMismatch);
        }
        let identity = SessionIdentity {
            peer_role: d2b_contracts::v2_component_session::EndpointRole::ProviderAgent,
            service: ServicePackage::ProviderV2,
            provider_id: descriptor.provider_id.clone(),
            provider_type: descriptor.provider_type(),
            provider_generation: descriptor.registry_generation,
        };
        let adapter = Arc::new(ProviderAgentAdapter::new(
            instance,
            identity,
            clock.clone(),
        )?);
        Ok(Self {
            adapter,
            driver,
            clock,
            descriptor,
            objects: Mutex::new(ObjectStore::default()),
            accepting: AtomicBool::new(true),
            in_flight: AtomicUsize::new(0),
            idle: Notify::new(),
        })
    }

    pub async fn shutdown(&self, timeout: Duration) -> bool {
        self.accepting.store(false, Ordering::Release);
        tokio::time::timeout(timeout, async {
            loop {
                let notified = self.idle.notified();
                if self.in_flight.load(Ordering::Acquire) == 0 {
                    break;
                }
                notified.await;
            }
        })
        .await
        .is_ok()
    }

    pub fn generated_services(self: &Arc<Self>) -> HashMap<String, ttrpc::r#async::Service> {
        match self.descriptor.provider_type() {
            ProviderType::Runtime => {
                provider_runtime_ttrpc::create_runtime_provider_service(self.clone())
            }
            ProviderType::Infrastructure => {
                provider_infrastructure_ttrpc::create_infrastructure_provider_service(self.clone())
            }
            ProviderType::Transport => {
                provider_transport_ttrpc::create_transport_provider_service(self.clone())
            }
            ProviderType::Substrate => {
                provider_substrate_ttrpc::create_substrate_provider_service(self.clone())
            }
            ProviderType::Credential => {
                provider_credential_ttrpc::create_credential_provider_service(self.clone())
            }
            ProviderType::Display => {
                provider_display_ttrpc::create_display_provider_service(self.clone())
            }
            ProviderType::Network => {
                provider_network_ttrpc::create_network_provider_service(self.clone())
            }
            ProviderType::Storage => {
                provider_storage_ttrpc::create_storage_provider_service(self.clone())
            }
            ProviderType::Device => {
                provider_device_ttrpc::create_device_provider_service(self.clone())
            }
            ProviderType::Audio => {
                provider_audio_ttrpc::create_audio_provider_service(self.clone())
            }
            ProviderType::Observability => {
                provider_observability_ttrpc::create_observability_provider_service(self.clone())
            }
        }
    }

    async fn capability_call(
        &self,
        request: common::CapabilityRequest,
        ttrpc_timeout_nanos: Option<u64>,
    ) -> ttrpc::Result<common::CapabilityResponse> {
        let _admission = self.admit_request()?;
        request
            .validate_wire(false)
            .map_err(invalid_request_contract)?;
        let wire_context = request
            .context
            .as_ref()
            .ok_or_else(|| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
        let admitted = self.admit_context(
            wire_context,
            wire_context_method(wire_context)?,
            false,
            ttrpc_timeout_nanos,
        )?;
        let inbound =
            InboundCallRegistration::register(self.driver.clone(), admitted.request_id.clone())
                .await?;
        let call_context = admitted.call_context();
        let response = tokio::select! {
            biased;
            () = inbound.cancellation().cancelled() => {
                Err(rpc_status(ttrpc::Code::CANCELLED))
            }
            response = self.adapter.invoke_session(
                RpcCall {
                    operation: RpcOperation::Capabilities,
                    context: &call_context,
                    payload: RpcPayload::None,
                },
                &mut [],
            ) => Ok(response),
        };
        match inbound.finish(response).await? {
            Ok(RpcResponse::Capabilities(capabilities)) => {
                let mut wire = common::CapabilityResponse::new();
                wire.capabilities = capabilities
                    .as_slice()
                    .iter()
                    .map(|capability| capability_to_wire(*capability))
                    .collect::<Result<_, _>>()?;
                wire.provider_generation = self.descriptor.registry_generation.get();
                wire.descriptor_digest =
                    decode_fingerprint(&self.descriptor.configured_scope_digest)?;
                wire.validate_wire(false)
                    .map_err(invalid_response_contract)?;
                Ok(wire)
            }
            Ok(_) => Err(rpc_status(ttrpc::Code::INTERNAL)),
            Err(failure) => {
                let wire = capability_failure(&failure);
                wire.validate_wire(false)
                    .map_err(invalid_response_contract)?;
                Ok(wire)
            }
        }
    }

    async fn provider_call(
        &self,
        operation: RpcOperation,
        request: common::ProviderRequest,
        ttrpc_timeout_nanos: Option<u64>,
    ) -> ttrpc::Result<common::ProviderResponse> {
        let _admission = self.admit_request()?;
        let method = match operation {
            RpcOperation::Method(method) => method,
            RpcOperation::Health => {
                let context = request
                    .context
                    .as_ref()
                    .ok_or_else(|| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
                wire_context_method(context)?
            }
            RpcOperation::Capabilities => return Err(rpc_status(ttrpc::Code::INVALID_ARGUMENT)),
        };
        if !provider_method_is_dispatchable(method) {
            return Err(rpc_status(ttrpc::Code::FAILED_PRECONDITION));
        }
        let requires_idempotency = matches!(operation, RpcOperation::Method(method) if method_requires_idempotency(method));
        request
            .validate_wire(requires_idempotency)
            .map_err(invalid_request_contract)?;
        let wire_context = request
            .context
            .as_ref()
            .ok_or_else(|| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
        let admitted = self.admit_context(
            wire_context,
            method,
            requires_idempotency,
            ttrpc_timeout_nanos,
        )?;
        let inbound =
            InboundCallRegistration::register(self.driver.clone(), admitted.request_id.clone())
                .await?;
        let response = tokio::select! {
            biased;
            () = inbound.cancellation().cancelled() => {
                Err(rpc_status(ttrpc::Code::CANCELLED))
            }
            response = self.dispatch_provider_call(operation, method, request, &admitted) => response,
        };
        inbound.finish(response).await
    }

    async fn dispatch_provider_call(
        &self,
        operation: RpcOperation,
        method: ProviderMethod,
        request: common::ProviderRequest,
        admitted: &AdmittedWireContext,
    ) -> ttrpc::Result<common::ProviderResponse> {
        let mut attachments = if request.attachment_indexes.is_empty() {
            Vec::new()
        } else {
            self.driver
                .receive_attachments()
                .await
                .map_err(session_error)?
        };
        validate_attachments(&request.attachment_indexes, &attachments)?;
        let canonical_request = self.canonical_request(
            &request,
            admitted.operation.clone(),
            admitted.session_generation,
        )?;
        let payload = self.payload_for(
            method,
            &request,
            &canonical_request,
            admitted.session_generation,
            &attachments,
        )?;
        let rpc_payload = match &payload {
            OwnedDispatchPayload::Operation => RpcPayload::Operation(&canonical_request),
            OwnedDispatchPayload::Plan(plan) => RpcPayload::Plan(plan),
            OwnedDispatchPayload::Adoption(adoption) => RpcPayload::Adoption(adoption),
            OwnedDispatchPayload::LeaseRequest(request) => RpcPayload::LeaseRequest(request),
            OwnedDispatchPayload::Lease(lease) => RpcPayload::Lease(lease),
        };
        let call_context = admitted.call_context();
        let response = self
            .adapter
            .invoke_session(
                RpcCall {
                    operation,
                    context: &call_context,
                    payload: rpc_payload,
                },
                &mut attachments,
            )
            .await;
        self.response_to_wire(&canonical_request, admitted.session_generation, response)
    }

    fn canonical_request(
        &self,
        request: &common::ProviderRequest,
        context: ProviderOperationContext,
        session_generation: u64,
    ) -> ttrpc::Result<ProviderOperationRequest> {
        let target = if request.resource_id.is_empty() {
            target_from_scope(&context.scope)
        } else {
            let objects = self
                .objects
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(handle) = objects
                .handles
                .get(&(session_generation, request.resource_id.clone()))
            {
                ProviderTarget::Handle {
                    realm_id: handle.realm_id.clone(),
                    workload_id: handle.workload_id.clone(),
                    handle_id: handle.handle_id.clone(),
                    handle_generation: handle.resource_generation,
                }
            } else {
                target_from_scope(&context.scope)
            }
        };
        let canonical = ProviderOperationRequest {
            context,
            target,
            expected_configuration_fingerprint: self
                .descriptor
                .configuration_schema_fingerprint
                .clone(),
            input: provider_operation_input(
                request
                    .input
                    .as_ref()
                    .ok_or_else(|| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
            )
            .map_err(invalid_request_contract)?,
        };
        canonical
            .validate(&self.descriptor, self.clock.now_unix_ms())
            .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
        Ok(canonical)
    }

    fn payload_for(
        &self,
        method: ProviderMethod,
        wire: &common::ProviderRequest,
        request: &ProviderOperationRequest,
        session_generation: u64,
        attachments: &[OwnedAttachment],
    ) -> ttrpc::Result<OwnedDispatchPayload> {
        match method {
            ProviderMethod::RuntimeEnsure
            | ProviderMethod::InfrastructureApply
            | ProviderMethod::SubstrateApply
            | ProviderMethod::NetworkEnsure
            | ProviderMethod::StorageEnsure
            | ProviderMethod::DeviceAttach => {
                let plan = self
                    .objects
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .plans
                    .get(&(session_generation, wire.resource_id.clone()))
                    .cloned()
                    .ok_or_else(|| rpc_status(ttrpc::Code::FAILED_PRECONDITION))?;
                Ok(OwnedDispatchPayload::Plan(Box::new(plan)))
            }
            ProviderMethod::RuntimeAdopt
            | ProviderMethod::InfrastructureAdopt
            | ProviderMethod::DisplayAdopt
            | ProviderMethod::NetworkAdopt
            | ProviderMethod::StorageAdopt
            | ProviderMethod::DeviceAdopt
            | ProviderMethod::AudioAdopt => {
                let handle = self
                    .objects
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .handles
                    .get(&(session_generation, wire.resource_id.clone()))
                    .cloned()
                    .ok_or_else(|| rpc_status(ttrpc::Code::FAILED_PRECONDITION))?;
                let adoption = AdoptionRequest {
                    context: request.context.clone(),
                    expected_owner: handle.owner.clone(),
                    expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
                    expected_resource_generation: handle.resource_generation,
                    handle,
                };
                Ok(OwnedDispatchPayload::Adoption(Box::new(adoption)))
            }
            ProviderMethod::CredentialAcquireLease => {
                let [attachment] = attachments else {
                    return Err(rpc_status(ttrpc::Code::INVALID_ARGUMENT));
                };
                let bytes = attachment
                    .payload()
                    .and_then(|payload| payload.downcast_ref::<Vec<u8>>())
                    .ok_or_else(|| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
                let mut lease_request: CredentialLeaseRequest = serde_json::from_slice(bytes)
                    .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
                lease_request.context = request.context.clone();
                Ok(OwnedDispatchPayload::LeaseRequest(Box::new(lease_request)))
            }
            ProviderMethod::CredentialRefreshLease | ProviderMethod::CredentialRevokeLease => {
                if !attachments.is_empty() {
                    return Err(rpc_status(ttrpc::Code::INVALID_ARGUMENT));
                }
                let lease = self
                    .objects
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .leases
                    .get(&(session_generation, wire.resource_id.clone()))
                    .cloned()
                    .ok_or_else(|| rpc_status(ttrpc::Code::FAILED_PRECONDITION))?;
                Ok(OwnedDispatchPayload::Lease(Box::new(lease)))
            }
            _ => Ok(OwnedDispatchPayload::Operation),
        }
    }

    fn response_to_wire(
        &self,
        request: &ProviderOperationRequest,
        session_generation: u64,
        response: Result<RpcResponse, ProviderFailure>,
    ) -> ttrpc::Result<common::ProviderResponse> {
        let mut wire = common::ProviderResponse::new();
        wire.operation_id = request.context.operation_id.as_str().to_owned();
        match response {
            Ok(RpcResponse::Health(health)) => {
                wire.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
                wire.observations.push(health_to_wire(&health)?);
            }
            Ok(RpcResponse::Plan(plan)) => {
                wire.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
                wire.resource_handle = plan.plan_id.as_str().to_owned();
                let mut objects = self
                    .objects
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let key = (session_generation, wire.resource_handle.clone());
                if !objects.plans.contains_key(&key) && objects.plans.len() >= MAX_SESSION_PLANS {
                    return Err(rpc_status(ttrpc::Code::RESOURCE_EXHAUSTED));
                }
                objects.plans.insert(key, *plan);
            }
            Ok(RpcResponse::Handle(handle)) => {
                wire.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
                wire.resource_handle = handle.handle_id.as_str().to_owned();
                let mut objects = self
                    .objects
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let key = (session_generation, wire.resource_handle.clone());
                if !objects.handles.contains_key(&key)
                    && objects.handles.len() >= MAX_SESSION_HANDLES
                {
                    return Err(rpc_status(ttrpc::Code::RESOURCE_EXHAUSTED));
                }
                objects.handles.insert(key, *handle);
            }
            Ok(RpcResponse::Observation(observation)) => {
                wire.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
                wire.observations.push(observation_to_wire(&observation)?);
            }
            Ok(RpcResponse::Mutation(receipt)) => {
                wire.outcome = EnumOrUnknown::new(match receipt.state {
                    MutationState::Applied | MutationState::AlreadyApplied => {
                        common::Outcome::OUTCOME_SUCCEEDED
                    }
                    MutationState::NotApplicable => common::Outcome::OUTCOME_NOT_APPLICABLE,
                    MutationState::CancelledBeforeMutation => {
                        let response = provider_failure_response(
                            request,
                            ProviderFailureKind::Cancelled,
                            RetryClass::Never,
                        );
                        response
                            .validate_wire(false)
                            .map_err(invalid_response_contract)?;
                        return Ok(response);
                    }
                    MutationState::CompletionAmbiguous => common::Outcome::OUTCOME_DEGRADED,
                });
            }
            Ok(RpcResponse::Lease(lease)) => {
                wire.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
                wire.resource_handle = lease.lease_id.as_str().to_owned();
                let mut objects = self
                    .objects
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let key = (session_generation, wire.resource_handle.clone());
                if !objects.leases.contains_key(&key) && objects.leases.len() >= MAX_SESSION_LEASES
                {
                    return Err(rpc_status(ttrpc::Code::RESOURCE_EXHAUSTED));
                }
                objects.leases.insert(key, *lease);
            }
            Ok(RpcResponse::Capabilities(_)) => return Err(rpc_status(ttrpc::Code::INTERNAL)),
            Err(failure) => {
                let response = failure_to_wire(request, &failure);
                response
                    .validate_wire(false)
                    .map_err(invalid_response_contract)?;
                return Ok(response);
            }
        }
        wire.result_digest = decode_fingerprint(&self.descriptor.configuration_schema_fingerprint)?;
        wire.validate_wire(false)
            .map_err(invalid_response_contract)?;
        Ok(wire)
    }

    fn admit_context(
        &self,
        wire: &common::ProviderOperationContext,
        method: ProviderMethod,
        requires_idempotency: bool,
        ttrpc_timeout_nanos: Option<u64>,
    ) -> ttrpc::Result<AdmittedWireContext> {
        let metadata = wire
            .metadata
            .as_ref()
            .ok_or_else(|| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
        let session_generation = self.driver.generation();
        let now = Instant::now();
        let deadline = DeadlineBudget::admit_metadata(
            metadata,
            session_generation,
            requires_idempotency,
            self.clock.now_unix_ms(),
            now,
            d2b_contracts::v2_provider::MAX_PROVIDER_REQUEST_LIFETIME_MS,
            ttrpc_timeout_nanos,
        )
        .map_err(session_error)?;
        let remaining_nanos = deadline
            .remaining_nanos(self.clock.now_unix_ms(), now, ttrpc_timeout_nanos)
            .map_err(session_error)?;
        let request_id = RequestId::new(metadata.request_id.clone())
            .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
        let scope = scope_from_wire(wire)?;
        if wire.provider_id != self.descriptor.provider_id.as_str()
            || scope.realm_id() != self.descriptor.placement.realm_id()
        {
            return Err(rpc_status(ttrpc::Code::PERMISSION_DENIED));
        }
        let operation = canonical_context(wire, metadata, method, &self.descriptor, scope)?;
        operation
            .validate(&self.descriptor, self.clock.now_unix_ms())
            .map_err(|_| rpc_status(ttrpc::Code::PERMISSION_DENIED))?;
        let remaining_ms = u32::try_from((remaining_nanos / 1_000_000).max(1)).unwrap_or(u32::MAX);
        Ok(AdmittedWireContext {
            operation,
            request_id,
            remaining_ms,
            session_generation,
        })
    }

    fn admit_request(&self) -> ttrpc::Result<AgentAdmission<'_>> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(rpc_status(ttrpc::Code::UNAVAILABLE));
        }
        let previous = self.in_flight.fetch_add(1, Ordering::AcqRel);
        if previous >= MAX_AGENT_IN_FLIGHT || !self.accepting.load(Ordering::Acquire) {
            self.release_request();
            return Err(rpc_status(if previous >= MAX_AGENT_IN_FLIGHT {
                ttrpc::Code::RESOURCE_EXHAUSTED
            } else {
                ttrpc::Code::UNAVAILABLE
            }));
        }
        Ok(AgentAdmission { server: self })
    }

    fn release_request(&self) {
        if self.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle.notify_one();
        }
    }
}

struct AgentAdmission<'a> {
    server: &'a GeneratedProviderServiceServer,
}

impl Drop for AgentAdmission<'_> {
    fn drop(&mut self) {
        self.server.release_request();
    }
}

enum OwnedDispatchPayload {
    Operation,
    Plan(Box<ProviderPlan>),
    Adoption(Box<AdoptionRequest>),
    LeaseRequest(Box<CredentialLeaseRequest>),
    Lease(Box<CredentialLease>),
}

struct AdmittedWireContext {
    operation: ProviderOperationContext,
    request_id: RequestId,
    remaining_ms: u32,
    session_generation: u64,
}

impl AdmittedWireContext {
    fn call_context(&self) -> d2b_contracts::v2_provider::ProviderCallContext<'_> {
        d2b_contracts::v2_provider::ProviderCallContext {
            operation: &self.operation,
            peer_role: d2b_contracts::v2_component_session::EndpointRole::RealmController,
            service: ServicePackage::ProviderV2,
            monotonic_deadline_remaining_ms: self.remaining_ms,
            cancelled: false,
        }
    }
}

struct InboundCallRegistration {
    driver: Arc<dyn ComponentSessionDriver>,
    request_id: Option<RequestId>,
    cancellation: Cancellation,
}

impl InboundCallRegistration {
    async fn register(
        driver: Arc<dyn ComponentSessionDriver>,
        request_id: RequestId,
    ) -> ttrpc::Result<Self> {
        let cancellation = driver
            .register_inbound_call(request_id.clone())
            .await
            .map_err(session_error)?;
        Ok(Self {
            driver,
            request_id: Some(request_id),
            cancellation,
        })
    }

    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }

    async fn finish<T>(self, result: ttrpc::Result<T>) -> ttrpc::Result<T> {
        match result {
            Ok(response) => {
                self.complete().await?;
                Ok(response)
            }
            Err(error) => {
                self.remove().await?;
                Err(error)
            }
        }
    }

    async fn complete(mut self) -> ttrpc::Result<()> {
        let request_id = self
            .request_id
            .take()
            .ok_or_else(|| rpc_status(ttrpc::Code::INTERNAL))?;
        if self
            .driver
            .complete_inbound_call(request_id)
            .await
            .map_err(session_error)?
        {
            Ok(())
        } else {
            Err(rpc_status(ttrpc::Code::INTERNAL))
        }
    }

    async fn remove(mut self) -> ttrpc::Result<()> {
        let request_id = self
            .request_id
            .take()
            .ok_or_else(|| rpc_status(ttrpc::Code::INTERNAL))?;
        if self
            .driver
            .remove_inbound_call(request_id)
            .await
            .map_err(session_error)?
        {
            Ok(())
        } else {
            Err(rpc_status(ttrpc::Code::INTERNAL))
        }
    }
}

impl Drop for InboundCallRegistration {
    fn drop(&mut self) {
        let Some(request_id) = self.request_id.take() else {
            return;
        };
        let driver = self.driver.clone();
        tokio::spawn(async move {
            let _ = driver.remove_inbound_call(request_id).await;
        });
    }
}

fn scope_from_wire(
    wire: &common::ProviderOperationContext,
) -> ttrpc::Result<AuthorizedProviderScope> {
    let scope = wire
        .scope
        .as_ref()
        .ok_or_else(|| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
    let realm_id = RealmId::parse(scope.realm_id.clone())
        .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
    if scope.workload_id.is_empty() && scope.role_id.is_empty() {
        Ok(AuthorizedProviderScope::Realm { realm_id })
    } else if scope.role_id.is_empty() {
        Ok(AuthorizedProviderScope::Workload {
            realm_id,
            workload_id: WorkloadId::parse(scope.workload_id.clone())
                .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
        })
    } else {
        Ok(AuthorizedProviderScope::WorkloadRole {
            realm_id,
            workload_id: WorkloadId::parse(scope.workload_id.clone())
                .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
            role_id: RoleId::parse(scope.role_id.clone())
                .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
        })
    }
}

fn canonical_context(
    wire: &common::ProviderOperationContext,
    metadata: &common::RequestMetadata,
    method: ProviderMethod,
    descriptor: &ProviderDescriptor,
    scope: AuthorizedProviderScope,
) -> ttrpc::Result<ProviderOperationContext> {
    if provider_type(wire).map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?
        != descriptor.provider_type()
        || wire.provider_generation != descriptor.registry_generation.get()
    {
        return Err(rpc_status(ttrpc::Code::FAILED_PRECONDITION));
    }
    let request_hex = hex(&metadata.request_id);
    let idempotency = if metadata.idempotency_key.is_empty() {
        format!("request-{request_hex}")
    } else if metadata.idempotency_key.len() <= 32 {
        alpha_hex(&metadata.idempotency_key)
    } else {
        return Err(rpc_status(ttrpc::Code::INVALID_ARGUMENT));
    };
    let correlation = if metadata.correlation_id.is_empty() {
        format!("request-{request_hex}")
    } else {
        metadata.correlation_id.clone()
    };
    let trace = if metadata.trace_id.len() == 16 {
        let trace = hex(&metadata.trace_id);
        format!("{trace}{trace}")
    } else {
        return Err(rpc_status(ttrpc::Code::INVALID_ARGUMENT));
    };
    Ok(ProviderOperationContext {
        schema_version: PROVIDER_SCHEMA_VERSION,
        operation_id: OperationId::parse(wire.operation_id.clone())
            .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
        idempotency_key: IdempotencyKey::parse(idempotency)
            .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
        request_digest: fingerprint_from_bytes(&wire.request_digest)?,
        scope,
        principal: PrincipalRef::parse("component-session-peer")
            .map_err(|_| rpc_status(ttrpc::Code::INTERNAL))?,
        provider_id: descriptor.provider_id.clone(),
        provider_type: descriptor.provider_type(),
        provider_generation: descriptor.registry_generation,
        capability: ProviderCapability(method),
        method,
        policy_epoch: d2b_contracts::v2_provider::Generation::new(wire.policy_epoch)
            .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
        authorization_decision_digest: fingerprint_from_bytes(&wire.authorization_digest)?,
        issued_at_unix_ms: metadata.issued_at_unix_ms,
        expires_at_unix_ms: metadata.expires_at_unix_ms,
        correlation_id: CorrelationId::parse(correlation)
            .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
        trace_id: Fingerprint::parse(trace)
            .map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?,
    })
}

fn target_from_scope(scope: &AuthorizedProviderScope) -> ProviderTarget {
    match scope {
        AuthorizedProviderScope::Realm { realm_id } => ProviderTarget::Realm {
            realm_id: realm_id.clone(),
        },
        AuthorizedProviderScope::Workload {
            realm_id,
            workload_id,
        }
        | AuthorizedProviderScope::WorkloadRole {
            realm_id,
            workload_id,
            ..
        } => ProviderTarget::Workload {
            realm_id: realm_id.clone(),
            workload_id: workload_id.clone(),
        },
    }
}

fn validate_attachments(indexes: &[u32], attachments: &[OwnedAttachment]) -> ttrpc::Result<()> {
    if indexes.len() == attachments.len()
        && indexes.iter().zip(attachments).all(|(index, attachment)| {
            attachment
                .descriptor()
                .is_some_and(|descriptor| *index == u32::from(descriptor.index))
        })
    {
        Ok(())
    } else {
        Err(rpc_status(ttrpc::Code::INVALID_ARGUMENT))
    }
}

fn wire_context_method(
    context: &common::ProviderOperationContext,
) -> ttrpc::Result<ProviderMethod> {
    let provider_type =
        provider_type(context).map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))?;
    Ok(provider_inspection_method(provider_type))
}

fn method_requires_idempotency(method: ProviderMethod) -> bool {
    !matches!(
        method,
        ProviderMethod::RuntimePlan
            | ProviderMethod::RuntimeInspect
            | ProviderMethod::InfrastructurePlan
            | ProviderMethod::InfrastructureInspect
            | ProviderMethod::TransportInspect
            | ProviderMethod::SubstrateCheck
            | ProviderMethod::SubstratePlanRemediation
            | ProviderMethod::CredentialStatus
            | ProviderMethod::DisplayInspect
            | ProviderMethod::NetworkPlan
            | ProviderMethod::NetworkInspect
            | ProviderMethod::StoragePlan
            | ProviderMethod::StorageInspect
            | ProviderMethod::DevicePlanAttach
            | ProviderMethod::DeviceInspect
            | ProviderMethod::AudioInspect
            | ProviderMethod::ObservabilityStatus
            | ProviderMethod::ObservabilityQuery
            | ProviderMethod::ObservabilityExport
    )
}

fn capability_to_wire(
    capability: ProviderCapability,
) -> ttrpc::Result<EnumOrUnknown<common::ProviderCapability>> {
    let index = ProviderMethod::ALL
        .iter()
        .position(|method| *method == capability.0)
        .ok_or_else(|| rpc_status(ttrpc::Code::INTERNAL))?;
    let value = i32::try_from(index + 1).map_err(|_| rpc_status(ttrpc::Code::INTERNAL))?;
    common::ProviderCapability::from_i32(value)
        .map(EnumOrUnknown::new)
        .ok_or_else(|| rpc_status(ttrpc::Code::INTERNAL))
}

fn health_to_wire(health: &ProviderHealth) -> ttrpc::Result<common::Observation> {
    health
        .validate()
        .map_err(|_| rpc_status(ttrpc::Code::INTERNAL))?;
    let state = match health.state {
        ProviderHealthState::Healthy => common::ObservationState::OBSERVATION_STATE_READY,
        ProviderHealthState::Degraded => common::ObservationState::OBSERVATION_STATE_DEGRADED,
        ProviderHealthState::Unavailable => common::ObservationState::OBSERVATION_STATE_UNAVAILABLE,
        ProviderHealthState::Failed => common::ObservationState::OBSERVATION_STATE_FAILED,
    };
    Ok(common::Observation {
        state: EnumOrUnknown::new(state),
        generation: health.registry_generation.get(),
        digest: vec![1; 32],
        ..Default::default()
    })
}

fn observation_to_wire(observation: &ProviderObservation) -> ttrpc::Result<common::Observation> {
    observation
        .validate()
        .map_err(|_| rpc_status(ttrpc::Code::INTERNAL))?;
    let state = match observation.lifecycle {
        ObservedLifecycleState::Planned => common::ObservationState::OBSERVATION_STATE_PENDING,
        ObservedLifecycleState::Ready => common::ObservationState::OBSERVATION_STATE_READY,
        ObservedLifecycleState::Running => common::ObservationState::OBSERVATION_STATE_RUNNING,
        ObservedLifecycleState::Stopped => common::ObservationState::OBSERVATION_STATE_STOPPED,
        ObservedLifecycleState::Released | ObservedLifecycleState::Destroyed => {
            common::ObservationState::OBSERVATION_STATE_ABSENT
        }
        ObservedLifecycleState::Unknown => common::ObservationState::OBSERVATION_STATE_UNAVAILABLE,
        ObservedLifecycleState::Quarantined => common::ObservationState::OBSERVATION_STATE_FAILED,
    };
    Ok(common::Observation {
        resource_id: observation
            .handle_id
            .as_ref()
            .map_or_else(String::new, |handle| handle.as_str().to_owned()),
        state: EnumOrUnknown::new(state),
        generation: observation
            .resource_generation
            .unwrap_or(observation.provider_generation)
            .get(),
        digest: vec![1; 32],
        ..Default::default()
    })
}

fn failure_to_wire(
    request: &ProviderOperationRequest,
    failure: &ProviderFailure,
) -> common::ProviderResponse {
    let mut response = provider_failure_response(request, failure.kind, failure.retry);
    if let Some(error) = response.error.as_mut() {
        error.correlation_id = failure.correlation_id.as_str().to_owned();
    }
    response
}

fn provider_failure_response(
    request: &ProviderOperationRequest,
    kind: ProviderFailureKind,
    retry: RetryClass,
) -> common::ProviderResponse {
    common::ProviderResponse {
        outcome: EnumOrUnknown::new(match kind {
            ProviderFailureKind::Cancelled | ProviderFailureKind::DeadlineExpired => {
                common::Outcome::OUTCOME_CANCELLED
            }
            _ => common::Outcome::OUTCOME_FAILED,
        }),
        operation_id: request.context.operation_id.as_str().to_owned(),
        error: MessageField::some(common::ErrorEnvelope {
            kind: EnumOrUnknown::new(failure_kind(kind)),
            retry: EnumOrUnknown::new(retry_class(retry)),
            correlation_id: request.context.correlation_id.as_str().to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn capability_failure(failure: &ProviderFailure) -> common::CapabilityResponse {
    common::CapabilityResponse {
        error: MessageField::some(common::ErrorEnvelope {
            kind: EnumOrUnknown::new(failure_kind(failure.kind)),
            retry: EnumOrUnknown::new(retry_class(failure.retry)),
            correlation_id: failure.correlation_id.as_str().to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn failure_kind(kind: ProviderFailureKind) -> common::ErrorKind {
    match kind {
        ProviderFailureKind::CapabilityDenied => common::ErrorKind::ERROR_KIND_CAPABILITY_DENIED,
        ProviderFailureKind::InvalidRequest => common::ErrorKind::ERROR_KIND_INVALID_REQUEST,
        ProviderFailureKind::UnauthorizedScope => common::ErrorKind::ERROR_KIND_UNAUTHORIZED,
        ProviderFailureKind::Cancelled => common::ErrorKind::ERROR_KIND_CANCELLED,
        ProviderFailureKind::DeadlineExpired => common::ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
        ProviderFailureKind::Unavailable => common::ErrorKind::ERROR_KIND_UNAVAILABLE,
        ProviderFailureKind::InvariantViolation | ProviderFailureKind::AdoptionRejected => {
            common::ErrorKind::ERROR_KIND_INVARIANT_VIOLATION
        }
        ProviderFailureKind::AmbiguousMutation => common::ErrorKind::ERROR_KIND_CONFLICT,
        ProviderFailureKind::RegistryChanged => common::ErrorKind::ERROR_KIND_GENERATION_MISMATCH,
        ProviderFailureKind::CredentialLeaseInvalid => {
            common::ErrorKind::ERROR_KIND_INVALID_REQUEST
        }
    }
}

fn retry_class(retry: RetryClass) -> common::RetryClass {
    match retry {
        RetryClass::Never => common::RetryClass::RETRY_CLASS_NEVER,
        RetryClass::SameOperation => common::RetryClass::RETRY_CLASS_SAME_OPERATION,
        RetryClass::AfterObservation => common::RetryClass::RETRY_CLASS_AFTER_OBSERVATION,
        RetryClass::AfterInteraction => common::RetryClass::RETRY_CLASS_AFTER_INTERACTION,
    }
}

fn fingerprint_from_bytes(value: &[u8]) -> ttrpc::Result<Fingerprint> {
    if value.len() != 32 {
        return Err(rpc_status(ttrpc::Code::INVALID_ARGUMENT));
    }
    Fingerprint::parse(hex(value)).map_err(|_| rpc_status(ttrpc::Code::INVALID_ARGUMENT))
}

fn decode_fingerprint(value: &Fingerprint) -> ttrpc::Result<Vec<u8>> {
    value
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| rpc_status(ttrpc::Code::INTERNAL))?;
            u8::from_str_radix(text, 16).map_err(|_| rpc_status(ttrpc::Code::INTERNAL))
        })
        .collect()
}

fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn alpha_hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"abcdefghijklmnop";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn session_error(error: SessionError) -> ttrpc::Error {
    let code = match error.code() {
        SessionErrorCode::GenerationMismatch => ttrpc::Code::FAILED_PRECONDITION,
        SessionErrorCode::DeadlineInvalid | SessionErrorCode::DeadlineExpired => {
            ttrpc::Code::DEADLINE_EXCEEDED
        }
        SessionErrorCode::Cancelled => ttrpc::Code::CANCELLED,
        SessionErrorCode::AttachmentDescriptorMismatch
        | SessionErrorCode::AttachmentObjectMismatch
        | SessionErrorCode::AttachmentAccessMismatch
        | SessionErrorCode::AttachmentMissingCloexec => ttrpc::Code::INVALID_ARGUMENT,
        _ => ttrpc::Code::UNAVAILABLE,
    };
    rpc_status_with_reason(code, error.code().as_str())
}

fn rpc_status(code: ttrpc::Code) -> ttrpc::Error {
    let reason = match code {
        ttrpc::Code::INVALID_ARGUMENT => "provider request invalid",
        ttrpc::Code::FAILED_PRECONDITION => "provider precondition failed",
        ttrpc::Code::RESOURCE_EXHAUSTED => "provider resource limit exceeded",
        ttrpc::Code::PERMISSION_DENIED => "provider authorization denied",
        ttrpc::Code::DEADLINE_EXCEEDED => "provider request deadline exceeded",
        ttrpc::Code::CANCELLED => "provider request cancelled",
        ttrpc::Code::UNAVAILABLE => "provider unavailable",
        _ => "provider internal invariant failed",
    };
    rpc_status_with_reason(code, reason)
}

fn invalid_request_contract(error: ServiceContractError) -> ttrpc::Error {
    rpc_status_with_reason(ttrpc::Code::INVALID_ARGUMENT, error.to_string())
}

fn invalid_response_contract(error: ServiceContractError) -> ttrpc::Error {
    rpc_status_with_reason(ttrpc::Code::INTERNAL, error.to_string())
}

fn rpc_status_with_reason(code: ttrpc::Code, reason: impl Into<String>) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, reason.into()))
}

fn ttrpc_timeout(context: &ttrpc::r#async::TtrpcContext) -> Option<u64> {
    u64::try_from(context.timeout_nano)
        .ok()
        .filter(|value| *value > 0)
}

macro_rules! provider_service {
    (
        $trait:path,
        $health:ident,
        $capabilities:ident,
        {$($name:ident => $method:ident),+ $(,)?}
    ) => {
        #[async_trait]
        impl $trait for GeneratedProviderServiceServer {
            async fn $health(
                &self,
                context: &ttrpc::r#async::TtrpcContext,
                request: common::ProviderRequest,
            ) -> ttrpc::Result<common::ProviderResponse> {
                self.provider_call(
                    RpcOperation::Health,
                    request,
                    ttrpc_timeout(context),
                )
                .await
            }

            async fn $capabilities(
                &self,
                context: &ttrpc::r#async::TtrpcContext,
                request: common::CapabilityRequest,
            ) -> ttrpc::Result<common::CapabilityResponse> {
                self.capability_call(request, ttrpc_timeout(context)).await
            }

            $(
                async fn $name(
                    &self,
                    context: &ttrpc::r#async::TtrpcContext,
                    request: common::ProviderRequest,
                ) -> ttrpc::Result<common::ProviderResponse> {
                    self.provider_call(
                        RpcOperation::Method(ProviderMethod::$method),
                        request,
                        ttrpc_timeout(context),
                    )
                    .await
                }
            )+
        }
    };
}

provider_service!(
    provider_runtime_ttrpc::RuntimeProviderService,
    health,
    capabilities,
    {
        plan => RuntimePlan,
        ensure => RuntimeEnsure,
        start => RuntimeStart,
        stop => RuntimeStop,
        execute => RuntimeExecute,
        inspect => RuntimeInspect,
        adopt => RuntimeAdopt,
        destroy => RuntimeDestroy,
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    fn status_message(error: ttrpc::Error) -> String {
        match error {
            ttrpc::Error::RpcStatus(status) => status.message,
            _ => panic!("expected typed rpc status"),
        }
    }

    #[test]
    fn rpc_statuses_retain_closed_actionable_reasons() {
        assert_eq!(
            status_message(rpc_status(ttrpc::Code::FAILED_PRECONDITION)),
            "provider precondition failed"
        );
        assert_eq!(
            status_message(invalid_request_contract(
                ServiceContractError::InvalidDeadline
            )),
            "v2-service-invalid-deadline"
        );
        assert_eq!(
            status_message(session_error(SessionError::new(
                SessionErrorCode::GenerationMismatch
            ))),
            "generation-mismatch"
        );
    }
}
provider_service!(
    provider_infrastructure_ttrpc::InfrastructureProviderService,
    health,
    capabilities,
    {
        plan => InfrastructurePlan,
        apply => InfrastructureApply,
        set_power_state => InfrastructureSetPowerState,
        inspect => InfrastructureInspect,
        adopt => InfrastructureAdopt,
        bootstrap_binding => InfrastructureBootstrapBinding,
        destroy => InfrastructureDestroy,
    }
);
provider_service!(
    provider_transport_ttrpc::TransportProviderService,
    health,
    capabilities,
    {
        connect => TransportConnect,
        listen => TransportListen,
        issue_binding => TransportIssueBinding,
        revoke_binding => TransportRevokeBinding,
        inspect => TransportInspect,
    }
);
provider_service!(
    provider_substrate_ttrpc::SubstrateProviderService,
    health,
    capabilities,
    {
        check => SubstrateCheck,
        plan_remediation => SubstratePlanRemediation,
        apply => SubstrateApply,
    }
);
provider_service!(
    provider_credential_ttrpc::CredentialProviderService,
    health,
    capabilities,
    {
        status => CredentialStatus,
        acquire_lease => CredentialAcquireLease,
        refresh_lease => CredentialRefreshLease,
        revoke_lease => CredentialRevokeLease,
    }
);
provider_service!(
    provider_display_ttrpc::DisplayProviderService,
    health,
    capabilities,
    {
        open => DisplayOpen,
        inspect => DisplayInspect,
        adopt => DisplayAdopt,
        close => DisplayClose,
    }
);
provider_service!(
    provider_network_ttrpc::NetworkProviderService,
    health,
    capabilities,
    {
        plan => NetworkPlan,
        ensure => NetworkEnsure,
        inspect => NetworkInspect,
        adopt => NetworkAdopt,
        release => NetworkRelease,
    }
);
provider_service!(
    provider_storage_ttrpc::StorageProviderService,
    health,
    capabilities,
    {
        plan => StoragePlan,
        ensure => StorageEnsure,
        inspect => StorageInspect,
        adopt => StorageAdopt,
        snapshot => StorageSnapshot,
        destroy => StorageDestroy,
    }
);
provider_service!(
    provider_device_ttrpc::DeviceProviderService,
    health,
    capabilities,
    {
        plan_attach => DevicePlanAttach,
        attach => DeviceAttach,
        inspect => DeviceInspect,
        adopt => DeviceAdopt,
        detach => DeviceDetach,
    }
);
provider_service!(
    provider_audio_ttrpc::AudioProviderService,
    health,
    capabilities,
    {
        open => AudioOpen,
        set_state => AudioSetState,
        inspect => AudioInspect,
        adopt => AudioAdopt,
        close => AudioClose,
    }
);
provider_service!(
    provider_observability_ttrpc::ObservabilityProviderService,
    health,
    capabilities,
    {
        status => ObservabilityStatus,
        query => ObservabilityQuery,
        subscribe => ObservabilitySubscribe,
        export => ObservabilityExport,
    }
);
