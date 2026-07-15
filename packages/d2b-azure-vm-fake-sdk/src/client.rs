use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
};

use tokio::sync::Mutex;

use crate::{
    ApplyDisposition, BootstrapBinding, CallDisposition, CallRecord, CallSnapshot,
    ConfiguredOutcome, DeploymentHandle, DeploymentMutation, DeploymentObservation,
    DeploymentState, FakeSdkError, FakeSdkErrorKind, InfrastructureHandle, InfrastructureMutation,
    InfrastructureObservation, MAX_CALL_LOG_ENTRIES, MAX_CONFIGURED_OUTCOMES, MAX_REPLAY_ENTRIES,
    MutationResult, OperationKey, PowerState, SdkCallContext, SdkOperation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
    InfrastructureCreate {
        handle: InfrastructureHandle,
        initial_power: PowerState,
    },
    InfrastructureSetPowerState {
        handle: InfrastructureHandle,
        state: PowerState,
    },
    InfrastructureAdopt {
        handle: InfrastructureHandle,
    },
    InfrastructureBootstrap {
        handle: InfrastructureHandle,
    },
    InfrastructureInspect {
        handle: InfrastructureHandle,
    },
    InfrastructureDelete {
        handle: InfrastructureHandle,
    },
    RuntimeDeploy {
        handle: DeploymentHandle,
    },
    RuntimeStart {
        handle: DeploymentHandle,
    },
    RuntimeStop {
        handle: DeploymentHandle,
    },
    RuntimeAdopt {
        handle: DeploymentHandle,
    },
    RuntimeInspect {
        handle: DeploymentHandle,
    },
    RuntimeRemoveDeployment {
        handle: DeploymentHandle,
    },
}

impl Request {
    const fn operation(self) -> SdkOperation {
        match self {
            Self::InfrastructureCreate { .. } => SdkOperation::InfrastructureCreate,
            Self::InfrastructureSetPowerState { .. } => SdkOperation::InfrastructureSetPowerState,
            Self::InfrastructureAdopt { .. } => SdkOperation::InfrastructureAdopt,
            Self::InfrastructureBootstrap { .. } => SdkOperation::InfrastructureBootstrap,
            Self::InfrastructureInspect { .. } => SdkOperation::InfrastructureInspect,
            Self::InfrastructureDelete { .. } => SdkOperation::InfrastructureDelete,
            Self::RuntimeDeploy { .. } => SdkOperation::RuntimeDeploy,
            Self::RuntimeStart { .. } => SdkOperation::RuntimeStart,
            Self::RuntimeStop { .. } => SdkOperation::RuntimeStop,
            Self::RuntimeAdopt { .. } => SdkOperation::RuntimeAdopt,
            Self::RuntimeInspect { .. } => SdkOperation::RuntimeInspect,
            Self::RuntimeRemoveDeployment { .. } => SdkOperation::RuntimeRemoveDeployment,
        }
    }

    const fn infrastructure(self) -> InfrastructureHandle {
        match self {
            Self::InfrastructureCreate { handle, .. }
            | Self::InfrastructureSetPowerState { handle, .. }
            | Self::InfrastructureAdopt { handle }
            | Self::InfrastructureBootstrap { handle }
            | Self::InfrastructureInspect { handle }
            | Self::InfrastructureDelete { handle } => handle,
            Self::RuntimeDeploy { handle }
            | Self::RuntimeStart { handle }
            | Self::RuntimeStop { handle }
            | Self::RuntimeAdopt { handle }
            | Self::RuntimeInspect { handle }
            | Self::RuntimeRemoveDeployment { handle } => handle.infrastructure(),
        }
    }

    const fn deployment(self) -> Option<DeploymentHandle> {
        match self {
            Self::InfrastructureCreate { .. }
            | Self::InfrastructureSetPowerState { .. }
            | Self::InfrastructureAdopt { .. }
            | Self::InfrastructureBootstrap { .. }
            | Self::InfrastructureInspect { .. }
            | Self::InfrastructureDelete { .. } => None,
            Self::RuntimeDeploy { handle }
            | Self::RuntimeStart { handle }
            | Self::RuntimeStop { handle }
            | Self::RuntimeAdopt { handle }
            | Self::RuntimeInspect { handle }
            | Self::RuntimeRemoveDeployment { handle } => Some(handle),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Response {
    InfrastructureMutation(InfrastructureMutation),
    InfrastructureObservation(InfrastructureObservation),
    Bootstrap(BootstrapBinding),
    DeploymentMutation(DeploymentMutation),
    DeploymentObservation(DeploymentObservation),
    Mutation(MutationResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplayEntry {
    request: Request,
    result: Result<Response, FakeSdkError>,
}

struct State {
    configured: [VecDeque<ConfiguredOutcome>; SdkOperation::COUNT],
    counters: [u64; SdkOperation::COUNT],
    total_calls: u64,
    next_sequence: u64,
    log: VecDeque<CallRecord>,
    replay: BTreeMap<(SdkOperation, OperationKey), ReplayEntry>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            configured: std::array::from_fn(|_| VecDeque::new()),
            counters: [0; SdkOperation::COUNT],
            total_calls: 0,
            next_sequence: 1,
            log: VecDeque::new(),
            replay: BTreeMap::new(),
        }
    }
}

impl State {
    fn record(
        &mut self,
        operation: SdkOperation,
        result: &Result<Response, FakeSdkError>,
        replayed: bool,
    ) {
        self.total_calls = self.total_calls.saturating_add(1);
        self.counters[operation.index()] = self.counters[operation.index()].saturating_add(1);
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        if self.log.len() == MAX_CALL_LOG_ENTRIES {
            self.log.pop_front();
        }
        self.log.push_back(CallRecord::new(
            sequence,
            operation,
            disposition(result),
            replayed,
        ));
    }
}

/// A deterministic, bounded, in-process fake. It never acquires credentials or
/// creates an Azure/network client.
pub struct FakeAzureVmSdk {
    state: Mutex<State>,
}

impl Default for FakeAzureVmSdk {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }
}

impl fmt::Debug for FakeAzureVmSdk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeAzureVmSdk")
            .field("backend", &"in-process")
            .finish_non_exhaustive()
    }
}

impl FakeAzureVmSdk {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn configure_outcomes(
        &self,
        operation: SdkOperation,
        outcomes: impl IntoIterator<Item = ConfiguredOutcome>,
    ) -> Result<(), FakeSdkError> {
        let outcomes: Vec<_> = outcomes
            .into_iter()
            .take(MAX_CONFIGURED_OUTCOMES + 1)
            .collect();
        if outcomes.len() > MAX_CONFIGURED_OUTCOMES {
            return Err(FakeSdkError::new(
                operation,
                FakeSdkErrorKind::BoundExceeded,
            ));
        }
        if outcomes.is_empty()
            || outcomes.iter().any(|outcome| {
                *outcome == ConfiguredOutcome::AlreadyApplied
                    && !operation.permits_already_applied()
            })
        {
            return Err(FakeSdkError::new(
                operation,
                FakeSdkErrorKind::OutcomeMismatch,
            ));
        }
        let mut state = self.state.lock().await;
        state.configured[operation.index()] = outcomes.into();
        Ok(())
    }

    pub async fn snapshot(&self) -> CallSnapshot {
        let state = self.state.lock().await;
        CallSnapshot::new(
            state.total_calls,
            state.counters,
            state.log.iter().copied().collect(),
        )
    }

    pub async fn create_infrastructure(
        &self,
        context: &SdkCallContext,
        handle: InfrastructureHandle,
        initial_power: PowerState,
    ) -> Result<InfrastructureMutation, FakeSdkError> {
        match self
            .invoke(
                *context,
                Request::InfrastructureCreate {
                    handle,
                    initial_power,
                },
            )
            .await?
        {
            Response::InfrastructureMutation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::InfrastructureCreate,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn set_power_state(
        &self,
        context: &SdkCallContext,
        handle: InfrastructureHandle,
        state: PowerState,
    ) -> Result<InfrastructureObservation, FakeSdkError> {
        match self
            .invoke(
                *context,
                Request::InfrastructureSetPowerState { handle, state },
            )
            .await?
        {
            Response::InfrastructureObservation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::InfrastructureSetPowerState,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn adopt_infrastructure(
        &self,
        context: &SdkCallContext,
        handle: InfrastructureHandle,
    ) -> Result<InfrastructureObservation, FakeSdkError> {
        match self
            .invoke(*context, Request::InfrastructureAdopt { handle })
            .await?
        {
            Response::InfrastructureObservation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::InfrastructureAdopt,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn bootstrap_binding(
        &self,
        context: &SdkCallContext,
        handle: InfrastructureHandle,
    ) -> Result<BootstrapBinding, FakeSdkError> {
        match self
            .invoke(*context, Request::InfrastructureBootstrap { handle })
            .await?
        {
            Response::Bootstrap(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::InfrastructureBootstrap,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn inspect_infrastructure(
        &self,
        context: &SdkCallContext,
        handle: InfrastructureHandle,
    ) -> Result<InfrastructureObservation, FakeSdkError> {
        match self
            .invoke(*context, Request::InfrastructureInspect { handle })
            .await?
        {
            Response::InfrastructureObservation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::InfrastructureInspect,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn delete_infrastructure(
        &self,
        context: &SdkCallContext,
        handle: InfrastructureHandle,
    ) -> Result<MutationResult, FakeSdkError> {
        match self
            .invoke(*context, Request::InfrastructureDelete { handle })
            .await?
        {
            Response::Mutation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::InfrastructureDelete,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn deploy_runtime(
        &self,
        context: &SdkCallContext,
        handle: DeploymentHandle,
    ) -> Result<DeploymentMutation, FakeSdkError> {
        match self
            .invoke(*context, Request::RuntimeDeploy { handle })
            .await?
        {
            Response::DeploymentMutation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::RuntimeDeploy,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn start_runtime(
        &self,
        context: &SdkCallContext,
        handle: DeploymentHandle,
    ) -> Result<DeploymentObservation, FakeSdkError> {
        match self
            .invoke(*context, Request::RuntimeStart { handle })
            .await?
        {
            Response::DeploymentObservation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::RuntimeStart,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn stop_runtime(
        &self,
        context: &SdkCallContext,
        handle: DeploymentHandle,
    ) -> Result<DeploymentObservation, FakeSdkError> {
        match self
            .invoke(*context, Request::RuntimeStop { handle })
            .await?
        {
            Response::DeploymentObservation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::RuntimeStop,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn adopt_runtime(
        &self,
        context: &SdkCallContext,
        handle: DeploymentHandle,
    ) -> Result<DeploymentObservation, FakeSdkError> {
        match self
            .invoke(*context, Request::RuntimeAdopt { handle })
            .await?
        {
            Response::DeploymentObservation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::RuntimeAdopt,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn inspect_runtime(
        &self,
        context: &SdkCallContext,
        handle: DeploymentHandle,
    ) -> Result<DeploymentObservation, FakeSdkError> {
        match self
            .invoke(*context, Request::RuntimeInspect { handle })
            .await?
        {
            Response::DeploymentObservation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::RuntimeInspect,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    pub async fn remove_runtime_deployment(
        &self,
        context: &SdkCallContext,
        handle: DeploymentHandle,
    ) -> Result<MutationResult, FakeSdkError> {
        match self
            .invoke(*context, Request::RuntimeRemoveDeployment { handle })
            .await?
        {
            Response::Mutation(value) => Ok(value),
            _ => Err(FakeSdkError::new(
                SdkOperation::RuntimeRemoveDeployment,
                FakeSdkErrorKind::StateUnavailable,
            )),
        }
    }

    async fn invoke(
        &self,
        context: SdkCallContext,
        request: Request,
    ) -> Result<Response, FakeSdkError> {
        let operation = request.operation();
        context.validate(operation, request.infrastructure(), request.deployment())?;

        let mut state = self.state.lock().await;
        let replay_key = (operation, context.operation_key());
        if let Some(entry) = state.replay.get(&replay_key).copied() {
            let result = if entry.request == request {
                entry.result
            } else {
                Err(FakeSdkError::new(
                    operation,
                    FakeSdkErrorKind::IdempotencyConflict,
                ))
            };
            state.record(operation, &result, true);
            return result;
        }

        if state.replay.len() >= MAX_REPLAY_ENTRIES {
            let result = Err(FakeSdkError::new(
                operation,
                FakeSdkErrorKind::BoundExceeded,
            ));
            state.record(operation, &result, false);
            return result;
        }

        let outcome = state.configured[operation.index()]
            .pop_front()
            .unwrap_or(ConfiguredOutcome::Applied);
        let result = resolve(request, outcome);
        state
            .replay
            .insert(replay_key, ReplayEntry { request, result });
        state.record(operation, &result, false);
        result
    }
}

fn resolve(request: Request, outcome: ConfiguredOutcome) -> Result<Response, FakeSdkError> {
    let operation = request.operation();
    let disposition = match outcome {
        ConfiguredOutcome::Applied => ApplyDisposition::Applied,
        ConfiguredOutcome::AlreadyApplied if operation.permits_already_applied() => {
            ApplyDisposition::AlreadyApplied
        }
        ConfiguredOutcome::AlreadyApplied => {
            return Err(FakeSdkError::new(
                operation,
                FakeSdkErrorKind::OutcomeMismatch,
            ));
        }
        ConfiguredOutcome::Missing => {
            return Err(FakeSdkError::new(operation, FakeSdkErrorKind::NotFound));
        }
        ConfiguredOutcome::IdentityMismatch => {
            return Err(FakeSdkError::new(
                operation,
                FakeSdkErrorKind::IdentityMismatch,
            ));
        }
        ConfiguredOutcome::GenerationMismatch => {
            return Err(FakeSdkError::new(
                operation,
                FakeSdkErrorKind::GenerationMismatch,
            ));
        }
        ConfiguredOutcome::Unavailable => {
            return Err(FakeSdkError::new(operation, FakeSdkErrorKind::Unavailable));
        }
    };

    Ok(match request {
        Request::InfrastructureCreate {
            handle,
            initial_power: _,
        } => Response::InfrastructureMutation(InfrastructureMutation::new(handle, disposition)),
        Request::InfrastructureSetPowerState { handle, state } => {
            Response::InfrastructureObservation(InfrastructureObservation::new(handle, state))
        }
        Request::InfrastructureAdopt { handle } | Request::InfrastructureInspect { handle } => {
            Response::InfrastructureObservation(InfrastructureObservation::new(
                handle,
                PowerState::Running,
            ))
        }
        Request::InfrastructureBootstrap { handle } => Response::Bootstrap(BootstrapBinding::new(
            handle,
            handle.generation(),
            disposition,
        )),
        Request::InfrastructureDelete { .. } | Request::RuntimeRemoveDeployment { .. } => {
            Response::Mutation(MutationResult::new(disposition))
        }
        Request::RuntimeDeploy { handle } => {
            Response::DeploymentMutation(DeploymentMutation::new(handle, disposition))
        }
        Request::RuntimeStart { handle } => Response::DeploymentObservation(
            DeploymentObservation::new(handle, DeploymentState::Running),
        ),
        Request::RuntimeStop { handle } => Response::DeploymentObservation(
            DeploymentObservation::new(handle, DeploymentState::Stopped),
        ),
        Request::RuntimeAdopt { handle } | Request::RuntimeInspect { handle } => {
            Response::DeploymentObservation(DeploymentObservation::new(
                handle,
                DeploymentState::Running,
            ))
        }
    })
}

fn disposition(result: &Result<Response, FakeSdkError>) -> CallDisposition {
    match result {
        Ok(
            Response::InfrastructureMutation(InfrastructureMutation { .. })
            | Response::Bootstrap(BootstrapBinding { .. })
            | Response::DeploymentMutation(DeploymentMutation { .. })
            | Response::Mutation(MutationResult { .. }),
        ) => match result {
            Ok(Response::InfrastructureMutation(value)) => match value.disposition() {
                ApplyDisposition::Applied => CallDisposition::Succeeded,
                ApplyDisposition::AlreadyApplied => CallDisposition::AlreadyApplied,
            },
            Ok(Response::Bootstrap(value)) => match value.disposition() {
                ApplyDisposition::Applied => CallDisposition::Succeeded,
                ApplyDisposition::AlreadyApplied => CallDisposition::AlreadyApplied,
            },
            Ok(Response::DeploymentMutation(value)) => match value.disposition() {
                ApplyDisposition::Applied => CallDisposition::Succeeded,
                ApplyDisposition::AlreadyApplied => CallDisposition::AlreadyApplied,
            },
            Ok(Response::Mutation(value)) => match value.disposition() {
                ApplyDisposition::Applied => CallDisposition::Succeeded,
                ApplyDisposition::AlreadyApplied => CallDisposition::AlreadyApplied,
            },
            _ => CallDisposition::Rejected,
        },
        Ok(Response::InfrastructureObservation(_) | Response::DeploymentObservation(_)) => {
            CallDisposition::Succeeded
        }
        Err(error) if error.kind() == FakeSdkErrorKind::NotFound => CallDisposition::NotFound,
        Err(_) => CallDisposition::Rejected,
    }
}
