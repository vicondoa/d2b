use std::{error::Error, fmt};

use async_trait::async_trait;
use d2b_contracts::{
    v2_identity::ProviderId,
    v2_provider::{
        AdoptionState, AuthorizedProviderScope, ConfiguredItemId, Fingerprint, Generation,
        HandleId, HandleOwner, MutationState, ObservationReason, ObservedLifecycleState, PlanId,
        ProviderHealthReason, ProviderHealthState, ProviderOperationContext, ProviderPlan,
        ProviderRemediation, ProviderTarget,
    },
};
use d2b_provider::CancellationToken;

use crate::LocalRuntimeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeControlContractError {
    InvalidHealth,
    InvalidObservation,
}

impl fmt::Display for RuntimeControlContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidHealth => "runtime control health is invalid",
            Self::InvalidObservation => "runtime control observation is invalid",
        })
    }
}

impl Error for RuntimeControlContractError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAdoptionMismatch {
    RuntimeKind,
    ProviderIdentity,
    ProviderGeneration,
    ResourceGeneration,
    Configuration,
    Scope,
    Owner,
    MissingEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeControlError {
    InvalidRequest,
    UnauthorizedScope,
    CancelledBeforeMutation,
    DeadlineExpiredBeforeMutation,
    Unavailable,
    InvariantViolation,
    CompletionAmbiguous,
    AdoptionRejected(RuntimeAdoptionMismatch),
}

impl fmt::Display for RuntimeControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "runtime control rejected the request",
            Self::UnauthorizedScope => "runtime control rejected the authorized scope",
            Self::CancelledBeforeMutation => "runtime control cancelled before mutation",
            Self::DeadlineExpiredBeforeMutation => {
                "runtime control deadline expired before mutation"
            }
            Self::Unavailable => "runtime control is unavailable",
            Self::InvariantViolation => "runtime control invariant violation",
            Self::CompletionAmbiguous => "runtime control mutation completion is ambiguous",
            Self::AdoptionRejected(_) => "runtime control rejected adoption evidence",
        })
    }
}

impl Error for RuntimeControlError {}

#[derive(Clone)]
pub struct RuntimeControlContext {
    kind: LocalRuntimeKind,
    operation: ProviderOperationContext,
    effective_deadline_remaining_ms: u32,
    cancellation: CancellationToken,
}

impl PartialEq for RuntimeControlContext {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.operation == other.operation
            && self.effective_deadline_remaining_ms == other.effective_deadline_remaining_ms
            && self.cancellation.is_cancelled() == other.cancellation.is_cancelled()
    }
}

impl Eq for RuntimeControlContext {}

impl fmt::Debug for RuntimeControlContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeControlContext")
            .field("kind", &self.kind)
            .field(
                "effective_deadline_remaining_ms",
                &self.effective_deadline_remaining_ms,
            )
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl RuntimeControlContext {
    pub const fn kind(&self) -> LocalRuntimeKind {
        self.kind
    }

    pub fn operation(&self) -> &ProviderOperationContext {
        &self.operation
    }

    pub const fn effective_deadline_remaining_ms(&self) -> u32 {
        self.effective_deadline_remaining_ms
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub(crate) fn new(
        kind: LocalRuntimeKind,
        operation: ProviderOperationContext,
        effective_deadline_remaining_ms: u32,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            kind,
            operation,
            effective_deadline_remaining_ms,
            cancellation,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeOperationControl {
    context: RuntimeControlContext,
    target: ProviderTarget,
}

impl fmt::Debug for RuntimeOperationControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeOperationControl")
            .field("context", &self.context)
            .field("target", &self.target)
            .finish()
    }
}

impl RuntimeOperationControl {
    pub fn context(&self) -> &RuntimeControlContext {
        &self.context
    }

    pub fn target(&self) -> &ProviderTarget {
        &self.target
    }

    pub(crate) fn new(context: RuntimeControlContext, target: ProviderTarget) -> Self {
        Self { context, target }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeEnsureControl {
    context: RuntimeControlContext,
    plan: ProviderPlan,
}

impl fmt::Debug for RuntimeEnsureControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeEnsureControl")
            .field("context", &self.context)
            .field("plan", &self.plan)
            .finish()
    }
}

impl RuntimeEnsureControl {
    pub fn context(&self) -> &RuntimeControlContext {
        &self.context
    }

    pub fn plan(&self) -> &ProviderPlan {
        &self.plan
    }

    pub(crate) fn new(context: RuntimeControlContext, plan: ProviderPlan) -> Self {
        Self { context, plan }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeAdoptionControl {
    context: RuntimeControlContext,
    expected: RuntimeResourceIdentity,
}

impl fmt::Debug for RuntimeAdoptionControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeAdoptionControl")
            .field("context", &self.context)
            .field("expected", &self.expected)
            .finish()
    }
}

impl RuntimeAdoptionControl {
    pub fn context(&self) -> &RuntimeControlContext {
        &self.context
    }

    pub fn expected(&self) -> &RuntimeResourceIdentity {
        &self.expected
    }

    pub(crate) fn new(context: RuntimeControlContext, expected: RuntimeResourceIdentity) -> Self {
        Self { context, expected }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeConfiguredItemControl {
    context: RuntimeControlContext,
    configured_item_id: ConfiguredItemId,
}

impl fmt::Debug for RuntimeConfiguredItemControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfiguredItemControl")
            .field("context", &self.context)
            .field("configured_item_id", &self.configured_item_id)
            .finish()
    }
}

impl RuntimeConfiguredItemControl {
    pub fn context(&self) -> &RuntimeControlContext {
        &self.context
    }

    pub fn configured_item_id(&self) -> &ConfiguredItemId {
        &self.configured_item_id
    }

    #[allow(dead_code)]
    pub(crate) fn new(
        context: RuntimeControlContext,
        configured_item_id: ConfiguredItemId,
    ) -> Self {
        Self {
            context,
            configured_item_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePlanDecision {
    plan_id: PlanId,
    expires_at_unix_ms: u64,
}

impl RuntimePlanDecision {
    pub fn new(plan_id: PlanId, expires_at_unix_ms: u64) -> Self {
        Self {
            plan_id,
            expires_at_unix_ms,
        }
    }

    pub fn plan_id(&self) -> &PlanId {
        &self.plan_id
    }

    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeResourceIdentity {
    kind: LocalRuntimeKind,
    provider_id: ProviderId,
    provider_generation: Generation,
    scope: AuthorizedProviderScope,
    handle_id: HandleId,
    owner: HandleOwner,
    resource_generation: Generation,
    configuration_fingerprint: Fingerprint,
}

impl fmt::Debug for RuntimeResourceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeResourceIdentity")
            .field("kind", &self.kind)
            .field("provider_generation", &self.provider_generation)
            .field("scope", &"<redacted>")
            .field("owner", &self.owner)
            .field("resource_generation", &self.resource_generation)
            .finish_non_exhaustive()
    }
}

impl RuntimeResourceIdentity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: LocalRuntimeKind,
        provider_id: ProviderId,
        provider_generation: Generation,
        scope: AuthorizedProviderScope,
        handle_id: HandleId,
        owner: HandleOwner,
        resource_generation: Generation,
        configuration_fingerprint: Fingerprint,
    ) -> Self {
        Self {
            kind,
            provider_id,
            provider_generation,
            scope,
            handle_id,
            owner,
            resource_generation,
            configuration_fingerprint,
        }
    }

    pub const fn kind(&self) -> LocalRuntimeKind {
        self.kind
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub const fn provider_generation(&self) -> Generation {
        self.provider_generation
    }

    pub fn scope(&self) -> &AuthorizedProviderScope {
        &self.scope
    }

    pub fn handle_id(&self) -> &HandleId {
        &self.handle_id
    }

    pub fn owner(&self) -> &HandleOwner {
        &self.owner
    }

    pub const fn resource_generation(&self) -> Generation {
        self.resource_generation
    }

    pub fn configuration_fingerprint(&self) -> &Fingerprint {
        &self.configuration_fingerprint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeHealth {
    state: ProviderHealthState,
    reason: ProviderHealthReason,
    remediation: ProviderRemediation,
}

impl RuntimeHealth {
    pub const fn healthy() -> Self {
        Self {
            state: ProviderHealthState::Healthy,
            reason: ProviderHealthReason::None,
            remediation: ProviderRemediation::None,
        }
    }

    pub fn new(
        state: ProviderHealthState,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> Result<Self, RuntimeControlContractError> {
        let valid = match state {
            ProviderHealthState::Healthy => {
                reason == ProviderHealthReason::None && remediation == ProviderRemediation::None
            }
            ProviderHealthState::Degraded => {
                reason != ProviderHealthReason::None && remediation != ProviderRemediation::None
            }
            ProviderHealthState::Unavailable => {
                matches!(
                    reason,
                    ProviderHealthReason::HealthTimeout
                        | ProviderHealthReason::HealthStale
                        | ProviderHealthReason::SessionDisconnected
                        | ProviderHealthReason::HandshakeTimeout
                ) && remediation != ProviderRemediation::None
            }
            ProviderHealthState::Failed => {
                matches!(
                    reason,
                    ProviderHealthReason::AuthenticationFailed
                        | ProviderHealthReason::IdentityMismatch
                        | ProviderHealthReason::ConfigurationMismatch
                        | ProviderHealthReason::GenerationMismatch
                        | ProviderHealthReason::CapabilityMismatch
                        | ProviderHealthReason::AdoptionAmbiguous
                ) && matches!(
                    remediation,
                    ProviderRemediation::ReEnrollPeer
                        | ProviderRemediation::RepairConfiguration
                        | ProviderRemediation::ReplaceGeneration
                        | ProviderRemediation::OperatorInteraction
                )
            }
        };
        if valid {
            Ok(Self {
                state,
                reason,
                remediation,
            })
        } else {
            Err(RuntimeControlContractError::InvalidHealth)
        }
    }

    pub const fn state(self) -> ProviderHealthState {
        self.state
    }

    pub const fn reason(self) -> ProviderHealthReason {
        self.reason
    }

    pub const fn remediation(self) -> ProviderRemediation {
        self.remediation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeObservedState {
    identity: Option<RuntimeResourceIdentity>,
    lifecycle: ObservedLifecycleState,
    reason: ObservationReason,
    health: RuntimeHealth,
}

impl RuntimeObservedState {
    pub fn new(
        identity: Option<RuntimeResourceIdentity>,
        lifecycle: ObservedLifecycleState,
        reason: ObservationReason,
        health: RuntimeHealth,
    ) -> Result<Self, RuntimeControlContractError> {
        if matches!(
            lifecycle,
            ObservedLifecycleState::Ready | ObservedLifecycleState::Running
        ) && identity.is_none()
            || reason == ObservationReason::MultipleCandidates
        {
            return Err(RuntimeControlContractError::InvalidObservation);
        }
        Ok(Self {
            identity,
            lifecycle,
            reason,
            health,
        })
    }

    pub fn identity(&self) -> Option<&RuntimeResourceIdentity> {
        self.identity.as_ref()
    }

    pub const fn lifecycle(&self) -> ObservedLifecycleState {
        self.lifecycle
    }

    pub const fn reason(&self) -> ObservationReason {
        self.reason
    }

    pub const fn health(&self) -> RuntimeHealth {
        self.health
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeAdoptionOutcome {
    Adopted(Box<RuntimeObservedState>),
    Rejected(RuntimeAdoptionMismatch),
    Ambiguous,
}

impl RuntimeAdoptionOutcome {
    pub const fn adoption_state(&self) -> AdoptionState {
        match self {
            Self::Adopted(_) => AdoptionState::Adopted,
            Self::Rejected(_) => AdoptionState::Rejected,
            Self::Ambiguous => AdoptionState::Ambiguous,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeMutationOutcome {
    state: MutationState,
}

impl RuntimeMutationOutcome {
    pub const fn new(state: MutationState) -> Self {
        Self { state }
    }

    pub const fn state(self) -> MutationState {
        self.state
    }
}

#[async_trait]
pub trait RuntimeControlPort: Send + Sync {
    async fn health(
        &self,
        context: RuntimeControlContext,
    ) -> Result<RuntimeHealth, RuntimeControlError>;

    async fn plan(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimePlanDecision, RuntimeControlError>;

    async fn ensure(
        &self,
        request: RuntimeEnsureControl,
    ) -> Result<RuntimeResourceIdentity, RuntimeControlError>;

    async fn start(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError>;

    async fn stop(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError>;

    async fn inspect(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError>;

    async fn adopt(
        &self,
        request: RuntimeAdoptionControl,
    ) -> Result<RuntimeAdoptionOutcome, RuntimeControlError>;

    async fn destroy(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeMutationOutcome, RuntimeControlError>;

    async fn execute_configured_item(
        &self,
        request: RuntimeConfiguredItemControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError>;
}
