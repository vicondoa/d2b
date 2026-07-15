use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    v2_component_session::ServicePackage,
    v2_identity::ProviderType,
    v2_provider::{
        AdoptionState, Generation, HandleId, MAX_SAFE_JSON_INTEGER, MutationReceipt, MutationState,
        ObservationReason, ObservedLifecycleState, OperationBinding, Provider, ProviderCallContext,
        ProviderCapability, ProviderCapabilitySet, ProviderContractError, ProviderDescriptor,
        ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle, ProviderHealth,
        ProviderHealthReason, ProviderHealthState, ProviderMethod, ProviderObservation,
        ProviderOperationContext, ProviderOperationInput, ProviderOperationRequest,
        ProviderPlacement, ProviderRemediation, ProviderResult, ProviderTarget, RetryClass,
        TransportBindingId, TransportProvider,
    },
};
use d2b_provider_toolkit::ProviderValues;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::{
    EndpointCloseRequest, EndpointCloseState, EndpointConnectRequest, EndpointConnection,
    EndpointInspectRequest, EndpointObservation, EndpointObservationState, EndpointPortError,
    LocalEndpointPort, LocalTransportKind, ReachabilityEvidence, TransportBinding,
};

pub const MAX_LOCAL_TRANSPORT_BINDINGS: usize = 256;
pub const MAX_ACTIVE_LOCAL_TRANSPORTS: usize = 256;

/// Resource bounds for one local transport provider instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTransportLimits {
    pub max_bindings: usize,
    pub max_active: usize,
}

impl Default for LocalTransportLimits {
    fn default() -> Self {
        Self {
            max_bindings: MAX_LOCAL_TRANSPORT_BINDINGS,
            max_active: MAX_ACTIVE_LOCAL_TRANSPORTS,
        }
    }
}

/// Construction failures are closed and contain no endpoint details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalTransportConfigurationError {
    Contract(ProviderContractError),
    WrongAuthority,
    WrongImplementation,
    WrongPlacement,
    CapabilityMismatch,
    InvalidLimit,
    BindingLimit,
    DuplicateBinding,
    BindingMismatch,
}

impl fmt::Display for LocalTransportConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => {
                write!(formatter, "local transport contract invalid ({error})")
            }
            Self::WrongAuthority => formatter.write_str("local transport authority mismatch"),
            Self::WrongImplementation => {
                formatter.write_str("local transport implementation mismatch")
            }
            Self::WrongPlacement => formatter.write_str("local transport placement mismatch"),
            Self::CapabilityMismatch => formatter.write_str("local transport capability mismatch"),
            Self::InvalidLimit => formatter.write_str("local transport limit invalid"),
            Self::BindingLimit => formatter.write_str("local transport binding limit exceeded"),
            Self::DuplicateBinding => formatter.write_str("duplicate local transport binding"),
            Self::BindingMismatch => formatter.write_str("local transport binding mismatch"),
        }
    }
}

impl std::error::Error for LocalTransportConfigurationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProviderContractError> for LocalTransportConfigurationError {
    fn from(value: ProviderContractError) -> Self {
        Self::Contract(value)
    }
}

/// Time source used for absolute provider-contract validation.
pub trait LocalTransportClock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemTransportClock;

impl LocalTransportClock for SystemTransportClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .unwrap_or(0)
    }
}

struct ActiveTransport {
    binding_id: TransportBindingId,
    connection: EndpointConnection,
    handle: ProviderHandle,
    _permit: OwnedSemaphorePermit,
}

#[derive(Clone)]
struct ActiveSnapshot {
    binding_id: TransportBindingId,
    connection: EndpointConnection,
    handle: ProviderHandle,
}

impl From<&ActiveTransport> for ActiveSnapshot {
    fn from(value: &ActiveTransport) -> Self {
        Self {
            binding_id: value.binding_id.clone(),
            connection: value.connection.clone(),
            handle: value.handle.clone(),
        }
    }
}

/// Canonical local transport provider for exactly one implementation kind.
pub struct LocalTransportProvider {
    descriptor: ProviderDescriptor,
    kind: LocalTransportKind,
    bindings: BTreeMap<TransportBindingId, TransportBinding>,
    endpoint_port: Arc<dyn LocalEndpointPort>,
    clock: Arc<dyn LocalTransportClock>,
    active: Mutex<BTreeMap<HandleId, ActiveTransport>>,
    revoked_bindings: Mutex<BTreeSet<TransportBindingId>>,
    active_slots: Arc<Semaphore>,
    binding_gate: Mutex<()>,
}

impl fmt::Debug for LocalTransportProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalTransportProvider")
            .field("provider_generation", &self.descriptor.registry_generation)
            .field("kind", &self.kind)
            .field("binding_count", &self.bindings.len())
            .finish_non_exhaustive()
    }
}

impl LocalTransportProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        kind: LocalTransportKind,
        bindings: Vec<TransportBinding>,
        endpoint_port: Arc<dyn LocalEndpointPort>,
    ) -> Result<Self, LocalTransportConfigurationError> {
        Self::with_clock_and_limits(
            descriptor,
            kind,
            bindings,
            endpoint_port,
            Arc::new(SystemTransportClock),
            LocalTransportLimits::default(),
        )
    }

    pub fn with_clock_and_limits(
        descriptor: ProviderDescriptor,
        kind: LocalTransportKind,
        bindings: Vec<TransportBinding>,
        endpoint_port: Arc<dyn LocalEndpointPort>,
        clock: Arc<dyn LocalTransportClock>,
        limits: LocalTransportLimits,
    ) -> Result<Self, LocalTransportConfigurationError> {
        descriptor.validate()?;
        if descriptor.provider_type() != ProviderType::Transport {
            return Err(LocalTransportConfigurationError::WrongAuthority);
        }
        if &descriptor.implementation_id != kind.implementation_id() {
            return Err(LocalTransportConfigurationError::WrongImplementation);
        }
        if !matches!(
            descriptor.placement,
            ProviderPlacement::TrustedFirstPartyInProcess { .. }
        ) {
            return Err(LocalTransportConfigurationError::WrongPlacement);
        }
        if descriptor.capabilities != local_transport_capabilities() {
            return Err(LocalTransportConfigurationError::CapabilityMismatch);
        }
        if limits.max_bindings == 0
            || limits.max_bindings > MAX_LOCAL_TRANSPORT_BINDINGS
            || limits.max_active == 0
            || limits.max_active > MAX_ACTIVE_LOCAL_TRANSPORTS
        {
            return Err(LocalTransportConfigurationError::InvalidLimit);
        }
        if bindings.is_empty() || bindings.len() > limits.max_bindings {
            return Err(LocalTransportConfigurationError::BindingLimit);
        }

        let mut binding_map = BTreeMap::new();
        for binding in bindings {
            validate_configured_binding(&descriptor, kind, &binding)?;
            if binding_map
                .insert(binding.binding_id().clone(), binding)
                .is_some()
            {
                return Err(LocalTransportConfigurationError::DuplicateBinding);
            }
        }

        Ok(Self {
            descriptor,
            kind,
            bindings: binding_map,
            endpoint_port,
            clock,
            active: Mutex::new(BTreeMap::new()),
            revoked_bindings: Mutex::new(BTreeSet::new()),
            active_slots: Arc::new(Semaphore::new(limits.max_active)),
            binding_gate: Mutex::new(()),
        })
    }

    pub const fn kind(&self) -> LocalTransportKind {
        self.kind
    }

    pub fn binding(&self, id: &TransportBindingId) -> Option<&TransportBinding> {
        self.bindings.get(id)
    }

    async fn connect_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderHandle> {
        let now = self.clock.now_unix_ms();
        let binding_id = match self.validate_connect(context, request, now) {
            Ok(binding_id) => binding_id,
            Err(denial) => return Err(self.denial(&request.context, now, denial)),
        };
        let Some(binding) = self.bindings.get(&binding_id) else {
            return Err(self.failure(&request.context, now, FailureClass::Invariant));
        };
        let budget = Duration::from_millis(u64::from(context.monotonic_deadline_remaining_ms));
        match tokio::time::timeout(budget, self.connect_authorized(request, binding, budget)).await
        {
            Ok(result) => result,
            Err(_) => Err(self.denial(
                &request.context,
                self.clock.now_unix_ms(),
                RequestDenial::Deadline,
            )),
        }
    }

    async fn connect_authorized(
        &self,
        request: &ProviderOperationRequest,
        binding: &TransportBinding,
        budget: Duration,
    ) -> ProviderResult<ProviderHandle> {
        let _binding_guard = self.binding_gate.lock().await;
        if self
            .revoked_bindings
            .lock()
            .await
            .contains(binding.binding_id())
        {
            return Err(self.denial(
                &request.context,
                self.clock.now_unix_ms(),
                RequestDenial::Unauthorized,
            ));
        }
        let handle_id =
            HandleId::parse(request.context.operation_id.as_str().to_owned()).map_err(|_| {
                self.denial(
                    &request.context,
                    self.clock.now_unix_ms(),
                    RequestDenial::Invalid,
                )
            })?;

        if let Some(active) = self.active.lock().await.get(&handle_id) {
            if active.binding_id == *binding.binding_id()
                && active.handle.created_by == request.context.binding()
            {
                return Ok(active.handle.clone());
            }
            return Err(self.denial(
                &request.context,
                self.clock.now_unix_ms(),
                RequestDenial::Invalid,
            ));
        }

        let permit = self.active_slots.clone().try_acquire_owned().map_err(|_| {
            self.failure(
                &request.context,
                self.clock.now_unix_ms(),
                FailureClass::BoundExceeded,
            )
        })?;
        let connector_request = EndpointConnectRequest {
            operation_id: request.context.operation_id.clone(),
            handle_id: handle_id.clone(),
            binding_id: binding.binding_id().clone(),
            endpoint: binding.endpoint().clone(),
            expected_identity: binding.endpoint_identity().clone(),
            expected_generation: binding.endpoint_generation(),
            kind: binding.kind(),
            capabilities: binding.capabilities(),
            deadline: budget,
        };
        let connection = self
            .endpoint_port
            .connect(connector_request)
            .await
            .map_err(|error| {
                self.failure(
                    &request.context,
                    self.clock.now_unix_ms(),
                    FailureClass::from(error),
                )
            })?;
        if !connection_envelope_matches(
            &connection,
            request,
            binding,
            &handle_id,
            ReachabilityEvidence::ReachableOnly,
        ) {
            return Err(self.failure(
                &request.context,
                self.clock.now_unix_ms(),
                FailureClass::Invariant,
            ));
        }
        if connection.identity != *binding.endpoint_identity() {
            return Err(self.failure(
                &request.context,
                self.clock.now_unix_ms(),
                FailureClass::IdentityMismatch,
            ));
        }
        if connection.generation != binding.endpoint_generation() {
            return Err(self.failure(
                &request.context,
                self.clock.now_unix_ms(),
                FailureClass::GenerationMismatch,
            ));
        }

        let now = self.clock.now_unix_ms();
        let values = ProviderValues::new(&self.descriptor, now)
            .map_err(|_| self.failure(&request.context, now, FailureClass::Invariant))?;
        let handle = values
            .handle_from_request(
                request,
                handle_id.clone(),
                values.provider_owner(request.target.realm_id()),
                binding.endpoint_generation(),
                None,
            )
            .map_err(|_| self.failure(&request.context, now, FailureClass::Invariant))?;
        self.active.lock().await.insert(
            handle_id,
            ActiveTransport {
                binding_id: binding.binding_id().clone(),
                connection,
                handle: handle.clone(),
                _permit: permit,
            },
        );
        Ok(handle)
    }

    async fn inspect_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderObservation> {
        let now = self.clock.now_unix_ms();
        let (handle_id, handle_generation) = match self.validate_handle_request(
            context,
            request,
            now,
            ProviderMethod::TransportInspect,
        ) {
            Ok(target) => target,
            Err(denial) => return Err(self.denial(&request.context, now, denial)),
        };
        let active = {
            let active = self.active.lock().await;
            active.get(&handle_id).map(ActiveSnapshot::from)
        };
        let Some(active) = active else {
            return self.observation_without_handle(
                &request.context,
                ObservedLifecycleState::Released,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            );
        };
        if active.handle.resource_generation != handle_generation {
            return self.rejected_observation(
                &request.context,
                &active.handle,
                ObservationReason::GenerationMismatch,
            );
        }
        let budget = Duration::from_millis(u64::from(context.monotonic_deadline_remaining_ms));
        let port_request = EndpointInspectRequest {
            operation_id: request.context.operation_id.clone(),
            handle_id: handle_id.clone(),
            binding_id: active.binding_id.clone(),
            expected_identity: active.connection.identity.clone(),
            expected_generation: active.connection.generation,
            kind: active.connection.kind,
            capabilities: active.connection.capabilities,
            deadline: budget,
        };
        let observation = match tokio::time::timeout(
            budget,
            self.endpoint_port
                .inspect(port_request, active.connection.owned()),
        )
        .await
        {
            Ok(Ok(observation)) => observation,
            Ok(Err(error)) => {
                return Err(self.failure(
                    &request.context,
                    self.clock.now_unix_ms(),
                    FailureClass::from(error),
                ));
            }
            Err(_) => {
                return Err(self.denial(
                    &request.context,
                    self.clock.now_unix_ms(),
                    RequestDenial::Deadline,
                ));
            }
        };
        if !observation_envelope_matches(&observation, request, &active) {
            return Err(self.failure(
                &request.context,
                self.clock.now_unix_ms(),
                FailureClass::Invariant,
            ));
        }
        if observation.identity != active.connection.identity {
            return self.rejected_observation(
                &request.context,
                &active.handle,
                ObservationReason::IdentityMismatch,
            );
        }
        if observation.generation != active.connection.generation {
            return self.rejected_observation(
                &request.context,
                &active.handle,
                ObservationReason::GenerationMismatch,
            );
        }

        let result = match observation.state {
            EndpointObservationState::Connected => self.observation(
                &request.context,
                Some(&active.handle),
                ObservedLifecycleState::Running,
                AdoptionState::NotAttempted,
                ObservationReason::None,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            EndpointObservationState::Closed => self.observation(
                &request.context,
                Some(&active.handle),
                ObservedLifecycleState::Released,
                AdoptionState::NotAttempted,
                ObservationReason::None,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            EndpointObservationState::Unavailable => self.observation(
                &request.context,
                Some(&active.handle),
                ObservedLifecycleState::Unknown,
                AdoptionState::NotAttempted,
                ObservationReason::None,
                ProviderHealthState::Unavailable,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
        };
        if observation.state == EndpointObservationState::Closed {
            let _ = active.connection.owned().close();
            self.remove_if_same(&handle_id, &active.connection).await;
        }
        result
    }

    async fn revoke_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<MutationReceipt> {
        let now = self.clock.now_unix_ms();
        let (binding_id, target_handle) = match self.validate_revoke(context, request, now) {
            Ok(target) => target,
            Err(denial) => return Err(self.denial(&request.context, now, denial)),
        };
        let budget = Duration::from_millis(u64::from(context.monotonic_deadline_remaining_ms));
        match tokio::time::timeout(
            budget,
            self.revoke_authorized(request, &binding_id, target_handle, budget),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(self.denial(
                &request.context,
                self.clock.now_unix_ms(),
                RequestDenial::Deadline,
            )),
        }
    }

    async fn revoke_authorized(
        &self,
        request: &ProviderOperationRequest,
        binding_id: &TransportBindingId,
        target_handle: Option<(HandleId, Generation)>,
        budget: Duration,
    ) -> ProviderResult<MutationReceipt> {
        let _binding_guard = self.binding_gate.lock().await;
        let active = {
            let active = self.active.lock().await;
            if let Some((handle_id, expected_generation)) = &target_handle
                && let Some(candidate) = active.get(handle_id)
            {
                if candidate.binding_id != *binding_id {
                    return Err(self.denial(
                        &request.context,
                        self.clock.now_unix_ms(),
                        RequestDenial::Unauthorized,
                    ));
                }
                if candidate.handle.resource_generation != *expected_generation {
                    return Err(self.failure(
                        &request.context,
                        self.clock.now_unix_ms(),
                        FailureClass::GenerationMismatch,
                    ));
                }
            }
            active
                .iter()
                .filter(|(_, candidate)| candidate.binding_id == *binding_id)
                .map(|(handle_id, candidate)| (handle_id.clone(), ActiveSnapshot::from(candidate)))
                .collect::<Vec<_>>()
        };
        let newly_revoked = self
            .revoked_bindings
            .lock()
            .await
            .insert(binding_id.clone());
        let mut closed_any = false;
        for (handle_id, active) in active {
            let port_request = EndpointCloseRequest {
                operation_id: request.context.operation_id.clone(),
                handle_id: handle_id.clone(),
                binding_id: active.binding_id.clone(),
                expected_identity: active.connection.identity.clone(),
                expected_generation: active.connection.generation,
                kind: active.connection.kind,
                deadline: budget,
            };
            let close = self
                .endpoint_port
                .close(port_request, active.connection.owned())
                .await
                .map_err(|error| {
                    self.failure(
                        &request.context,
                        self.clock.now_unix_ms(),
                        FailureClass::from(error),
                    )
                })?;
            if close.operation_id != request.context.operation_id
                || close.handle_id != handle_id
                || close.binding_id != active.binding_id
            {
                return Err(self.failure(
                    &request.context,
                    self.clock.now_unix_ms(),
                    FailureClass::Invariant,
                ));
            }
            if close.identity != active.connection.identity {
                return Err(self.failure(
                    &request.context,
                    self.clock.now_unix_ms(),
                    FailureClass::IdentityMismatch,
                ));
            }
            if close.generation != active.connection.generation {
                return Err(self.failure(
                    &request.context,
                    self.clock.now_unix_ms(),
                    FailureClass::GenerationMismatch,
                ));
            }
            let owned_close = active.connection.owned().close();
            closed_any |= close.state == EndpointCloseState::Closed
                || owned_close == EndpointCloseState::Closed;
            self.remove_if_same(&handle_id, &active.connection).await;
        }
        let state = if newly_revoked || closed_any {
            MutationState::Applied
        } else {
            MutationState::AlreadyApplied
        };
        self.receipt(&request.context, state)
    }

    fn validate_connect(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        now: u64,
    ) -> Result<TransportBindingId, RequestDenial> {
        self.validate_request_common(context, request, now, ProviderMethod::TransportConnect)?;
        if matches!(request.target, ProviderTarget::Handle { .. }) {
            return Err(RequestDenial::Invalid);
        }
        let ProviderOperationInput::TransportBinding {
            transport_binding_id,
        } = &request.input
        else {
            return Err(RequestDenial::Invalid);
        };
        let binding = self
            .bindings
            .get(transport_binding_id)
            .ok_or(RequestDenial::Invalid)?;
        self.validate_binding_access(request, binding)?;
        Ok(transport_binding_id.clone())
    }

    fn validate_binding_access(
        &self,
        request: &ProviderOperationRequest,
        binding: &TransportBinding,
    ) -> Result<(), RequestDenial> {
        if binding.scope() != &request.context.scope
            || binding.provider_id() != &self.descriptor.provider_id
            || binding.provider_generation() != self.descriptor.registry_generation
            || binding.configuration_fingerprint() != &request.expected_configuration_fingerprint
            || binding.configured_scope_digest() != &self.descriptor.configured_scope_digest
            || binding.kind() != self.kind
        {
            return Err(RequestDenial::Unauthorized);
        }
        Ok(())
    }

    fn validate_handle_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        now: u64,
        expected_method: ProviderMethod,
    ) -> Result<(HandleId, Generation), RequestDenial> {
        self.validate_request_common(context, request, now, expected_method)?;
        match (&request.target, expected_method) {
            (
                ProviderTarget::Handle {
                    handle_id,
                    handle_generation,
                    ..
                },
                ProviderMethod::TransportInspect,
            ) if matches!(request.input, ProviderOperationInput::NoInput) => {
                Ok((handle_id.clone(), *handle_generation))
            }
            _ => Err(RequestDenial::Invalid),
        }
    }

    fn validate_revoke(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        now: u64,
    ) -> Result<(TransportBindingId, Option<(HandleId, Generation)>), RequestDenial> {
        self.validate_request_common(
            context,
            request,
            now,
            ProviderMethod::TransportRevokeBinding,
        )?;
        let ProviderOperationInput::TransportBinding {
            transport_binding_id,
        } = &request.input
        else {
            return Err(RequestDenial::Invalid);
        };
        let binding = self
            .bindings
            .get(transport_binding_id)
            .ok_or(RequestDenial::Invalid)?;
        self.validate_binding_access(request, binding)?;
        let target = match &request.target {
            ProviderTarget::Handle {
                handle_id,
                handle_generation,
                ..
            } => Some((handle_id.clone(), *handle_generation)),
            ProviderTarget::Realm { .. } | ProviderTarget::Workload { .. } => None,
        };
        Ok((transport_binding_id.clone(), target))
    }

    fn validate_request_common(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        now: u64,
        expected_method: ProviderMethod,
    ) -> Result<(), RequestDenial> {
        if context.cancelled {
            return Err(RequestDenial::Cancelled);
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(RequestDenial::Deadline);
        }
        if context.service != ServicePackage::ProviderV2 {
            return Err(RequestDenial::Unauthorized);
        }
        let ProviderPlacement::TrustedFirstPartyInProcess {
            controller_role, ..
        } = &self.descriptor.placement
        else {
            return Err(RequestDenial::Unauthorized);
        };
        if context.peer_role != *controller_role || context.operation != &request.context {
            return Err(RequestDenial::Unauthorized);
        }
        request
            .context
            .validate(&self.descriptor, now)
            .map_err(RequestDenial::from_contract)?;
        if request.context.method != expected_method {
            return Err(RequestDenial::Capability);
        }
        if request.target.realm_id() != request.context.scope.realm_id()
            || request.target.workload_id() != request.context.scope.workload_id()
        {
            return Err(RequestDenial::Unauthorized);
        }
        if request.expected_configuration_fingerprint
            != self.descriptor.configuration_schema_fingerprint
        {
            return Err(RequestDenial::Unauthorized);
        }
        Ok(())
    }

    async fn remove_if_same(&self, handle_id: &HandleId, connection: &EndpointConnection) {
        let mut active = self.active.lock().await;
        if active.get(handle_id).is_some_and(|candidate| {
            candidate.connection.identity == connection.identity
                && candidate.connection.generation == connection.generation
        }) {
            active.remove(handle_id);
        }
    }

    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    fn observation(
        &self,
        context: &ProviderOperationContext,
        handle: Option<&ProviderHandle>,
        lifecycle: ObservedLifecycleState,
        adoption: AdoptionState,
        reason: ObservationReason,
        health_state: ProviderHealthState,
        health_reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderResult<ProviderObservation> {
        let now = self.clock.now_unix_ms();
        ProviderValues::new(&self.descriptor, now)
            .and_then(|values| {
                values.observation(
                    context,
                    handle,
                    lifecycle,
                    adoption,
                    reason,
                    health_state,
                    health_reason,
                    remediation,
                )
            })
            .map_err(|_| self.failure(context, now, FailureClass::Invariant))
    }

    #[allow(clippy::result_large_err)]
    fn observation_without_handle(
        &self,
        context: &ProviderOperationContext,
        lifecycle: ObservedLifecycleState,
        health_state: ProviderHealthState,
        health_reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderResult<ProviderObservation> {
        self.observation(
            context,
            None,
            lifecycle,
            AdoptionState::NotAttempted,
            ObservationReason::None,
            health_state,
            health_reason,
            remediation,
        )
    }

    #[allow(clippy::result_large_err)]
    fn rejected_observation(
        &self,
        context: &ProviderOperationContext,
        handle: &ProviderHandle,
        reason: ObservationReason,
    ) -> ProviderResult<ProviderObservation> {
        let (health_reason, remediation) = match reason {
            ObservationReason::IdentityMismatch => (
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ),
            ObservationReason::GenerationMismatch => (
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ),
            _ => (
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
        };
        self.observation(
            context,
            Some(handle),
            ObservedLifecycleState::Quarantined,
            AdoptionState::Rejected,
            reason,
            ProviderHealthState::Failed,
            health_reason,
            remediation,
        )
    }

    #[allow(clippy::result_large_err)]
    fn receipt(
        &self,
        context: &ProviderOperationContext,
        state: MutationState,
    ) -> ProviderResult<MutationReceipt> {
        let now = self.clock.now_unix_ms();
        ProviderValues::new(&self.descriptor, now)
            .and_then(|values| values.receipt(context, state))
            .map_err(|_| self.failure(context, now, FailureClass::Invariant))
    }

    fn denial(
        &self,
        context: &ProviderOperationContext,
        now: u64,
        denial: RequestDenial,
    ) -> ProviderFailure {
        self.failure(context, now, FailureClass::from(denial))
    }

    fn failure(
        &self,
        context: &ProviderOperationContext,
        now: u64,
        class: FailureClass,
    ) -> ProviderFailure {
        let shape = class.shape();
        if let Ok(values) = ProviderValues::new(&self.descriptor, now)
            && let Ok(failure) = values.failure(
                context,
                shape.kind,
                shape.retry,
                shape.reason,
                shape.remediation,
            )
        {
            return failure;
        }
        ProviderFailure {
            kind: shape.kind,
            retry: shape.retry,
            provider_type: ProviderType::Transport,
            binding: OperationBinding {
                operation_id: context.operation_id.clone(),
                idempotency_key: context.idempotency_key.clone(),
                request_digest: context.request_digest.clone(),
                provider_id: self.descriptor.provider_id.clone(),
                provider_generation: self.descriptor.registry_generation,
            },
            correlation_id: context.correlation_id.clone(),
            occurred_at_unix_ms: now.min(MAX_SAFE_JSON_INTEGER),
            reason: shape.reason,
            remediation: shape.remediation,
        }
    }

    fn unsupported(&self, context: &ProviderOperationContext) -> ProviderFailure {
        self.failure(context, self.clock.now_unix_ms(), FailureClass::Capability)
    }
}

impl Provider for LocalTransportProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            let now = self.clock.now_unix_ms();
            if context.cancelled {
                return Err(self.denial(context.operation, now, RequestDenial::Cancelled));
            }
            if context.monotonic_deadline_remaining_ms == 0 {
                return Err(self.denial(context.operation, now, RequestDenial::Deadline));
            }
            ProviderValues::new(&self.descriptor, now)
                .and_then(|values| {
                    values.health(
                        ProviderHealthState::Healthy,
                        ProviderHealthReason::None,
                        ProviderRemediation::None,
                    )
                })
                .map_err(|_| self.failure(context.operation, now, FailureClass::Invariant))
        })
    }
}

impl TransportProvider for LocalTransportProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        local_transport_capabilities()
    }

    fn connect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { self.connect_inner(context, request).await })
    }

    fn listen<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        _request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { Err(self.unsupported(context.operation)) })
    }

    fn issue_binding<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        _request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { Err(self.unsupported(context.operation)) })
    }

    fn revoke_binding<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move { self.revoke_inner(context, request).await })
    }

    fn inspect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move { self.inspect_inner(context, request).await })
    }
}

pub fn local_transport_capabilities() -> ProviderCapabilitySet {
    ProviderCapabilitySet::new(vec![
        ProviderCapability(ProviderMethod::TransportConnect),
        ProviderCapability(ProviderMethod::TransportRevokeBinding),
        ProviderCapability(ProviderMethod::TransportInspect),
    ])
    .unwrap_or_else(|_| unreachable!("fixed local transport capabilities are valid"))
}

fn validate_configured_binding(
    descriptor: &ProviderDescriptor,
    kind: LocalTransportKind,
    binding: &TransportBinding,
) -> Result<(), LocalTransportConfigurationError> {
    if binding.provider_id() != &descriptor.provider_id
        || binding.provider_generation() != descriptor.registry_generation
        || binding.configuration_fingerprint() != &descriptor.configuration_schema_fingerprint
        || binding.configured_scope_digest() != &descriptor.configured_scope_digest
        || binding.scope().realm_id() != descriptor.placement.realm_id()
        || binding.kind() != kind
        || binding.capabilities() != kind.capability_profile()
    {
        Err(LocalTransportConfigurationError::BindingMismatch)
    } else {
        Ok(())
    }
}

fn connection_envelope_matches(
    connection: &EndpointConnection,
    request: &ProviderOperationRequest,
    binding: &TransportBinding,
    handle_id: &HandleId,
    reachability: ReachabilityEvidence,
) -> bool {
    connection.operation_id == request.context.operation_id
        && &connection.handle_id == handle_id
        && connection.binding_id == *binding.binding_id()
        && connection.kind == binding.kind()
        && connection.capabilities == binding.capabilities()
        && connection.reachability == reachability
}

fn observation_envelope_matches(
    observation: &EndpointObservation,
    request: &ProviderOperationRequest,
    active: &ActiveSnapshot,
) -> bool {
    observation.operation_id == request.context.operation_id
        && observation.handle_id == active.handle.handle_id
        && observation.binding_id == active.binding_id
        && observation.kind == active.connection.kind
        && observation.capabilities == active.connection.capabilities
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestDenial {
    Capability,
    Invalid,
    Unauthorized,
    Cancelled,
    Deadline,
}

impl RequestDenial {
    fn from_contract(error: ProviderContractError) -> Self {
        match error {
            ProviderContractError::RequestExpired
            | ProviderContractError::RequestLifetimeExceeded => Self::Deadline,
            ProviderContractError::CapabilityMismatch
            | ProviderContractError::ProviderTypeMismatch
            | ProviderContractError::MissingRequiredCapability => Self::Capability,
            ProviderContractError::ScopeMismatch
            | ProviderContractError::OperationBindingMismatch
            | ProviderContractError::PlacementMismatch => Self::Unauthorized,
            _ => Self::Invalid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureClass {
    Capability,
    Invalid,
    Unauthorized,
    Cancelled,
    Deadline,
    Unavailable,
    BoundExceeded,
    IdentityMismatch,
    GenerationMismatch,
    Invariant,
}

impl From<RequestDenial> for FailureClass {
    fn from(value: RequestDenial) -> Self {
        match value {
            RequestDenial::Capability => Self::Capability,
            RequestDenial::Invalid => Self::Invalid,
            RequestDenial::Unauthorized => Self::Unauthorized,
            RequestDenial::Cancelled => Self::Cancelled,
            RequestDenial::Deadline => Self::Deadline,
        }
    }
}

impl From<EndpointPortError> for FailureClass {
    fn from(value: EndpointPortError) -> Self {
        match value {
            EndpointPortError::Cancelled => Self::Cancelled,
            EndpointPortError::DeadlineExpired => Self::Deadline,
            EndpointPortError::Unavailable => Self::Unavailable,
            EndpointPortError::BoundExceeded => Self::BoundExceeded,
            EndpointPortError::IdentityMismatch => Self::IdentityMismatch,
            EndpointPortError::GenerationMismatch => Self::GenerationMismatch,
            EndpointPortError::InvariantViolation => Self::Invariant,
        }
    }
}

struct FailureShape {
    kind: ProviderFailureKind,
    retry: RetryClass,
    reason: ProviderHealthReason,
    remediation: ProviderRemediation,
}

impl FailureClass {
    const fn shape(self) -> FailureShape {
        match self {
            Self::Capability => FailureShape {
                kind: ProviderFailureKind::CapabilityDenied,
                retry: RetryClass::Never,
                reason: ProviderHealthReason::CapabilityMismatch,
                remediation: ProviderRemediation::RepairConfiguration,
            },
            Self::Invalid => FailureShape {
                kind: ProviderFailureKind::InvalidRequest,
                retry: RetryClass::Never,
                reason: ProviderHealthReason::ConfigurationMismatch,
                remediation: ProviderRemediation::RepairConfiguration,
            },
            Self::Unauthorized => FailureShape {
                kind: ProviderFailureKind::UnauthorizedScope,
                retry: RetryClass::Never,
                reason: ProviderHealthReason::IdentityMismatch,
                remediation: ProviderRemediation::ReEnrollPeer,
            },
            Self::Cancelled => FailureShape {
                kind: ProviderFailureKind::Cancelled,
                retry: RetryClass::Never,
                reason: ProviderHealthReason::ProviderDegraded,
                remediation: ProviderRemediation::None,
            },
            Self::Deadline => FailureShape {
                kind: ProviderFailureKind::DeadlineExpired,
                retry: RetryClass::SameOperation,
                reason: ProviderHealthReason::HealthTimeout,
                remediation: ProviderRemediation::RetryBounded,
            },
            Self::Unavailable => FailureShape {
                kind: ProviderFailureKind::Unavailable,
                retry: RetryClass::SameOperation,
                reason: ProviderHealthReason::SessionDisconnected,
                remediation: ProviderRemediation::RetryBounded,
            },
            Self::BoundExceeded => FailureShape {
                kind: ProviderFailureKind::Unavailable,
                retry: RetryClass::SameOperation,
                reason: ProviderHealthReason::QueuePressure,
                remediation: ProviderRemediation::RetryBounded,
            },
            Self::IdentityMismatch => FailureShape {
                kind: ProviderFailureKind::AdoptionRejected,
                retry: RetryClass::Never,
                reason: ProviderHealthReason::IdentityMismatch,
                remediation: ProviderRemediation::ReEnrollPeer,
            },
            Self::GenerationMismatch => FailureShape {
                kind: ProviderFailureKind::AdoptionRejected,
                retry: RetryClass::Never,
                reason: ProviderHealthReason::GenerationMismatch,
                remediation: ProviderRemediation::ReplaceGeneration,
            },
            Self::Invariant => FailureShape {
                kind: ProviderFailureKind::InvariantViolation,
                retry: RetryClass::Never,
                reason: ProviderHealthReason::CapabilityMismatch,
                remediation: ProviderRemediation::RepairConfiguration,
            },
        }
    }
}
