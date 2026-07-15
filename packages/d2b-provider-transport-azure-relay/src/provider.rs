use std::{error::Error, fmt, future::Future, sync::Arc, time::Duration};

use d2b_contracts::{
    v2_component_session::{EndpointRole, ServicePackage},
    v2_identity::ProviderType,
    v2_provider::{
        AdoptionState, AuthorizedProviderScope, HandleOwner, LeaseId, MAX_SAFE_JSON_INTEGER,
        MutationReceipt, MutationState, ObservationReason, ObservedLifecycleState, Provider,
        ProviderCallContext, ProviderCapability, ProviderCapabilitySet, ProviderDescriptor,
        ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle, ProviderHandleKind,
        ProviderHealth, ProviderHealthReason, ProviderHealthState, ProviderMethod,
        ProviderObservation, ProviderOperationContext, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlacement, ProviderRemediation, ProviderResult,
        ProviderTarget, RetryClass, TransportBindingId, TransportProvider,
    },
};
use d2b_provider::{ProviderClock, ProviderInstance, SystemProviderClock};
use d2b_provider_toolkit::ProviderValues;

use crate::port::{
    RelayAdoptRequest, RelayCloseOutcome, RelayCloseRequest, RelayControlPort,
    RelayExpectedResource, RelayInspectRequest, RelayInspection, RelayOpenRequest,
    RelayPortCapabilities, RelayPortFailure, RelayRendezvousId, RelayResource, RelayResourceState,
    RelayTransportLimits,
};

pub const AZURE_RELAY_IMPLEMENTATION_ID: &str = "azure-relay";

/// Opaque agent-local configuration for one Azure Relay transport provider.
#[derive(Clone, PartialEq, Eq)]
pub struct AzureRelayConfiguration {
    scope: AuthorizedProviderScope,
    transport_binding_id: TransportBindingId,
    rendezvous_id: RelayRendezvousId,
    connect_credential_lease_id: LeaseId,
    listen_credential_lease_id: LeaseId,
}

impl AzureRelayConfiguration {
    pub fn new(
        scope: AuthorizedProviderScope,
        transport_binding_id: TransportBindingId,
        rendezvous_id: RelayRendezvousId,
        connect_credential_lease_id: LeaseId,
        listen_credential_lease_id: LeaseId,
    ) -> Result<Self, AzureRelayProviderBuildError> {
        if connect_credential_lease_id == listen_credential_lease_id {
            return Err(AzureRelayProviderBuildError::CredentialRoleOverlap);
        }
        Ok(Self {
            scope,
            transport_binding_id,
            rendezvous_id,
            connect_credential_lease_id,
            listen_credential_lease_id,
        })
    }

    pub fn scope(&self) -> &AuthorizedProviderScope {
        &self.scope
    }

    pub fn transport_binding_id(&self) -> &TransportBindingId {
        &self.transport_binding_id
    }

    pub fn rendezvous_id(&self) -> &RelayRendezvousId {
        &self.rendezvous_id
    }

    pub fn connect_credential_lease_id(&self) -> &LeaseId {
        &self.connect_credential_lease_id
    }

    pub fn listen_credential_lease_id(&self) -> &LeaseId {
        &self.listen_credential_lease_id
    }
}

impl fmt::Debug for AzureRelayConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureRelayConfiguration")
            .field("scope", &"<redacted>")
            .field("transport_binding_id", &"<redacted>")
            .field("rendezvous_id", &"<redacted>")
            .field("credential_leases", &"<configured>")
            .finish()
    }
}

/// Construction failures contain no descriptor identifiers or configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureRelayProviderBuildError {
    InvalidDescriptor,
    WrongAuthority,
    WrongImplementation,
    WrongPlacement,
    WrongCapabilities,
    ScopeMismatch,
    CredentialRoleOverlap,
}

impl fmt::Display for AzureRelayProviderBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor => "invalid Azure Relay provider descriptor",
            Self::WrongAuthority => "Azure Relay descriptor has the wrong provider authority",
            Self::WrongImplementation => {
                "Azure Relay descriptor has the wrong implementation identifier"
            }
            Self::WrongPlacement => {
                "Azure Relay must be placed in its credential-owning provider agent"
            }
            Self::WrongCapabilities => "Azure Relay descriptor capabilities are not exact",
            Self::ScopeMismatch => "Azure Relay configuration is outside its provider-agent scope",
            Self::CredentialRoleOverlap => {
                "Azure Relay connect and listen credential leases must be distinct"
            }
        })
    }
}

impl Error for AzureRelayProviderBuildError {}

/// Exact live capabilities: connect/listen, idempotent close, and inspection.
///
/// Binding issuance remains outside this adapter because the canonical request
/// has no safe endpoint/credential input and the production path uses an
/// already-configured opaque binding.
pub fn azure_relay_capabilities() -> ProviderCapabilitySet {
    capabilities_for_port(RelayPortCapabilities::production())
}

fn capabilities_for_port(capabilities: RelayPortCapabilities) -> ProviderCapabilitySet {
    let mut methods = Vec::with_capacity(4);
    if capabilities.connect() {
        methods.push(ProviderCapability(ProviderMethod::TransportConnect));
    }
    if capabilities.listen() {
        methods.push(ProviderCapability(ProviderMethod::TransportListen));
    }
    if capabilities.close() {
        methods.push(ProviderCapability(ProviderMethod::TransportRevokeBinding));
    }
    if capabilities.inspect() && capabilities.adopt() {
        methods.push(ProviderCapability(ProviderMethod::TransportInspect));
    }
    ProviderCapabilitySet::new(methods).expect("the static Azure Relay capability set is valid")
}

/// Canonical async Azure Relay transport provider.
///
/// All Azure/credential behavior is injected through [`RelayControlPort`].
/// This type has no environment lookup, daemon call, broker call, endpoint URL,
/// token, SDK response, or free-form diagnostic surface.
pub struct AzureRelayTransportProvider {
    descriptor: ProviderDescriptor,
    configuration: AzureRelayConfiguration,
    port: Arc<dyn RelayControlPort>,
    clock: Arc<dyn ProviderClock>,
    limits: RelayTransportLimits,
}

impl AzureRelayTransportProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        configuration: AzureRelayConfiguration,
        port: Arc<dyn RelayControlPort>,
    ) -> Result<Self, AzureRelayProviderBuildError> {
        Self::with_clock(
            descriptor,
            configuration,
            port,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        configuration: AzureRelayConfiguration,
        port: Arc<dyn RelayControlPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, AzureRelayProviderBuildError> {
        validate_descriptor(&descriptor, &configuration, port.capabilities())?;
        Ok(Self {
            descriptor,
            configuration,
            port,
            clock,
            limits: RelayTransportLimits::production(),
        })
    }

    pub fn configuration(&self) -> &AzureRelayConfiguration {
        &self.configuration
    }

    pub const fn limits(&self) -> RelayTransportLimits {
        self.limits
    }

    pub fn instance(self: Arc<Self>) -> ProviderInstance {
        ProviderInstance::Transport(self)
    }

    /// Agent-local restart adoption under the canonical inspect capability.
    ///
    /// Transport has no serialized `transport.adopt` method. The owning agent
    /// may use this helper with an already-bound transport handle; both provider
    /// and resource generations, owner, scope, configuration, and resource
    /// identity are checked before and after the co-located port call.
    pub async fn adopt(
        &self,
        context: &ProviderCallContext<'_>,
        handle: &ProviderHandle,
    ) -> ProviderResult<ProviderObservation> {
        let deadline_ms =
            self.validate_operation_context(context, ProviderMethod::TransportInspect)?;
        self.validate_handle_for_adoption(context.operation, handle)?;
        let expected = RelayExpectedResource::new(
            self.descriptor.provider_id.clone(),
            handle.handle_id.clone(),
            handle.provider_generation,
            handle.resource_generation,
        );
        let request = RelayAdoptRequest::new(
            context.operation.binding(),
            self.configuration.transport_binding_id.clone(),
            self.configuration.rendezvous_id.clone(),
            expected.clone(),
            deadline_ms,
            self.limits,
        );
        let resource = self
            .await_port(
                context.operation,
                deadline_ms,
                self.port.adopt(request),
                PortCallClass::Observation,
                true,
            )
            .await?;
        self.validate_resource(&resource, Some(&expected), None, true, context.operation)?;
        self.observation(context.operation, Some(&resource), AdoptionState::Adopted)
    }

    fn now(&self) -> u64 {
        self.clock.now_unix_ms().min(MAX_SAFE_JSON_INTEGER)
    }

    fn failure(
        &self,
        context: &ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Transport,
            binding: context.binding(),
            correlation_id: context.correlation_id.clone(),
            occurred_at_unix_ms: self.now(),
            reason,
            remediation,
        }
    }

    fn invalid_request(&self, context: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::CapabilityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn validate_operation_context(
        &self,
        context: &ProviderCallContext<'_>,
        expected_method: ProviderMethod,
    ) -> Result<u32, ProviderFailure> {
        if context.cancelled {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::None,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.deadline_failure(context.operation, false));
        }
        context
            .validate()
            .map_err(|_| self.invalid_request(context.operation))?;
        if context.operation.scope != self.configuration.scope {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        if context.operation.method != expected_method {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        let now = self.now();
        if now >= context.operation.expires_at_unix_ms {
            return Err(self.deadline_failure(context.operation, false));
        }
        context
            .operation
            .validate(&self.descriptor, now)
            .map_err(|_| self.invalid_request(context.operation))?;
        let wall_remaining = context.operation.expires_at_unix_ms - now;
        let wall_remaining = u32::try_from(wall_remaining).unwrap_or(u32::MAX).max(1);
        Ok(context.monotonic_deadline_remaining_ms.min(wall_remaining))
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected_method: ProviderMethod,
    ) -> Result<u32, ProviderFailure> {
        if context.operation != &request.context {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        let deadline_ms = self.validate_operation_context(context, expected_method)?;
        if request.context.method != expected_method {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        request
            .validate_method(&self.descriptor, self.now(), expected_method)
            .map_err(|_| self.invalid_request(context.operation))?;
        Ok(deadline_ms)
    }

    fn validate_handle_for_adoption(
        &self,
        context: &ProviderOperationContext,
        handle: &ProviderHandle,
    ) -> ProviderResult<()> {
        let expected_owner = HandleOwner::Provider {
            realm_id: self.configuration.scope.realm_id().clone(),
            provider_id: self.descriptor.provider_id.clone(),
        };
        let valid = handle.validate().is_ok()
            && handle.kind == ProviderHandleKind::Transport
            && handle.provider_id == self.descriptor.provider_id
            && handle.provider_generation == self.descriptor.registry_generation
            && handle.realm_id == *self.configuration.scope.realm_id()
            && handle.workload_id.as_ref() == self.configuration.scope.workload_id()
            && handle.owner == expected_owner
            && handle.configuration_fingerprint == self.descriptor.configuration_schema_fingerprint;
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

    async fn await_port<T>(
        &self,
        context: &ProviderOperationContext,
        deadline_ms: u32,
        future: impl Future<Output = Result<T, RelayPortFailure>>,
        class: PortCallClass,
        adopting: bool,
    ) -> ProviderResult<T> {
        match tokio::time::timeout(Duration::from_millis(u64::from(deadline_ms)), future).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(failure)) => Err(self.port_failure(context, failure, adopting)),
            Err(_) => Err(self.deadline_failure(context, class.is_mutation())),
        }
    }

    fn deadline_failure(
        &self,
        context: &ProviderOperationContext,
        mutation: bool,
    ) -> ProviderFailure {
        if mutation {
            self.failure(
                context,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::HandshakeTimeout,
                ProviderRemediation::InspectProvider,
            )
        } else {
            self.failure(
                context,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            )
        }
    }

    fn port_failure(
        &self,
        context: &ProviderOperationContext,
        failure: RelayPortFailure,
        adopting: bool,
    ) -> ProviderFailure {
        let (kind, retry, reason, remediation) = match failure {
            RelayPortFailure::CredentialLeaseInvalid => (
                ProviderFailureKind::CredentialLeaseInvalid,
                RetryClass::Never,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::ReEnrollPeer,
            ),
            RelayPortFailure::AuthenticationFailed => (
                ProviderFailureKind::Unavailable,
                RetryClass::AfterInteraction,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::ReEnrollPeer,
            ),
            RelayPortFailure::HandshakeTimeout => (
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HandshakeTimeout,
                ProviderRemediation::RetryBounded,
            ),
            RelayPortFailure::Unavailable => (
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            RelayPortFailure::QueueFull => (
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::RetryBounded,
            ),
            RelayPortFailure::FrameTooLarge => (
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            RelayPortFailure::Cancelled => (
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::None,
            ),
            RelayPortFailure::CompletionAmbiguous => (
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::HealthStale,
                ProviderRemediation::InspectProvider,
            ),
            RelayPortFailure::BindingMismatch | RelayPortFailure::IdentityMismatch => (
                if adopting {
                    ProviderFailureKind::AdoptionRejected
                } else {
                    ProviderFailureKind::InvariantViolation
                },
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::InspectProvider,
            ),
            RelayPortFailure::GenerationMismatch => (
                if adopting {
                    ProviderFailureKind::AdoptionRejected
                } else {
                    ProviderFailureKind::RegistryChanged
                },
                RetryClass::Never,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ),
        };
        self.failure(context, kind, retry, reason, remediation)
    }

    fn validate_resource(
        &self,
        resource: &RelayResource,
        expected: Option<&RelayExpectedResource>,
        required_state: Option<RelayResourceState>,
        adopting: bool,
        context: &ProviderOperationContext,
    ) -> ProviderResult<()> {
        let identity_matches = resource.provider_id() == &self.descriptor.provider_id
            && resource.transport_binding_id() == &self.configuration.transport_binding_id
            && resource.rendezvous_id() == &self.configuration.rendezvous_id;
        if !identity_matches {
            return Err(self.port_failure(context, RelayPortFailure::IdentityMismatch, adopting));
        }
        if resource.provider_generation() != self.descriptor.registry_generation {
            return Err(self.port_failure(context, RelayPortFailure::GenerationMismatch, adopting));
        }
        if let Some(expected) = expected {
            if resource.provider_id() != expected.provider_id()
                || resource.handle_id() != expected.handle_id()
            {
                return Err(self.port_failure(context, RelayPortFailure::IdentityMismatch, true));
            }
            if resource.provider_generation() != expected.provider_generation()
                || resource.resource_generation() != expected.resource_generation()
            {
                return Err(self.port_failure(context, RelayPortFailure::GenerationMismatch, true));
            }
        }
        if required_state.is_some_and(|state| resource.state() != state)
            || adopting
                && !matches!(
                    resource.state(),
                    RelayResourceState::Connected | RelayResourceState::Listening
                )
        {
            return Err(self.port_failure(context, RelayPortFailure::IdentityMismatch, adopting));
        }
        if resource
            .expires_at_unix_ms()
            .is_some_and(|expires| expires <= self.now())
        {
            return Err(self.port_failure(context, RelayPortFailure::GenerationMismatch, adopting));
        }
        Ok(())
    }

    fn handle(
        &self,
        request: &ProviderOperationRequest,
        resource: &RelayResource,
    ) -> ProviderResult<ProviderHandle> {
        let values = ProviderValues::new(&self.descriptor, self.now())
            .map_err(|_| self.invalid_request(&request.context))?;
        values
            .handle_from_request(
                request,
                resource.handle_id().clone(),
                values.provider_owner(request.target.realm_id()),
                resource.resource_generation(),
                resource.expires_at_unix_ms(),
            )
            .map_err(|_| {
                self.failure(
                    &request.context,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })
    }

    fn observation(
        &self,
        context: &ProviderOperationContext,
        resource: Option<&RelayResource>,
        adoption: AdoptionState,
    ) -> ProviderResult<ProviderObservation> {
        let (handle_id, resource_generation, lifecycle, health_state, health_reason, remediation) =
            match resource {
                None => (
                    None,
                    None,
                    ObservedLifecycleState::Stopped,
                    ProviderHealthState::Healthy,
                    ProviderHealthReason::None,
                    ProviderRemediation::None,
                ),
                Some(resource) => {
                    let (lifecycle, health_state, health_reason, remediation) =
                        match resource.state() {
                            RelayResourceState::Connected | RelayResourceState::Listening => (
                                ObservedLifecycleState::Running,
                                ProviderHealthState::Healthy,
                                ProviderHealthReason::None,
                                ProviderRemediation::None,
                            ),
                            RelayResourceState::Closed => (
                                ObservedLifecycleState::Released,
                                ProviderHealthState::Healthy,
                                ProviderHealthReason::None,
                                ProviderRemediation::None,
                            ),
                            RelayResourceState::Unknown => (
                                ObservedLifecycleState::Unknown,
                                ProviderHealthState::Degraded,
                                ProviderHealthReason::ProviderDegraded,
                                ProviderRemediation::InspectProvider,
                            ),
                        };
                    (
                        Some(resource.handle_id().clone()),
                        Some(resource.resource_generation()),
                        lifecycle,
                        health_state,
                        health_reason,
                        remediation,
                    )
                }
            };
        let values = ProviderValues::new(&self.descriptor, self.now())
            .map_err(|_| self.invalid_request(context))?;
        let health = values
            .health(health_state, health_reason, remediation)
            .map_err(|_| self.invalid_request(context))?;
        let observation = ProviderObservation {
            provider_id: self.descriptor.provider_id.clone(),
            provider_generation: self.descriptor.registry_generation,
            realm_id: context.scope.realm_id().clone(),
            workload_id: context.scope.workload_id().cloned(),
            handle_id,
            resource_generation,
            observed_at_unix_ms: self.now(),
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

    async fn open(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        kind: OpenKind,
    ) -> ProviderResult<ProviderHandle> {
        let method = kind.method();
        let deadline_ms = self.validate_request(context, request, method)?;
        let credential_lease_id = match kind {
            OpenKind::Connect => self.configuration.connect_credential_lease_id.clone(),
            OpenKind::Listen => self.configuration.listen_credential_lease_id.clone(),
        };
        let port_request = RelayOpenRequest::new(
            request.context.binding(),
            self.configuration.transport_binding_id.clone(),
            self.configuration.rendezvous_id.clone(),
            credential_lease_id,
            deadline_ms,
            self.limits,
        );
        let resource = match kind {
            OpenKind::Connect => {
                self.await_port(
                    &request.context,
                    deadline_ms,
                    self.port.connect(port_request),
                    PortCallClass::Mutation,
                    false,
                )
                .await?
            }
            OpenKind::Listen => {
                self.await_port(
                    &request.context,
                    deadline_ms,
                    self.port.listen(port_request),
                    PortCallClass::Mutation,
                    false,
                )
                .await?
            }
        };
        self.validate_resource(
            &resource,
            None,
            Some(kind.resource_state()),
            false,
            &request.context,
        )?;
        self.handle(request, &resource)
    }

    async fn inspect_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderObservation> {
        let deadline_ms =
            self.validate_request(context, request, ProviderMethod::TransportInspect)?;
        if let ProviderTarget::Handle {
            handle_id,
            handle_generation,
            ..
        } = &request.target
        {
            let expected = RelayExpectedResource::new(
                self.descriptor.provider_id.clone(),
                handle_id.clone(),
                self.descriptor.registry_generation,
                *handle_generation,
            );
            let port_request = RelayAdoptRequest::new(
                request.context.binding(),
                self.configuration.transport_binding_id.clone(),
                self.configuration.rendezvous_id.clone(),
                expected.clone(),
                deadline_ms,
                self.limits,
            );
            let resource = self
                .await_port(
                    &request.context,
                    deadline_ms,
                    self.port.adopt(port_request),
                    PortCallClass::Observation,
                    true,
                )
                .await?;
            self.validate_resource(&resource, Some(&expected), None, true, &request.context)?;
            return self.observation(&request.context, Some(&resource), AdoptionState::Adopted);
        }

        let port_request = RelayInspectRequest::new(
            request.context.binding(),
            self.configuration.transport_binding_id.clone(),
            self.configuration.rendezvous_id.clone(),
            deadline_ms,
            self.limits,
        );
        let inspection = self
            .await_port(
                &request.context,
                deadline_ms,
                self.port.inspect(port_request),
                PortCallClass::Observation,
                false,
            )
            .await?;
        match inspection {
            RelayInspection::Absent => {
                self.observation(&request.context, None, AdoptionState::NotAttempted)
            }
            RelayInspection::Present(resource) => {
                self.validate_resource(&resource, None, None, false, &request.context)?;
                self.observation(
                    &request.context,
                    Some(&resource),
                    AdoptionState::NotAttempted,
                )
            }
        }
    }

    async fn close(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<MutationReceipt> {
        let deadline_ms =
            self.validate_request(context, request, ProviderMethod::TransportRevokeBinding)?;
        let ProviderOperationInput::TransportBinding {
            transport_binding_id,
        } = &request.input
        else {
            return Err(self.invalid_request(&request.context));
        };
        if transport_binding_id != &self.configuration.transport_binding_id {
            return Err(self.failure(
                &request.context,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        let port_request = RelayCloseRequest::new(
            request.context.binding(),
            self.configuration.transport_binding_id.clone(),
            self.configuration.rendezvous_id.clone(),
            deadline_ms,
            self.limits,
        );
        let outcome = self
            .await_port(
                &request.context,
                deadline_ms,
                self.port.close(port_request),
                PortCallClass::Mutation,
                false,
            )
            .await?;
        let state = match outcome {
            RelayCloseOutcome::Closed => MutationState::Applied,
            RelayCloseOutcome::AlreadyClosed => MutationState::AlreadyApplied,
            RelayCloseOutcome::NotFound => MutationState::NotApplicable,
        };
        ProviderValues::new(&self.descriptor, self.now())
            .and_then(|values| values.receipt(&request.context, state))
            .map_err(|_| {
                self.failure(
                    &request.context,
                    ProviderFailureKind::InvariantViolation,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                )
            })
    }
}

impl fmt::Debug for AzureRelayTransportProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureRelayTransportProvider")
            .field("provider_generation", &self.descriptor.registry_generation)
            .field("implementation", &AZURE_RELAY_IMPLEMENTATION_ID)
            .field("configuration", &self.configuration)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Provider for AzureRelayTransportProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            self.validate_operation_context(context, context.operation.method)?;
            ProviderValues::new(&self.descriptor, self.now())
                .and_then(|values| {
                    values.health(
                        ProviderHealthState::Healthy,
                        ProviderHealthReason::None,
                        ProviderRemediation::None,
                    )
                })
                .map_err(|_| self.invalid_request(context.operation))
        })
    }
}

impl TransportProvider for AzureRelayTransportProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn connect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { self.open(context, request, OpenKind::Connect).await })
    }

    fn listen<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { self.open(context, request, OpenKind::Listen).await })
    }

    fn issue_binding<'a>(
        &'a self,
        _context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move {
            Err(self.failure(
                &request.context,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ))
        })
    }

    fn revoke_binding<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move { self.close(context, request).await })
    }

    fn inspect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move { self.inspect_request(context, request).await })
    }
}

#[derive(Debug, Clone, Copy)]
enum OpenKind {
    Connect,
    Listen,
}

impl OpenKind {
    const fn method(self) -> ProviderMethod {
        match self {
            Self::Connect => ProviderMethod::TransportConnect,
            Self::Listen => ProviderMethod::TransportListen,
        }
    }

    const fn resource_state(self) -> RelayResourceState {
        match self {
            Self::Connect => RelayResourceState::Connected,
            Self::Listen => RelayResourceState::Listening,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PortCallClass {
    Mutation,
    Observation,
}

impl PortCallClass {
    const fn is_mutation(self) -> bool {
        matches!(self, Self::Mutation)
    }
}

fn validate_descriptor(
    descriptor: &ProviderDescriptor,
    configuration: &AzureRelayConfiguration,
    port_capabilities: RelayPortCapabilities,
) -> Result<(), AzureRelayProviderBuildError> {
    descriptor
        .validate()
        .map_err(|_| AzureRelayProviderBuildError::InvalidDescriptor)?;
    if descriptor.provider_type() != ProviderType::Transport {
        return Err(AzureRelayProviderBuildError::WrongAuthority);
    }
    if descriptor.implementation_id.as_str() != AZURE_RELAY_IMPLEMENTATION_ID {
        return Err(AzureRelayProviderBuildError::WrongImplementation);
    }
    if !port_capabilities.connect()
        || !port_capabilities.close()
        || !port_capabilities.inspect()
        || !port_capabilities.adopt()
        || descriptor.capabilities != capabilities_for_port(port_capabilities)
    {
        return Err(AzureRelayProviderBuildError::WrongCapabilities);
    }
    let ProviderPlacement::ProviderAgent {
        realm_id,
        workload_id,
        role_id,
        endpoint_role,
        service,
        agent_generation,
    } = &descriptor.placement
    else {
        return Err(AzureRelayProviderBuildError::WrongPlacement);
    };
    if *endpoint_role != EndpointRole::ProviderAgent
        || *service != ServicePackage::ProviderV2
        || *agent_generation != descriptor.registry_generation
    {
        return Err(AzureRelayProviderBuildError::WrongPlacement);
    }
    let scope_matches = match &configuration.scope {
        AuthorizedProviderScope::Workload {
            realm_id: scope_realm,
            workload_id: scope_workload,
        } => scope_realm == realm_id && scope_workload == workload_id,
        AuthorizedProviderScope::WorkloadRole {
            realm_id: scope_realm,
            workload_id: scope_workload,
            role_id: scope_role,
        } => scope_realm == realm_id && scope_workload == workload_id && scope_role == role_id,
        AuthorizedProviderScope::Realm { .. } => false,
    };
    if !scope_matches {
        return Err(AzureRelayProviderBuildError::ScopeMismatch);
    }
    Ok(())
}
