use std::{error::Error, fmt};

use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_CONFIGURED_OUTCOMES: usize = 16;
pub const MAX_REPLAY_ENTRIES: usize = 128;
pub const MAX_CALL_LOG_ENTRIES: usize = 256;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

macro_rules! bounded_number {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub fn new(value: u64) -> Result<Self, FakeSdkErrorKind> {
                if (1..=MAX_SAFE_INTEGER).contains(&value) {
                    Ok(Self(value))
                } else {
                    Err(FakeSdkErrorKind::BoundExceeded)
                }
            }

            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($label, "(<opaque>)"))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_number!(OperationKey, "OperationKey");
bounded_number!(ResourceId, "ResourceId");
bounded_number!(ResourceGeneration, "ResourceGeneration");

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdkAxis {
    Infrastructure,
    Runtime,
}

impl fmt::Debug for SdkAxis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Infrastructure => "Infrastructure",
            Self::Runtime => "Runtime",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdkOperation {
    InfrastructureCreate,
    InfrastructureSetPowerState,
    InfrastructureAdopt,
    InfrastructureBootstrap,
    InfrastructureInspect,
    InfrastructureDelete,
    RuntimeDeploy,
    RuntimeStart,
    RuntimeStop,
    RuntimeAdopt,
    RuntimeInspect,
    RuntimeRemoveDeployment,
}

impl SdkOperation {
    pub const ALL: [Self; 12] = [
        Self::InfrastructureCreate,
        Self::InfrastructureSetPowerState,
        Self::InfrastructureAdopt,
        Self::InfrastructureBootstrap,
        Self::InfrastructureInspect,
        Self::InfrastructureDelete,
        Self::RuntimeDeploy,
        Self::RuntimeStart,
        Self::RuntimeStop,
        Self::RuntimeAdopt,
        Self::RuntimeInspect,
        Self::RuntimeRemoveDeployment,
    ];
    pub(crate) const COUNT: usize = Self::ALL.len();

    pub const fn axis(self) -> SdkAxis {
        match self {
            Self::InfrastructureCreate
            | Self::InfrastructureSetPowerState
            | Self::InfrastructureAdopt
            | Self::InfrastructureBootstrap
            | Self::InfrastructureInspect
            | Self::InfrastructureDelete => SdkAxis::Infrastructure,
            Self::RuntimeDeploy
            | Self::RuntimeStart
            | Self::RuntimeStop
            | Self::RuntimeAdopt
            | Self::RuntimeInspect
            | Self::RuntimeRemoveDeployment => SdkAxis::Runtime,
        }
    }

    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    pub(crate) const fn permits_already_applied(self) -> bool {
        !matches!(
            self,
            Self::InfrastructureAdopt
                | Self::InfrastructureInspect
                | Self::RuntimeAdopt
                | Self::RuntimeInspect
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InfrastructureHandle {
    identity: ResourceId,
    generation: ResourceGeneration,
}

impl InfrastructureHandle {
    pub const fn new(identity: ResourceId, generation: ResourceGeneration) -> Self {
        Self {
            identity,
            generation,
        }
    }

    pub const fn identity(self) -> ResourceId {
        self.identity
    }

    pub const fn generation(self) -> ResourceGeneration {
        self.generation
    }
}

impl fmt::Debug for InfrastructureHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InfrastructureHandle(<opaque>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentHandle {
    infrastructure: InfrastructureHandle,
    identity: ResourceId,
    generation: ResourceGeneration,
}

impl DeploymentHandle {
    pub const fn new(
        infrastructure: InfrastructureHandle,
        identity: ResourceId,
        generation: ResourceGeneration,
    ) -> Self {
        Self {
            infrastructure,
            identity,
            generation,
        }
    }

    pub const fn infrastructure(self) -> InfrastructureHandle {
        self.infrastructure
    }

    pub const fn identity(self) -> ResourceId {
        self.identity
    }

    pub const fn generation(self) -> ResourceGeneration {
        self.generation
    }
}

impl fmt::Debug for DeploymentHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeploymentHandle(<opaque>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerState {
    Running,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentState {
    Running,
    Stopped,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "axis", rename_all = "kebab-case", deny_unknown_fields)]
pub enum OperationScope {
    Infrastructure {
        handle: InfrastructureHandle,
    },
    Runtime {
        infrastructure: InfrastructureHandle,
        deployment: DeploymentHandle,
    },
}

impl OperationScope {
    pub const fn axis(self) -> SdkAxis {
        match self {
            Self::Infrastructure { .. } => SdkAxis::Infrastructure,
            Self::Runtime { .. } => SdkAxis::Runtime,
        }
    }

    pub(crate) fn validates(
        self,
        operation: SdkOperation,
        infrastructure: InfrastructureHandle,
        deployment: Option<DeploymentHandle>,
    ) -> bool {
        match (self, operation.axis(), deployment) {
            (Self::Infrastructure { handle }, SdkAxis::Infrastructure, None) => {
                handle == infrastructure
            }
            (
                Self::Runtime {
                    infrastructure: scoped_infrastructure,
                    deployment: scoped_deployment,
                },
                SdkAxis::Runtime,
                Some(deployment),
            ) => {
                scoped_infrastructure == infrastructure
                    && scoped_deployment == deployment
                    && deployment.infrastructure() == infrastructure
            }
            _ => false,
        }
    }
}

impl fmt::Debug for OperationScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationScope")
            .field("axis", &self.axis())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SdkCallContext {
    operation_key: OperationKey,
    scope: OperationScope,
    deadline_remaining_ms: u32,
    cancelled: bool,
}

impl SdkCallContext {
    pub const fn infrastructure(
        operation_key: OperationKey,
        handle: InfrastructureHandle,
        deadline_remaining_ms: u32,
    ) -> Self {
        Self {
            operation_key,
            scope: OperationScope::Infrastructure { handle },
            deadline_remaining_ms,
            cancelled: false,
        }
    }

    pub const fn runtime(
        operation_key: OperationKey,
        infrastructure: InfrastructureHandle,
        deployment: DeploymentHandle,
        deadline_remaining_ms: u32,
    ) -> Self {
        Self {
            operation_key,
            scope: OperationScope::Runtime {
                infrastructure,
                deployment,
            },
            deadline_remaining_ms,
            cancelled: false,
        }
    }

    pub const fn cancelled(mut self) -> Self {
        self.cancelled = true;
        self
    }

    pub const fn operation_key(self) -> OperationKey {
        self.operation_key
    }

    pub const fn scope(self) -> OperationScope {
        self.scope
    }

    pub const fn deadline_remaining_ms(self) -> u32 {
        self.deadline_remaining_ms
    }

    pub const fn is_cancelled(self) -> bool {
        self.cancelled
    }

    pub(crate) fn validate(
        self,
        operation: SdkOperation,
        infrastructure: InfrastructureHandle,
        deployment: Option<DeploymentHandle>,
    ) -> Result<(), FakeSdkError> {
        if self.cancelled {
            return Err(FakeSdkError::new(operation, FakeSdkErrorKind::Cancelled));
        }
        if self.deadline_remaining_ms == 0 {
            return Err(FakeSdkError::new(
                operation,
                FakeSdkErrorKind::DeadlineExpired,
            ));
        }
        if !self.scope.validates(operation, infrastructure, deployment) {
            return Err(FakeSdkError::new(
                operation,
                FakeSdkErrorKind::AuthorityDenied,
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for SdkCallContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SdkCallContext")
            .field("scope", &self.scope)
            .field("deadline_remaining_ms", &self.deadline_remaining_ms)
            .field("cancelled", &self.cancelled)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfiguredOutcome {
    Applied,
    AlreadyApplied,
    Missing,
    IdentityMismatch,
    GenerationMismatch,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApplyDisposition {
    Applied,
    AlreadyApplied,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InfrastructureMutation {
    handle: InfrastructureHandle,
    disposition: ApplyDisposition,
}

impl InfrastructureMutation {
    pub(crate) const fn new(handle: InfrastructureHandle, disposition: ApplyDisposition) -> Self {
        Self {
            handle,
            disposition,
        }
    }

    pub const fn handle(self) -> InfrastructureHandle {
        self.handle
    }

    pub const fn disposition(self) -> ApplyDisposition {
        self.disposition
    }
}

impl fmt::Debug for InfrastructureMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InfrastructureMutation")
            .field("disposition", &self.disposition)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InfrastructureObservation {
    handle: InfrastructureHandle,
    power_state: PowerState,
}

impl InfrastructureObservation {
    pub(crate) const fn new(handle: InfrastructureHandle, power_state: PowerState) -> Self {
        Self {
            handle,
            power_state,
        }
    }

    pub const fn handle(self) -> InfrastructureHandle {
        self.handle
    }

    pub const fn power_state(self) -> PowerState {
        self.power_state
    }
}

impl fmt::Debug for InfrastructureObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InfrastructureObservation")
            .field("power_state", &self.power_state)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapBinding {
    infrastructure: InfrastructureHandle,
    binding_generation: ResourceGeneration,
    disposition: ApplyDisposition,
}

impl BootstrapBinding {
    pub(crate) const fn new(
        infrastructure: InfrastructureHandle,
        binding_generation: ResourceGeneration,
        disposition: ApplyDisposition,
    ) -> Self {
        Self {
            infrastructure,
            binding_generation,
            disposition,
        }
    }

    pub const fn infrastructure(self) -> InfrastructureHandle {
        self.infrastructure
    }

    pub const fn binding_generation(self) -> ResourceGeneration {
        self.binding_generation
    }

    pub const fn disposition(self) -> ApplyDisposition {
        self.disposition
    }
}

impl fmt::Debug for BootstrapBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BootstrapBinding")
            .field("disposition", &self.disposition)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentMutation {
    handle: DeploymentHandle,
    disposition: ApplyDisposition,
}

impl DeploymentMutation {
    pub(crate) const fn new(handle: DeploymentHandle, disposition: ApplyDisposition) -> Self {
        Self {
            handle,
            disposition,
        }
    }

    pub const fn handle(self) -> DeploymentHandle {
        self.handle
    }

    pub const fn disposition(self) -> ApplyDisposition {
        self.disposition
    }
}

impl fmt::Debug for DeploymentMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeploymentMutation")
            .field("disposition", &self.disposition)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentObservation {
    handle: DeploymentHandle,
    state: DeploymentState,
}

impl DeploymentObservation {
    pub(crate) const fn new(handle: DeploymentHandle, state: DeploymentState) -> Self {
        Self { handle, state }
    }

    pub const fn handle(self) -> DeploymentHandle {
        self.handle
    }

    pub const fn state(self) -> DeploymentState {
        self.state
    }
}

impl fmt::Debug for DeploymentObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeploymentObservation")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MutationResult {
    disposition: ApplyDisposition,
}

impl MutationResult {
    pub(crate) const fn new(disposition: ApplyDisposition) -> Self {
        Self { disposition }
    }

    pub const fn disposition(self) -> ApplyDisposition {
        self.disposition
    }
}

impl fmt::Debug for MutationResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MutationResult")
            .field("disposition", &self.disposition)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FakeSdkErrorKind {
    AuthorityDenied,
    Cancelled,
    DeadlineExpired,
    Unavailable,
    NotFound,
    IdentityMismatch,
    GenerationMismatch,
    IdempotencyConflict,
    OutcomeMismatch,
    BoundExceeded,
    StateUnavailable,
}

impl fmt::Display for FakeSdkErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AuthorityDenied => "authority denied",
            Self::Cancelled => "operation cancelled",
            Self::DeadlineExpired => "deadline expired",
            Self::Unavailable => "fake SDK unavailable",
            Self::NotFound => "resource not found",
            Self::IdentityMismatch => "resource identity mismatch",
            Self::GenerationMismatch => "resource generation mismatch",
            Self::IdempotencyConflict => "idempotency conflict",
            Self::OutcomeMismatch => "configured outcome does not match operation",
            Self::BoundExceeded => "fake SDK bound exceeded",
            Self::StateUnavailable => "fake SDK state unavailable",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FakeSdkError {
    operation: SdkOperation,
    kind: FakeSdkErrorKind,
}

impl FakeSdkError {
    pub(crate) const fn new(operation: SdkOperation, kind: FakeSdkErrorKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> SdkOperation {
        self.operation
    }

    pub const fn kind(self) -> FakeSdkErrorKind {
        self.kind
    }
}

impl fmt::Debug for FakeSdkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeSdkError")
            .field("operation", &self.operation)
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for FakeSdkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "fake Azure VM SDK operation {:?} failed ({})",
            self.operation, self.kind
        )
    }
}

impl Error for FakeSdkError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CallDisposition {
    Succeeded,
    AlreadyApplied,
    NotFound,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallRecord {
    sequence: u64,
    operation: SdkOperation,
    disposition: CallDisposition,
    replayed: bool,
}

impl CallRecord {
    pub(crate) const fn new(
        sequence: u64,
        operation: SdkOperation,
        disposition: CallDisposition,
        replayed: bool,
    ) -> Self {
        Self {
            sequence,
            operation,
            disposition,
            replayed,
        }
    }

    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    pub const fn operation(self) -> SdkOperation {
        self.operation
    }

    pub const fn disposition(self) -> CallDisposition {
        self.disposition
    }

    pub const fn replayed(self) -> bool {
        self.replayed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSnapshot {
    total_calls: u64,
    per_operation: [u64; SdkOperation::COUNT],
    log: Vec<CallRecord>,
}

impl CallSnapshot {
    pub(crate) const fn new(
        total_calls: u64,
        per_operation: [u64; SdkOperation::COUNT],
        log: Vec<CallRecord>,
    ) -> Self {
        Self {
            total_calls,
            per_operation,
            log,
        }
    }

    pub const fn total_calls(&self) -> u64 {
        self.total_calls
    }

    pub fn calls(&self, operation: SdkOperation) -> u64 {
        self.per_operation[operation.index()]
    }

    pub fn log(&self) -> &[CallRecord] {
        &self.log
    }
}
