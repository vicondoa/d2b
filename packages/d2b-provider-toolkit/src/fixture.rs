use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType, RealmId, RoleId, WorkloadId},
    v2_provider::{
        AdoptionRequest, AdoptionState, AgentPlacementBinding, AudioChannel, AudioDirection,
        AudioProvider, CgroupAuthority, ConfiguredItemId, CorrelationId, CredentialLease,
        CredentialLeaseRequest, CredentialLeaseState, CredentialLeaseTransferPolicy,
        CredentialPlacementBinding, CredentialProvider, DeviceMediationPosture, DeviceProvider,
        DeviceSelectorId, DisplayProvider, Fingerprint, Generation, IdempotencyKey,
        ImplementationId, InfrastructurePowerState, InfrastructureProvider, LeaseId,
        MutationReceipt, MutationState, NetworkPosture, NetworkProvider, ObservabilityExportFormat,
        ObservabilityProvider, ObservabilityView, ObservationReason, ObservedLifecycleState,
        OperationId, PROVIDER_SCHEMA_VERSION, PersistentIdentityPosture, PlanId,
        PlannedResourceClass, PrincipalRef, ProcessAuthority, Provider, ProviderApiVersion,
        ProviderAuthority, ProviderCallContext, ProviderCapability, ProviderCapabilitySet,
        ProviderContractError, ProviderDescriptor, ProviderFuture, ProviderHandle, ProviderHealth,
        ProviderHealthReason, ProviderHealthState, ProviderMethod, ProviderObservation,
        ProviderOperationContext, ProviderOperationInput, ProviderOperationRequest,
        ProviderPlacement, ProviderPlan, ProviderRemediation, ProviderTarget,
        RuntimeAuthorityPosture, RuntimeProvider, SdkOperationClass, SourceVersion,
        StorageProvider, StorageSnapshotId, SubstrateProvider, TransportBindingId,
        TransportProvider, UserNamespacePosture,
    },
};
use d2b_provider::{
    ProviderClock, ProviderInstance, SessionIdentity, provider_method_is_dispatchable,
};

use crate::ProviderValues;

#[derive(Debug)]
pub struct DeterministicClock {
    now_unix_ms: AtomicU64,
}

impl DeterministicClock {
    pub fn new(now_unix_ms: u64) -> Self {
        Self {
            now_unix_ms: AtomicU64::new(now_unix_ms),
        }
    }

    pub fn set(&self, now_unix_ms: u64) {
        self.now_unix_ms.store(now_unix_ms, Ordering::Release);
    }
}

impl ProviderClock for DeterministicClock {
    fn now_unix_ms(&self) -> u64 {
        self.now_unix_ms.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
pub struct Fixture {
    pub descriptor: ProviderDescriptor,
    pub now_unix_ms: u64,
    target: ProviderTarget,
}

impl Fixture {
    pub fn new(provider_type: ProviderType, ordinal: usize) -> Result<Self, ProviderContractError> {
        let now_unix_ms = 1_700_000_000_000;
        let realm_id = RealmId::parse("aaaaaaaaaaaaaaaaaaaa")
            .map_err(|_| ProviderContractError::InvalidIdentifier)?;
        let workload_id = WorkloadId::parse("ccccccccccccccccccca")
            .map_err(|_| ProviderContractError::InvalidIdentifier)?;
        let role_id = RoleId::parse("ddddddddddddddddddda")
            .map_err(|_| ProviderContractError::InvalidIdentifier)?;
        let provider_char = char::from(b'b' + u8::try_from(ordinal % 24).unwrap_or(0));
        let provider_id = ProviderId::parse(format!("{provider_char:>19}a").replace(' ', "b"))
            .map_err(|_| ProviderContractError::InvalidIdentifier)?;
        let generation = Generation::new(1)?;
        let capabilities = ProviderCapabilitySet::new(
            ProviderMethod::ALL
                .iter()
                .filter(|method| method.provider_type() == provider_type)
                .filter(|method| provider_method_is_dispatchable(**method))
                .copied()
                .map(ProviderCapability)
                .collect(),
        )?;
        let authority = match provider_type {
            ProviderType::Runtime => ProviderAuthority::Runtime {
                posture: RuntimeAuthorityPosture {
                    process: ProcessAuthority::ProviderManagedRemote,
                    cgroup: CgroupAuthority::ProviderManagedRemote,
                    network: NetworkPosture::IsolatedNamespace,
                    user_namespace: UserNamespacePosture::None,
                    persistent_identity: PersistentIdentityPosture::NonCopyableAttested,
                    device_mediation: DeviceMediationPosture::ProviderManagedTyped,
                },
            },
            ProviderType::Infrastructure => ProviderAuthority::Infrastructure,
            ProviderType::Transport => ProviderAuthority::Transport,
            ProviderType::Substrate => ProviderAuthority::Substrate,
            ProviderType::Credential => ProviderAuthority::Credential,
            ProviderType::Display => ProviderAuthority::Display,
            ProviderType::Network => ProviderAuthority::Network,
            ProviderType::Storage => ProviderAuthority::Storage,
            ProviderType::Device => ProviderAuthority::Device,
            ProviderType::Audio => ProviderAuthority::Audio,
            ProviderType::Observability => ProviderAuthority::Observability,
        };
        let descriptor = ProviderDescriptor {
            schema_version: PROVIDER_SCHEMA_VERSION,
            provider_id,
            authority,
            implementation_id: ImplementationId::parse(format!("{}-fake", provider_type.as_str()))?,
            api_version: ProviderApiVersion::V2,
            capabilities,
            configuration_schema_fingerprint: fingerprint(ordinal + 1)?,
            configured_scope_digest: fingerprint(ordinal + 100)?,
            registry_generation: generation,
            placement: ProviderPlacement::ProviderAgent {
                realm_id: realm_id.clone(),
                workload_id: workload_id.clone(),
                role_id,
                endpoint_role: EndpointRole::ProviderAgent,
                service: ServicePackage::ProviderV2,
                agent_generation: generation,
            },
        };
        Self::from_descriptor(
            descriptor,
            ProviderTarget::Workload {
                realm_id,
                workload_id,
            },
            now_unix_ms,
        )
    }

    pub fn from_descriptor(
        descriptor: ProviderDescriptor,
        target: ProviderTarget,
        now_unix_ms: u64,
    ) -> Result<Self, ProviderContractError> {
        descriptor.validate()?;
        if target.realm_id() != descriptor.placement.realm_id() {
            return Err(ProviderContractError::ScopeMismatch);
        }
        if now_unix_ms <= 1_000 || now_unix_ms > d2b_contracts::v2_provider::MAX_SAFE_JSON_INTEGER {
            return Err(ProviderContractError::InvalidTimeRange);
        }
        Ok(Self {
            descriptor,
            now_unix_ms,
            target,
        })
    }

    pub fn operation(
        &self,
        method: ProviderMethod,
    ) -> Result<ProviderOperationContext, ProviderContractError> {
        Ok(ProviderOperationContext {
            schema_version: PROVIDER_SCHEMA_VERSION,
            operation_id: OperationId::parse("operation-fixture")?,
            idempotency_key: IdempotencyKey::parse("idempotency-fixture")?,
            request_digest: fingerprint(200)?,
            scope: match &self.target {
                ProviderTarget::Realm { realm_id }
                | ProviderTarget::Handle {
                    realm_id,
                    workload_id: None,
                    ..
                } => d2b_contracts::v2_provider::AuthorizedProviderScope::Realm {
                    realm_id: realm_id.clone(),
                },
                ProviderTarget::Workload {
                    realm_id,
                    workload_id,
                }
                | ProviderTarget::Handle {
                    realm_id,
                    workload_id: Some(workload_id),
                    ..
                } => d2b_contracts::v2_provider::AuthorizedProviderScope::Workload {
                    realm_id: realm_id.clone(),
                    workload_id: workload_id.clone(),
                },
            },
            principal: PrincipalRef::parse("principal-fixture")?,
            provider_id: self.descriptor.provider_id.clone(),
            provider_type: self.descriptor.provider_type(),
            provider_generation: self.descriptor.registry_generation,
            capability: ProviderCapability(method),
            method,
            policy_epoch: Generation::new(1)?,
            authorization_decision_digest: fingerprint(201)?,
            issued_at_unix_ms: self.now_unix_ms - 1_000,
            expires_at_unix_ms: self.now_unix_ms + 60_000,
            correlation_id: CorrelationId::parse("correlation-fixture")?,
            trace_id: fingerprint(202)?,
        })
    }

    pub fn request(
        &self,
        method: ProviderMethod,
    ) -> Result<ProviderOperationRequest, ProviderContractError> {
        self.request_with_input(method, sample_input(method)?)
    }

    pub fn request_with_input(
        &self,
        method: ProviderMethod,
        input: ProviderOperationInput,
    ) -> Result<ProviderOperationRequest, ProviderContractError> {
        Ok(ProviderOperationRequest {
            context: self.operation(method)?,
            target: self.target.clone(),
            expected_configuration_fingerprint: self
                .descriptor
                .configuration_schema_fingerprint
                .clone(),
            input,
        })
    }

    pub fn call_context<'a>(
        &self,
        operation: &'a ProviderOperationContext,
    ) -> ProviderCallContext<'a> {
        let (peer_role, service) = match &self.descriptor.placement {
            ProviderPlacement::TrustedFirstPartyInProcess {
                controller_role, ..
            } => (*controller_role, ServicePackage::ProviderV2),
            ProviderPlacement::ProviderAgent {
                endpoint_role,
                service,
                ..
            }
            | ProviderPlacement::UserAgent {
                endpoint_role,
                service,
                ..
            } => (*endpoint_role, *service),
        };
        ProviderCallContext {
            operation,
            peer_role,
            service,
            monotonic_deadline_remaining_ms: 30_000,
            cancelled: false,
        }
    }

    pub fn session_identity(&self) -> SessionIdentity {
        SessionIdentity {
            peer_role: EndpointRole::ProviderAgent,
            service: ServicePackage::ProviderV2,
            provider_id: self.descriptor.provider_id.clone(),
            provider_type: self.descriptor.provider_type(),
            provider_generation: self.descriptor.registry_generation,
        }
    }
}

fn sample_input(method: ProviderMethod) -> Result<ProviderOperationInput, ProviderContractError> {
    Ok(match method {
        ProviderMethod::RuntimeExecute => ProviderOperationInput::ConfiguredRuntimeExecution {
            configured_item_id: ConfiguredItemId::parse("configured-item")?,
        },
        ProviderMethod::InfrastructureSetPowerState => {
            ProviderOperationInput::InfrastructurePowerState {
                state: InfrastructurePowerState::Running,
            }
        }
        ProviderMethod::InfrastructureBootstrapBinding | ProviderMethod::TransportRevokeBinding => {
            ProviderOperationInput::TransportBinding {
                transport_binding_id: TransportBindingId::parse("transport-binding")?,
            }
        }
        ProviderMethod::StorageSnapshot => ProviderOperationInput::StorageSnapshot {
            snapshot_id: StorageSnapshotId::parse("snapshot-fixture")?,
        },
        ProviderMethod::DevicePlanAttach => ProviderOperationInput::DeviceSelector {
            device_selector_id: DeviceSelectorId::parse("device-selector")?,
        },
        ProviderMethod::AudioSetState => ProviderOperationInput::AudioState {
            channel: AudioChannel::Speaker,
            direction: AudioDirection::Output,
            mute: Some(false),
            volume: Some(50),
        },
        ProviderMethod::ObservabilityQuery => ProviderOperationInput::ObservabilityQuery {
            view: ObservabilityView::Lifecycle,
            cursor: None,
            limit: 32,
        },
        ProviderMethod::ObservabilityExport => ProviderOperationInput::ObservabilityExport {
            format: ObservabilityExportFormat::JsonLines,
            start_at_unix_ms: 1_699_999_940_000,
            end_at_unix_ms: 1_700_000_000_000,
        },
        _ => ProviderOperationInput::NoInput,
    })
}

fn fingerprint(value: usize) -> Result<Fingerprint, ProviderContractError> {
    Fingerprint::parse(format!("{value:064x}"))
}

pub struct FakeProvider {
    fixture: Fixture,
}

impl FakeProvider {
    pub fn new(fixture: Fixture) -> Self {
        Self { fixture }
    }

    pub fn instance(self: Arc<Self>) -> ProviderInstance {
        match self.fixture.descriptor.provider_type() {
            ProviderType::Runtime => ProviderInstance::Runtime(self),
            ProviderType::Infrastructure => ProviderInstance::Infrastructure(self),
            ProviderType::Transport => ProviderInstance::Transport(self),
            ProviderType::Substrate => ProviderInstance::Substrate(self),
            ProviderType::Credential => ProviderInstance::Credential(self),
            ProviderType::Display => ProviderInstance::Display(self),
            ProviderType::Network => ProviderInstance::Network(self),
            ProviderType::Storage => ProviderInstance::Storage(self),
            ProviderType::Device => ProviderInstance::Device(self),
            ProviderType::Audio => ProviderInstance::Audio(self),
            ProviderType::Observability => ProviderInstance::Observability(self),
        }
    }

    fn values(&self) -> ProviderValues {
        ProviderValues::new(&self.fixture.descriptor, self.fixture.now_unix_ms)
            .unwrap_or_else(|_| unreachable!())
    }

    fn health_value(&self) -> ProviderHealth {
        self.values()
            .health(
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            )
            .unwrap_or_else(|_| unreachable!())
    }

    fn plan_value(&self, request: &ProviderOperationRequest) -> ProviderPlan {
        self.values()
            .plan(
                request,
                PlanId::parse("plan-fixture").unwrap_or_else(|_| unreachable!()),
                self.fixture.now_unix_ms + 30_000,
                BoundedVec::new(Vec::<PlannedResourceClass>::new())
                    .unwrap_or_else(|_| unreachable!()),
            )
            .unwrap_or_else(|_| unreachable!())
    }

    fn handle_from_request(&self, request: &ProviderOperationRequest) -> ProviderHandle {
        let values = self.values();
        values
            .handle_from_request(
                request,
                d2b_contracts::v2_provider::HandleId::parse("handle-fixture")
                    .unwrap_or_else(|_| unreachable!()),
                values.provider_owner(request.target.realm_id()),
                Generation::new(1).unwrap_or_else(|_| unreachable!()),
                None,
            )
            .unwrap_or_else(|_| unreachable!())
    }

    fn handle_from_plan(&self, plan: &ProviderPlan) -> ProviderHandle {
        let values = self.values();
        values
            .handle_from_plan(
                plan,
                d2b_contracts::v2_provider::HandleId::parse("handle-fixture")
                    .unwrap_or_else(|_| unreachable!()),
                values.provider_owner(&plan.realm_id),
                Generation::new(1).unwrap_or_else(|_| unreachable!()),
                None,
            )
            .unwrap_or_else(|_| unreachable!())
    }

    fn observation(
        &self,
        context: &ProviderOperationContext,
        adoption: AdoptionState,
        handle: Option<&ProviderHandle>,
    ) -> ProviderObservation {
        self.values()
            .observation(
                context,
                handle,
                ObservedLifecycleState::Ready,
                adoption,
                ObservationReason::None,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            )
            .unwrap_or_else(|_| unreachable!())
    }

    fn receipt(&self, context: &ProviderOperationContext) -> MutationReceipt {
        self.values()
            .receipt(context, MutationState::Applied)
            .unwrap_or_else(|_| unreachable!())
    }
}

impl Provider for FakeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.fixture.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        _context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move { Ok(self.health_value()) })
    }
}

macro_rules! fake_capabilities {
    () => {
        fn capabilities(&self) -> ProviderCapabilitySet {
            self.fixture.descriptor.capabilities.clone()
        }
    };
}

macro_rules! fake_plan {
    ($name:ident) => {
        fn $name<'a>(
            &'a self,
            _context: &'a ProviderCallContext<'a>,
            request: &'a ProviderOperationRequest,
        ) -> ProviderFuture<'a, ProviderPlan> {
            Box::pin(async move { Ok(self.plan_value(request)) })
        }
    };
}

macro_rules! fake_plan_handle {
    ($name:ident) => {
        fn $name<'a>(
            &'a self,
            _context: &'a ProviderCallContext<'a>,
            plan: &'a ProviderPlan,
        ) -> ProviderFuture<'a, ProviderHandle> {
            Box::pin(async move { Ok(self.handle_from_plan(plan)) })
        }
    };
}

macro_rules! fake_handle {
    ($name:ident) => {
        fn $name<'a>(
            &'a self,
            _context: &'a ProviderCallContext<'a>,
            request: &'a ProviderOperationRequest,
        ) -> ProviderFuture<'a, ProviderHandle> {
            Box::pin(async move { Ok(self.handle_from_request(request)) })
        }
    };
}

macro_rules! fake_observation {
    ($name:ident) => {
        fn $name<'a>(
            &'a self,
            _context: &'a ProviderCallContext<'a>,
            request: &'a ProviderOperationRequest,
        ) -> ProviderFuture<'a, ProviderObservation> {
            Box::pin(async move {
                Ok(self.observation(&request.context, AdoptionState::NotAttempted, None))
            })
        }
    };
}

macro_rules! fake_adoption {
    () => {
        fn adopt<'a>(
            &'a self,
            _context: &'a ProviderCallContext<'a>,
            request: &'a AdoptionRequest,
        ) -> ProviderFuture<'a, ProviderObservation> {
            Box::pin(async move {
                Ok(self.observation(
                    &request.context,
                    AdoptionState::Adopted,
                    Some(&request.handle),
                ))
            })
        }
    };
}

macro_rules! fake_mutation {
    ($name:ident) => {
        fn $name<'a>(
            &'a self,
            _context: &'a ProviderCallContext<'a>,
            request: &'a ProviderOperationRequest,
        ) -> ProviderFuture<'a, MutationReceipt> {
            Box::pin(async move { Ok(self.receipt(&request.context)) })
        }
    };
}

macro_rules! fake_plan_mutation {
    ($name:ident) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            _plan: &'a ProviderPlan,
        ) -> ProviderFuture<'a, MutationReceipt> {
            Box::pin(async move { Ok(self.receipt(context.operation)) })
        }
    };
}

impl RuntimeProvider for FakeProvider {
    fake_capabilities!();
    fake_plan!(plan);
    fake_plan_handle!(ensure);
    fake_observation!(start);
    fake_observation!(stop);
    fake_observation!(inspect);
    fake_adoption!();
    fake_mutation!(destroy);
}

impl InfrastructureProvider for FakeProvider {
    fake_capabilities!();
    fake_plan!(plan);
    fake_plan_handle!(apply);
    fake_observation!(set_power_state);
    fake_observation!(inspect);
    fake_adoption!();
    fake_handle!(bootstrap_binding);
    fake_mutation!(destroy);
}

impl TransportProvider for FakeProvider {
    fake_capabilities!();
    fake_handle!(connect);
    fake_handle!(listen);
    fake_handle!(issue_binding);
    fake_mutation!(revoke_binding);
    fake_observation!(inspect);
}

impl SubstrateProvider for FakeProvider {
    fake_capabilities!();
    fake_observation!(check);
    fake_plan!(plan_remediation);
    fake_plan_mutation!(apply);
}

impl CredentialProvider for FakeProvider {
    fake_observation!(status);

    fn acquire_lease<'a>(
        &'a self,
        _context: &'a ProviderCallContext<'a>,
        request: &'a CredentialLeaseRequest,
    ) -> ProviderFuture<'a, CredentialLease> {
        Box::pin(async move {
            Ok(CredentialLease {
                lease_id: LeaseId::parse("lease-fixture").unwrap_or_else(|_| unreachable!()),
                credential_provider_id: self.fixture.descriptor.provider_id.clone(),
                consumer_provider_id: request.consumer_provider_id.clone(),
                placement_binding: request.placement_binding.clone(),
                allowed_operations: request.allowed_operations.clone(),
                issued_at_unix_ms: self.fixture.now_unix_ms,
                expires_at_unix_ms: request.requested_expiry_unix_ms,
                credential_provider_generation: self.fixture.descriptor.registry_generation,
                consumer_provider_generation: Generation::new(1).unwrap_or_else(|_| unreachable!()),
                source_version: SourceVersion::parse("source-fixture")
                    .unwrap_or_else(|_| unreachable!()),
                rotation_generation: Generation::new(1).unwrap_or_else(|_| unreachable!()),
                state: CredentialLeaseState::Active,
                transfer_policy: CredentialLeaseTransferPolicy::Forbidden,
                revoked_at_unix_ms: None,
            })
        })
    }

    fn refresh_lease<'a>(
        &'a self,
        _context: &'a ProviderCallContext<'a>,
        lease: &'a CredentialLease,
    ) -> ProviderFuture<'a, CredentialLease> {
        Box::pin(async move { Ok(lease.clone()) })
    }

    fn revoke_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        _lease: &'a CredentialLease,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move { Ok(self.receipt(context.operation)) })
    }
}

impl DisplayProvider for FakeProvider {
    fake_capabilities!();
    fake_handle!(open);
    fake_observation!(inspect);
    fake_adoption!();
    fake_mutation!(close);
}

impl NetworkProvider for FakeProvider {
    fake_capabilities!();
    fake_plan!(plan);
    fake_plan_handle!(ensure);
    fake_observation!(inspect);
    fake_adoption!();
    fake_mutation!(release);
}

impl StorageProvider for FakeProvider {
    fake_capabilities!();
    fake_plan!(plan);
    fake_plan_handle!(ensure);
    fake_observation!(inspect);
    fake_adoption!();
    fake_handle!(snapshot);
    fake_mutation!(destroy);
}

impl DeviceProvider for FakeProvider {
    fake_capabilities!();
    fake_plan!(plan_attach);
    fake_plan_handle!(attach);
    fake_observation!(inspect);
    fake_adoption!();
    fake_mutation!(detach);
}

impl AudioProvider for FakeProvider {
    fake_capabilities!();
    fake_handle!(open);
    fake_observation!(set_state);
    fake_observation!(inspect);
    fake_adoption!();
    fake_mutation!(close);
}

impl ObservabilityProvider for FakeProvider {
    fake_capabilities!();
    fake_observation!(status);
    fake_observation!(query);
    fake_handle!(subscribe);
    fake_mutation!(export);
}

pub fn sample_lease_request(
    fixture: &Fixture,
) -> Result<CredentialLeaseRequest, ProviderContractError> {
    let ProviderPlacement::ProviderAgent {
        realm_id,
        workload_id,
        role_id,
        agent_generation,
        ..
    } = &fixture.descriptor.placement
    else {
        return Err(ProviderContractError::PlacementMismatch);
    };
    Ok(CredentialLeaseRequest {
        context: fixture.operation(ProviderMethod::CredentialAcquireLease)?,
        consumer_provider_id: ProviderId::parse("eeeeeeeeeeeeeeeeeeea")
            .map_err(|_| ProviderContractError::InvalidIdentifier)?,
        placement_binding: CredentialPlacementBinding::ProviderAgent {
            binding: AgentPlacementBinding {
                realm_id: realm_id.clone(),
                workload_id: workload_id.clone(),
                role_id: role_id.clone(),
                agent_generation: *agent_generation,
            },
        },
        allowed_operations: BoundedVec::new(vec![SdkOperationClass::Read])
            .map_err(|_| ProviderContractError::BoundExceeded)?,
        requested_expiry_unix_ms: fixture.now_unix_ms + 30_000,
    })
}
