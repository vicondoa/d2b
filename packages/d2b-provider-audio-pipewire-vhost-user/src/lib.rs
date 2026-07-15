//! PipeWire/vhost-user-sound audio provider.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::ProviderType,
    v2_provider::{
        AdoptionRequest, AdoptionState, AudioChannel, AudioDirection, AudioProvider,
        AuthorizedProviderScope, Generation, HandleId, ImplementationId, MutationReceipt,
        MutationState, ObservationReason, ObservedLifecycleState, OperationBinding, Provider,
        ProviderCallContext, ProviderCapability, ProviderCapabilitySet, ProviderContractError,
        ProviderDescriptor, ProviderFactoryKey, ProviderFailure, ProviderFailureKind,
        ProviderFuture, ProviderHandle, ProviderHandleKind, ProviderHealth, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderObservation, ProviderOperationContext,
        ProviderOperationInput, ProviderOperationRequest, ProviderPlacement, ProviderRemediation,
        ProviderTarget, RetryClass,
    },
};
use d2b_host::audio_argv::{AudioArgvInput, generate_audio_argv};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};
use d2b_provider_toolkit::ProviderValues;

const GENERATED_ID_DIGEST_CHARS: usize = 24;
pub const IMPLEMENTATION_ID: &str = "pipewire-vhost-user";

pub fn implementation_id() -> ImplementationId {
    ImplementationId::parse(IMPLEMENTATION_ID)
        .unwrap_or_else(|_| unreachable!("static implementation id is valid"))
}

pub fn factory_key() -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Audio,
        implementation_id: implementation_id(),
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioRouteId(String);

impl AudioRouteId {
    pub fn parse(value: impl Into<String>) -> Result<Self, AudioProviderBuildError> {
        parse_generated_id(value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AudioRouteId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AudioRouteId(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioRoleId(String);

impl AudioRoleId {
    pub fn parse(value: impl Into<String>) -> Result<Self, AudioProviderBuildError> {
        parse_generated_id(value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AudioRoleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AudioRoleId(<redacted>)")
    }
}

fn parse_generated_id(value: String) -> Result<String, AudioProviderBuildError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(value)
    } else {
        Err(AudioProviderBuildError::InvalidGeneratedId)
    }
}

#[derive(Clone)]
pub struct AudioConfiguration {
    argv: AudioArgvInput,
}

impl fmt::Debug for AudioConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioConfiguration")
            .field("backend", &self.argv.backend)
            .finish_non_exhaustive()
    }
}

impl AudioConfiguration {
    pub fn new(argv: AudioArgvInput) -> Result<Self, AudioProviderBuildError> {
        let configuration = Self { argv };
        configuration.validate()?;
        Ok(configuration)
    }

    fn validate(&self) -> Result<(), AudioProviderBuildError> {
        if !self.argv.extra_args.is_empty() || generate_audio_argv(&self.argv).is_err() {
            Err(AudioProviderBuildError::InvalidTypedBuilder)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioImplementation {
    PipewireVhostUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioState {
    pub channel: AudioChannel,
    pub direction: AudioDirection,
    pub mute: Option<bool>,
    pub volume: Option<u8>,
}

impl AudioState {
    fn from_input(input: &ProviderOperationInput) -> Result<Self, ProviderContractError> {
        let ProviderOperationInput::AudioState {
            channel,
            direction,
            mute,
            volume,
        } = input
        else {
            return Err(ProviderContractError::OperationInputMismatch);
        };
        input.validate()?;
        Ok(Self {
            channel: *channel,
            direction: *direction,
            mute: *mute,
            volume: *volume,
        })
    }
}

#[derive(Clone)]
pub struct AudioCall {
    operation: ProviderOperationContext,
    peer_role: EndpointRole,
    monotonic_deadline_remaining_ms: u32,
}

impl fmt::Debug for AudioCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioCall")
            .field("method", &self.operation.method)
            .field("provider_generation", &self.operation.provider_generation)
            .field("peer_role", &self.peer_role)
            .field(
                "monotonic_deadline_remaining_ms",
                &self.monotonic_deadline_remaining_ms,
            )
            .finish_non_exhaustive()
    }
}

impl AudioCall {
    pub fn operation(&self) -> &ProviderOperationContext {
        &self.operation
    }

    pub fn binding(&self) -> OperationBinding {
        self.operation.binding()
    }

    pub fn scope(&self) -> &AuthorizedProviderScope {
        &self.operation.scope
    }

    pub const fn monotonic_deadline_remaining_ms(&self) -> u32 {
        self.monotonic_deadline_remaining_ms
    }

    fn with_deadline(&self, monotonic_deadline_remaining_ms: u32) -> Self {
        Self {
            operation: self.operation.clone(),
            peer_role: self.peer_role,
            monotonic_deadline_remaining_ms,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AudioSessionPlan {
    pub implementation: AudioImplementation,
    pub route_id: AudioRouteId,
    pub role_id: AudioRoleId,
}

impl fmt::Debug for AudioSessionPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioSessionPlan")
            .field("implementation", &self.implementation)
            .field("route_id", &self.route_id)
            .field("role_id", &self.role_id)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPlanOutcome {
    Planned,
    AlreadyPlanned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioEnsureOutcome {
    pub handle_id: HandleId,
    pub resource_generation: Generation,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AudioInspection {
    pub handle: Option<ProviderHandle>,
    pub state: Option<AudioState>,
    pub lifecycle: ObservedLifecycleState,
    pub reason: ObservationReason,
    pub health_state: ProviderHealthState,
    pub health_reason: ProviderHealthReason,
    pub remediation: ProviderRemediation,
}

impl fmt::Debug for AudioInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioInspection")
            .field("has_handle", &self.handle.is_some())
            .field("state", &self.state)
            .field("lifecycle", &self.lifecycle)
            .field("reason", &self.reason)
            .field("health_state", &self.health_state)
            .field("health_reason", &self.health_reason)
            .field("remediation", &self.remediation)
            .finish()
    }
}

impl AudioInspection {
    pub fn ready(handle: Option<ProviderHandle>, state: Option<AudioState>) -> Self {
        Self {
            handle,
            state,
            lifecycle: ObservedLifecycleState::Ready,
            reason: ObservationReason::None,
            health_state: ProviderHealthState::Healthy,
            health_reason: ProviderHealthReason::None,
            remediation: ProviderRemediation::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioAdoptionOutcome {
    Adopted,
    Rejected(ObservationReason),
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioHealth {
    pub state: ProviderHealthState,
    pub reason: ProviderHealthReason,
    pub remediation: ProviderRemediation,
}

impl AudioHealth {
    pub const fn healthy() -> Self {
        Self {
            state: ProviderHealthState::Healthy,
            reason: ProviderHealthReason::None,
            remediation: ProviderRemediation::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPortError {
    Denied,
    Unavailable,
    Stale,
    Ambiguous,
    Cancelled,
}

#[async_trait]
pub trait AudioEffectPort: Send + Sync {
    async fn plan(
        &self,
        context: AudioCall,
        plan: AudioSessionPlan,
    ) -> Result<AudioPlanOutcome, AudioPortError>;

    async fn ensure(
        &self,
        context: AudioCall,
        plan: AudioSessionPlan,
    ) -> Result<AudioEnsureOutcome, AudioPortError>;

    async fn set_state(
        &self,
        context: AudioCall,
        target: ProviderTarget,
        state: AudioState,
    ) -> Result<AudioInspection, AudioPortError>;

    async fn destroy(
        &self,
        context: AudioCall,
        target: ProviderTarget,
    ) -> Result<MutationState, AudioPortError>;
}

#[async_trait]
pub trait AudioQueryPort: Send + Sync {
    async fn health(&self, context: AudioCall) -> Result<AudioHealth, AudioPortError>;

    async fn inspect(
        &self,
        context: AudioCall,
        target: ProviderTarget,
    ) -> Result<AudioInspection, AudioPortError>;

    async fn adopt(
        &self,
        context: AudioCall,
        request: AdoptionRequest,
    ) -> Result<AudioAdoptionOutcome, AudioPortError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioProviderBuildError {
    Contract(ProviderContractError),
    WrongProviderType,
    WrongImplementation,
    WrongPlacement,
    CapabilityMismatch,
    InvalidTypedBuilder,
    InvalidGeneratedId,
}

impl From<ProviderContractError> for AudioProviderBuildError {
    fn from(value: ProviderContractError) -> Self {
        Self::Contract(value)
    }
}

#[derive(Clone)]
pub struct PipewireVhostUserAudioFactory {
    configuration: AudioConfiguration,
    effects: Arc<dyn AudioEffectPort>,
    queries: Arc<dyn AudioQueryPort>,
    clock: Arc<dyn ProviderClock>,
}

pub type Factory = PipewireVhostUserAudioFactory;

impl fmt::Debug for PipewireVhostUserAudioFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PipewireVhostUserAudioFactory")
            .field("key", &factory_key())
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

impl PipewireVhostUserAudioFactory {
    pub fn new(
        configuration: AudioConfiguration,
        effects: Arc<dyn AudioEffectPort>,
        queries: Arc<dyn AudioQueryPort>,
    ) -> Self {
        Self::with_clock(
            configuration,
            effects,
            queries,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn with_clock(
        configuration: AudioConfiguration,
        effects: Arc<dyn AudioEffectPort>,
        queries: Arc<dyn AudioQueryPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Self {
        Self {
            configuration,
            effects,
            queries,
            clock,
        }
    }
}

impl ProviderFactory for PipewireVhostUserAudioFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Audio
            || descriptor.implementation_id != implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        PipewireVhostUserAudioProvider::with_clock(
            descriptor.clone(),
            self.configuration.clone(),
            self.effects.clone(),
            self.queries.clone(),
            self.clock.clone(),
        )
        .map(|provider| ProviderInstance::Audio(Arc::new(provider)))
        .map_err(|_| FactoryError::Rejected)
    }
}

#[derive(Clone)]
pub struct PipewireVhostUserAudioProvider {
    descriptor: ProviderDescriptor,
    configuration: AudioConfiguration,
    effects: Arc<dyn AudioEffectPort>,
    queries: Arc<dyn AudioQueryPort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for PipewireVhostUserAudioProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PipewireVhostUserAudioProvider")
            .field("provider_generation", &self.descriptor.registry_generation)
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

impl PipewireVhostUserAudioProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        configuration: AudioConfiguration,
        effects: Arc<dyn AudioEffectPort>,
        queries: Arc<dyn AudioQueryPort>,
    ) -> Result<Self, AudioProviderBuildError> {
        Self::with_clock(
            descriptor,
            configuration,
            effects,
            queries,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        configuration: AudioConfiguration,
        effects: Arc<dyn AudioEffectPort>,
        queries: Arc<dyn AudioQueryPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, AudioProviderBuildError> {
        descriptor.validate()?;
        if descriptor.provider_type() != ProviderType::Audio {
            return Err(AudioProviderBuildError::WrongProviderType);
        }
        if descriptor.implementation_id != implementation_id() {
            return Err(AudioProviderBuildError::WrongImplementation);
        }
        if !matches!(
            descriptor.placement,
            ProviderPlacement::TrustedFirstPartyInProcess { .. }
        ) {
            return Err(AudioProviderBuildError::WrongPlacement);
        }
        if descriptor.capabilities != live_audio_capabilities()? {
            return Err(AudioProviderBuildError::CapabilityMismatch);
        }
        configuration.validate()?;
        Ok(Self {
            descriptor,
            configuration,
            effects,
            queries,
            clock,
        })
    }

    fn expected_peer_role(&self) -> EndpointRole {
        match &self.descriptor.placement {
            ProviderPlacement::TrustedFirstPartyInProcess {
                controller_role, ..
            } => *controller_role,
            ProviderPlacement::ProviderAgent { endpoint_role, .. } => *endpoint_role,
        }
    }

    fn now_unix_ms(&self) -> u64 {
        self.clock
            .now_unix_ms()
            .min(d2b_contracts::v2_provider::MAX_SAFE_JSON_INTEGER)
    }

    fn failure(
        &self,
        operation: &ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        let mut binding = operation.binding();
        binding.provider_id = self.descriptor.provider_id.clone();
        binding.provider_generation = self.descriptor.registry_generation;
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Audio,
            binding,
            correlation_id: operation.correlation_id.clone(),
            occurred_at_unix_ms: self.now_unix_ms(),
            reason,
            remediation,
        }
    }

    fn invalid_request(&self, operation: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn deadline_failure(&self, operation: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::DeadlineExpired,
            RetryClass::SameOperation,
            ProviderHealthReason::HealthTimeout,
            ProviderRemediation::RetryBounded,
        )
    }

    fn validate_call(
        &self,
        context: &ProviderCallContext<'_>,
        operation: &ProviderOperationContext,
        expected: ProviderMethod,
    ) -> Result<AudioCall, ProviderFailure> {
        if context.cancelled {
            return Err(self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.deadline_failure(operation));
        }
        if context.operation != operation
            || context.peer_role != self.expected_peer_role()
            || context.validate().is_err()
            || operation
                .validate(&self.descriptor, self.now_unix_ms())
                .is_err()
            || operation.method != expected
        {
            return Err(self.invalid_request(operation));
        }
        Ok(AudioCall {
            operation: operation.clone(),
            peer_role: context.peer_role,
            monotonic_deadline_remaining_ms: context.monotonic_deadline_remaining_ms,
        })
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
    ) -> Result<AudioCall, ProviderFailure> {
        let call = self.validate_call(context, &request.context, expected)?;
        if request
            .validate_method(&self.descriptor, self.now_unix_ms(), expected)
            .is_err()
        {
            return Err(self.invalid_request(&request.context));
        }
        Ok(call)
    }

    async fn invoke<T, F>(
        &self,
        operation: &ProviderOperationContext,
        deadline_ms: u32,
        future: F,
    ) -> Result<T, ProviderFailure>
    where
        F: Future<Output = Result<T, AudioPortError>> + Send,
    {
        match tokio::time::timeout(Duration::from_millis(u64::from(deadline_ms)), future).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(self.port_failure(operation, error)),
            Err(_) => Err(self.deadline_failure(operation)),
        }
    }

    fn port_failure(
        &self,
        operation: &ProviderOperationContext,
        error: AudioPortError,
    ) -> ProviderFailure {
        match error {
            AudioPortError::Denied => self.failure(
                operation,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            AudioPortError::Unavailable => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
            AudioPortError::Stale => self.failure(
                operation,
                ProviderFailureKind::AdoptionRejected,
                RetryClass::Never,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ),
            AudioPortError::Ambiguous => self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::AdoptionAmbiguous,
                ProviderRemediation::InspectProvider,
            ),
            AudioPortError::Cancelled => self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
        }
    }

    fn values(
        &self,
        operation: &ProviderOperationContext,
    ) -> Result<ProviderValues, ProviderFailure> {
        ProviderValues::new(&self.descriptor, self.now_unix_ms())
            .map_err(|_| self.invalid_request(operation))
    }

    fn session_plan(
        &self,
        operation: &ProviderOperationContext,
    ) -> Result<AudioSessionPlan, ProviderFailure> {
        let digest = operation.request_digest.as_str();
        let suffix = digest
            .get(..GENERATED_ID_DIGEST_CHARS)
            .ok_or_else(|| self.invalid_request(operation))?;
        Ok(AudioSessionPlan {
            implementation: AudioImplementation::PipewireVhostUser,
            route_id: AudioRouteId::parse(format!("audio-route-{suffix}"))
                .map_err(|_| self.invalid_request(operation))?,
            role_id: AudioRoleId::parse(format!("audio-role-{suffix}"))
                .map_err(|_| self.invalid_request(operation))?,
        })
    }

    fn validate_inspection(
        &self,
        operation: &ProviderOperationContext,
        target: &ProviderTarget,
        inspection: &AudioInspection,
    ) -> Result<(), ProviderFailure> {
        let Some(handle) = inspection.handle.as_ref() else {
            return if matches!(target, ProviderTarget::Workload { .. }) {
                Ok(())
            } else {
                Err(self.invalid_request(operation))
            };
        };
        if handle.validate().is_err()
            || handle.kind != ProviderHandleKind::Audio
            || handle.provider_id != self.descriptor.provider_id
            || handle.provider_generation != self.descriptor.registry_generation
            || handle.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || !target_matches_handle(target, handle)
        {
            Err(self.invalid_request(operation))
        } else {
            Ok(())
        }
    }
}

pub fn live_audio_capabilities() -> Result<ProviderCapabilitySet, ProviderContractError> {
    ProviderCapabilitySet::new(vec![
        ProviderCapability(ProviderMethod::AudioOpen),
        ProviderCapability(ProviderMethod::AudioSetState),
        ProviderCapability(ProviderMethod::AudioInspect),
        ProviderCapability(ProviderMethod::AudioAdopt),
        ProviderCapability(ProviderMethod::AudioClose),
    ])
}

impl Provider for PipewireVhostUserAudioProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            let call = self.validate_call(context, context.operation, context.operation.method)?;
            let deadline = call.monotonic_deadline_remaining_ms();
            let health = self
                .invoke(context.operation, deadline, self.queries.health(call))
                .await?;
            self.values(context.operation)?
                .health(health.state, health.reason, health.remediation)
                .map_err(|_| self.invalid_request(context.operation))
        })
    }
}

impl AudioProvider for PipewireVhostUserAudioProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn open<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move {
            let call = self.validate_request(context, request, ProviderMethod::AudioOpen)?;
            if !matches!(request.target, ProviderTarget::Workload { .. }) {
                return Err(self.invalid_request(&request.context));
            }
            let plan = self.session_plan(&request.context)?;
            let started = Instant::now();
            let total_ms = call.monotonic_deadline_remaining_ms();
            self.invoke(
                &request.context,
                total_ms,
                self.effects.plan(call.clone(), plan.clone()),
            )
            .await?;
            let elapsed_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
            let remaining_ms = total_ms.saturating_sub(elapsed_ms);
            if remaining_ms == 0 {
                return Err(self.deadline_failure(&request.context));
            }
            let outcome = self
                .invoke(
                    &request.context,
                    remaining_ms,
                    self.effects.ensure(call.with_deadline(remaining_ms), plan),
                )
                .await?;
            let values = self.values(&request.context)?;
            values
                .handle_from_request(
                    request,
                    outcome.handle_id,
                    values.provider_owner(request.target.realm_id()),
                    outcome.resource_generation,
                    None,
                )
                .map_err(|_| self.invalid_request(&request.context))
        })
    }

    fn set_state<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move {
            let call = self.validate_request(context, request, ProviderMethod::AudioSetState)?;
            if matches!(request.target, ProviderTarget::Realm { .. }) {
                return Err(self.invalid_request(&request.context));
            }
            let state = AudioState::from_input(&request.input)
                .map_err(|_| self.invalid_request(&request.context))?;
            let deadline = call.monotonic_deadline_remaining_ms();
            let inspection = self
                .invoke(
                    &request.context,
                    deadline,
                    self.effects.set_state(call, request.target.clone(), state),
                )
                .await?;
            self.validate_inspection(&request.context, &request.target, &inspection)?;
            if inspection.state != Some(state) {
                return Err(self.invalid_request(&request.context));
            }
            self.values(&request.context)?
                .observation(
                    &request.context,
                    inspection.handle.as_ref(),
                    inspection.lifecycle,
                    AdoptionState::NotAttempted,
                    inspection.reason,
                    inspection.health_state,
                    inspection.health_reason,
                    inspection.remediation,
                )
                .map_err(|_| self.invalid_request(&request.context))
        })
    }

    fn inspect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move {
            let call = self.validate_request(context, request, ProviderMethod::AudioInspect)?;
            if matches!(request.target, ProviderTarget::Realm { .. }) {
                return Err(self.invalid_request(&request.context));
            }
            let deadline = call.monotonic_deadline_remaining_ms();
            let inspection = self
                .invoke(
                    &request.context,
                    deadline,
                    self.queries.inspect(call, request.target.clone()),
                )
                .await?;
            self.validate_inspection(&request.context, &request.target, &inspection)?;
            self.values(&request.context)?
                .observation(
                    &request.context,
                    inspection.handle.as_ref(),
                    inspection.lifecycle,
                    AdoptionState::NotAttempted,
                    inspection.reason,
                    inspection.health_state,
                    inspection.health_reason,
                    inspection.remediation,
                )
                .map_err(|_| self.invalid_request(&request.context))
        })
    }

    fn adopt<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a AdoptionRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move {
            let call = self.validate_call(context, &request.context, ProviderMethod::AudioAdopt)?;
            if request
                .validate(&self.descriptor, self.now_unix_ms())
                .is_err()
                || request.handle.kind != ProviderHandleKind::Audio
            {
                return Err(self.invalid_request(&request.context));
            }
            let deadline = call.monotonic_deadline_remaining_ms();
            let outcome = self
                .invoke(
                    &request.context,
                    deadline,
                    self.queries.adopt(call, request.clone()),
                )
                .await?;
            let values = self.values(&request.context)?;
            let result = match outcome {
                AudioAdoptionOutcome::Adopted => values.observation(
                    &request.context,
                    Some(&request.handle),
                    ObservedLifecycleState::Ready,
                    AdoptionState::Adopted,
                    ObservationReason::None,
                    ProviderHealthState::Healthy,
                    ProviderHealthReason::None,
                    ProviderRemediation::None,
                ),
                AudioAdoptionOutcome::Rejected(reason)
                    if !matches!(
                        reason,
                        ObservationReason::None | ObservationReason::MultipleCandidates
                    ) =>
                {
                    values.observation(
                        &request.context,
                        Some(&request.handle),
                        ObservedLifecycleState::Unknown,
                        AdoptionState::Rejected,
                        reason,
                        ProviderHealthState::Healthy,
                        ProviderHealthReason::None,
                        ProviderRemediation::None,
                    )
                }
                AudioAdoptionOutcome::Ambiguous => values.observation(
                    &request.context,
                    Some(&request.handle),
                    ObservedLifecycleState::Quarantined,
                    AdoptionState::Ambiguous,
                    ObservationReason::MultipleCandidates,
                    ProviderHealthState::Failed,
                    ProviderHealthReason::AdoptionAmbiguous,
                    ProviderRemediation::OperatorInteraction,
                ),
                AudioAdoptionOutcome::Rejected(_) => {
                    return Err(self.invalid_request(&request.context));
                }
            };
            result.map_err(|_| self.invalid_request(&request.context))
        })
    }

    fn close<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move {
            let call = self.validate_request(context, request, ProviderMethod::AudioClose)?;
            if !matches!(request.target, ProviderTarget::Handle { .. }) {
                return Err(self.invalid_request(&request.context));
            }
            let deadline = call.monotonic_deadline_remaining_ms();
            let state = self
                .invoke(
                    &request.context,
                    deadline,
                    self.effects.destroy(call, request.target.clone()),
                )
                .await?;
            self.values(&request.context)?
                .receipt(&request.context, state)
                .map_err(|_| self.invalid_request(&request.context))
        })
    }
}

fn target_matches_handle(target: &ProviderTarget, handle: &ProviderHandle) -> bool {
    match target {
        ProviderTarget::Realm { .. } => false,
        ProviderTarget::Workload {
            realm_id,
            workload_id,
        } => &handle.realm_id == realm_id && handle.workload_id.as_ref() == Some(workload_id),
        ProviderTarget::Handle {
            realm_id,
            workload_id,
            handle_id,
            handle_generation,
        } => {
            &handle.realm_id == realm_id
                && &handle.workload_id == workload_id
                && &handle.handle_id == handle_id
                && &handle.resource_generation == handle_generation
        }
    }
}

#[cfg(test)]
mod tests;
