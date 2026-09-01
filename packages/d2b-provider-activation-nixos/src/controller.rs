//! Pure activation-nixos reconciliation policy.

use std::collections::BTreeSet;

use d2b_contracts_resource::v3::{
    ActivationMode, ActivationOutcomeCode, ActivationRunnerInput, ArtifactId, EnvironmentClass,
    ExecutionDomain, NixosGenerationSpec, ResourceName, ResourcePhase, ResourceRef,
    process::{EphemeralProcessSpec, ExecutionSpec, NamespaceClass, ProcessClass, SandboxSpec},
};
use sha2::{Digest, Sha256};

/// The target-local Process template used for activation effects.
pub const ACTIVATION_RUNNER_TEMPLATE: &str = "activation-nixos-runner";
/// The generic one-shot process resource type used for activation effects.
pub const ACTIVATION_RUNNER_RESOURCE_TYPE: &str = "EphemeralProcess";

/// Caller role derived from the authenticated daemon request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerRole {
    /// Lifecycle-authorized operator.
    Lifecycle,
    /// Administrator with lifecycle authority.
    Admin,
    /// Ordinary user without activation authority.
    User,
    /// Provider-internal caller; never accepted from a public request.
    Provider,
}

/// Authenticated activation caller context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationCaller {
    role: CallerRole,
    target: ResourceRef,
}

impl ActivationCaller {
    /// Bind a caller role to an authenticated execution target.
    pub const fn new(role: CallerRole, target: ResourceRef) -> Self {
        Self { role, target }
    }

    /// Borrow the caller target.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    fn authorize(&self, spec: &NixosGenerationSpec) -> Result<(), ActivationError> {
        if !matches!(self.role, CallerRole::Lifecycle | CallerRole::Admin) {
            return Err(ActivationError::Unauthorized);
        }
        if self.target != *spec.execution_ref() {
            return Err(ActivationError::TargetMismatch);
        }
        Ok(())
    }
}

/// Simplified observed generation phase used by the controller seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GenerationPhase {
    /// Reconciliation has not completed.
    Pending,
    /// Generation is the active known-good generation.
    Ready,
    /// One-shot test completed.
    Succeeded,
    /// Generation failed.
    Failed,
    /// Generation is degraded.
    Degraded,
    /// The resource was deleted.
    Deleted,
}

impl GenerationPhase {
    fn resource_phase(self) -> ResourcePhase {
        match self {
            Self::Pending => ResourcePhase::Pending,
            Self::Ready => ResourcePhase::Ready,
            Self::Succeeded => ResourcePhase::Succeeded,
            Self::Failed => ResourcePhase::Failed,
            Self::Degraded => ResourcePhase::Degraded,
            Self::Deleted => ResourcePhase::Deleted,
        }
    }

    fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// One observed generation row without private store information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationObservation {
    name: String,
    phase: GenerationPhase,
    ordinal: u64,
}

impl GenerationObservation {
    /// Construct a bounded observation.
    pub fn new(name: impl Into<String>, phase: GenerationPhase) -> Self {
        let name = name.into();
        let ordinal = name
            .rsplit('-')
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        Self::terminal(name, phase, ordinal)
    }

    /// Construct a bounded terminal observation.
    pub fn terminal(name: impl Into<String>, phase: GenerationPhase, ordinal: u64) -> Self {
        let name = name.into();
        assert!(!name.is_empty() && !name.contains('/') && name.len() <= 128);
        Self {
            name,
            phase,
            ordinal,
        }
    }

    /// Borrow the bounded row name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the observed phase.
    pub const fn phase(&self) -> GenerationPhase {
        self.phase
    }

    /// Return the monotonic generation ordinal.
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }
}

/// A typed runner launch request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerRequest {
    /// Deterministic child resource name, derived from the generation.
    pub runner_name: ResourceName,
    /// Target execution context.
    pub execution_ref: ResourceRef,
    /// Private-catalog artifact identifier.
    pub system_artifact_id: ArtifactId,
    /// Requested activation mode.
    pub activation_mode: ActivationMode,
    /// Target generation ordinal bound to the runner stdin envelope.
    pub target_generation: u64,
    /// Activation runners start without an in-namespace root UID.
    pub start_root: bool,
}

/// Return the deterministic target-local child name for one generation.
///
/// The name is derived from the qualified generation reference rather than
/// an operator-provided value, so retries and daemon restarts converge on the
/// same EphemeralProcess.
pub fn activation_runner_name(generation: &ResourceRef) -> ResourceName {
    let readable = format!("activation-nixos--runner--{}", generation.name().as_str());
    if let Ok(name) = ResourceName::parse(readable) {
        return name;
    }
    let mut digest = Sha256::new();
    digest.update(b"d2b-activation-runner-v1");
    digest.update([0]);
    digest.update(generation.to_canonical_string().as_bytes());
    let digest = digest.finalize();
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ResourceName::parse(format!("activation-runner-{suffix}"))
        .expect("activation runner name is bounded and lowercase")
}

/// Return the deterministic child reference owned by one generation.
pub fn activation_runner_ref(generation: &ResourceRef) -> ResourceRef {
    let canonical = format!(
        "{ACTIVATION_RUNNER_RESOURCE_TYPE}/{}",
        activation_runner_name(generation).as_str()
    );
    ResourceRef::parse(&canonical).expect("activation runner reference is valid")
}

/// Build the closed EphemeralProcess contract for one activation request.
///
pub fn activation_runner_spec(request: &RunnerRequest) -> EphemeralProcessSpec {
    let template = d2b_contracts_resource::v3::BoundedToken::parse(ACTIVATION_RUNNER_TEMPLATE)
        .expect("static activation runner template");
    let sandbox = SandboxSpec::new(
        vec![
            NamespaceClass::Pid,
            NamespaceClass::Mount,
            NamespaceClass::Ipc,
        ],
        Vec::new(),
        d2b_contracts_resource::v3::BoundedToken::parse("activation-nixos-runner")
            .expect("static activation runner seccomp class"),
        true,
        request.start_root,
        EnvironmentClass::Minimal,
        true,
        Some("0022".to_owned()),
        0,
        None,
    )
    .expect("static activation runner sandbox");
    let budget = d2b_contracts_resource::v3::BudgetSpec::new(
        Some(d2b_contracts_resource::v3::CpuBudget {
            request: Some(
                d2b_contracts_resource::v3::MilliCpu::parse("100m")
                    .expect("static activation runner cpu request"),
            ),
            limit: Some(
                d2b_contracts_resource::v3::MilliCpu::parse("2000m")
                    .expect("static activation runner cpu limit"),
            ),
        }),
        Some(d2b_contracts_resource::v3::MemoryBudget {
            request: Some(
                d2b_contracts_resource::v3::ByteQuantity::parse("32Mi")
                    .expect("static activation runner memory request"),
            ),
            limit: Some(
                d2b_contracts_resource::v3::ByteQuantity::parse("128Mi")
                    .expect("static activation runner memory limit"),
            ),
        }),
        Some(d2b_contracts_resource::v3::CountBudget { limit: Some(128) }),
        Some(d2b_contracts_resource::v3::CountBudget { limit: Some(512) }),
        None,
        None,
        None,
    )
    .expect("static activation runner budget");
    let execution = ExecutionSpec::new(
        request.execution_ref.clone(),
        Some(ExecutionDomain::System),
        None,
        ProcessClass::Worker,
        template,
        None,
        Vec::new(),
        Vec::new(),
        sandbox,
        budget,
        None,
        Vec::new(),
        Default::default(),
    )
    .expect("static activation runner execution");
    let spec = EphemeralProcessSpec::new(
        execution,
        d2b_contracts_resource::v3::DurationMs::parse("120s", 1_000, 3_600_000)
            .expect("static activation runner start deadline"),
        d2b_contracts_resource::v3::DurationMs::parse("600s", 1_000, 86_400_000)
            .expect("static activation runner runtime deadline"),
        d2b_contracts_resource::v3::DurationMs::parse("1h", 0, 7 * 86_400_000)
            .expect("static activation runner success ttl"),
        d2b_contracts_resource::v3::DurationMs::parse("24h", 0, 30 * 86_400_000)
            .expect("static activation runner failure ttl"),
        false,
    )
    .expect("static activation runner process");
    spec.with_activation_input(
        ActivationRunnerInput::new(
            request.system_artifact_id.clone(),
            request.target_generation,
            request.activation_mode,
        )
        .expect("activation runner generation is nonzero"),
    )
    .expect("activation runner accepts its typed input")
}

/// Stable controller failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationError {
    /// Caller lacks lifecycle authority.
    Unauthorized,
    /// Caller or runner target differs from the authenticated target.
    TargetMismatch,
    /// Generation resource is malformed.
    InvalidSpec,
    /// A deleted row cannot be started.
    AlreadyDeleted,
    /// Result code does not match the selected activation mode.
    OutcomeMismatch,
}

impl core::fmt::Display for ActivationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Unauthorized => "activation-unauthorized",
            Self::TargetMismatch => "activation-target-mismatch",
            Self::InvalidSpec => "activation-spec-invalid",
            Self::AlreadyDeleted => "activation-already-deleted",
            Self::OutcomeMismatch => "activation-outcome-mismatch",
        })
    }
}

impl std::error::Error for ActivationError {}

/// Result of one activation reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerResult {
    phase: ResourcePhase,
    source_generation_preserved: bool,
    audit_codes: Vec<ActivationOutcomeCode>,
    runner_requests: Vec<RunnerRequest>,
}

impl RunnerResult {
    /// Return the projected universal phase.
    pub const fn phase(&self) -> ResourcePhase {
        self.phase
    }

    /// Whether a failed effect left the source generation usable.
    pub const fn source_generation_preserved(&self) -> bool {
        self.source_generation_preserved
    }

    /// Borrow the bounded audit outcomes.
    pub fn audit_codes(&self) -> &[ActivationOutcomeCode] {
        &self.audit_codes
    }

    /// Borrow the typed runner requests.
    pub fn runner_requests(&self) -> &[RunnerRequest] {
        &self.runner_requests
    }
}

/// Retention result for terminal generation rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPlan {
    delete_names: Vec<String>,
}

impl RetentionPlan {
    /// Return rows eligible for finalizer-driven deletion.
    pub fn delete_names(&self) -> &[String] {
        &self.delete_names
    }

    /// Retention never uses a time-to-live.
    pub const fn uses_ttl(&self) -> bool {
        false
    }
}

/// Activation-nixos controller policy.
#[derive(Debug, Clone, Copy)]
pub struct ActivationController {
    retained_generations: usize,
}

impl ActivationController {
    /// Construct a controller with the bounded retention window.
    pub fn new(retained_generations: usize) -> Self {
        assert!((1..=16).contains(&retained_generations));
        Self {
            retained_generations,
        }
    }

    /// Reconcile one desired generation.
    pub fn reconcile(
        &self,
        spec: &NixosGenerationSpec,
        caller: &ActivationCaller,
        prior: &[GenerationObservation],
        observed: GenerationObservation,
    ) -> Result<RunnerResult, ActivationError> {
        caller.authorize(spec)?;
        if let Some(prior_ref) = spec.prior_generation_ref()
            && !prior
                .iter()
                .any(|generation| generation.name() == prior_ref.name().as_str())
        {
            return Err(ActivationError::InvalidSpec);
        }
        if observed.phase == GenerationPhase::Deleted {
            return Err(ActivationError::AlreadyDeleted);
        }
        let runner_requests = if matches!(
            observed.phase,
            GenerationPhase::Pending | GenerationPhase::Degraded
        ) && spec.activation_mode() != ActivationMode::Adopt
        {
            let generation_ref = format!(
                "activation-nixos.d2bus.org.NixosGeneration/{}",
                observed.name()
            );
            vec![RunnerRequest {
                runner_name: activation_runner_name(
                    &ResourceRef::parse(&generation_ref).expect("generation reference is valid"),
                ),
                execution_ref: spec.execution_ref().clone(),
                system_artifact_id: spec.system_artifact_id().clone(),
                activation_mode: spec.activation_mode(),
                target_generation: observed.ordinal(),
                start_root: true,
            }]
        } else {
            Vec::new()
        };
        Ok(RunnerResult {
            phase: observed.phase.resource_phase(),
            source_generation_preserved: true,
            audit_codes: Vec::new(),
            runner_requests,
        })
    }

    /// Apply a typed runner result while preserving the prior generation on
    /// every refusal or failure.
    pub fn apply_runner_result(
        &self,
        spec: &NixosGenerationSpec,
        outcome: ActivationOutcomeCode,
        source: GenerationObservation,
    ) -> Result<RunnerResult, ActivationError> {
        let outcome_matches_mode = match spec.activation_mode() {
            ActivationMode::Adopt => matches!(outcome, ActivationOutcomeCode::Adopted),
            _ => !matches!(outcome, ActivationOutcomeCode::Adopted),
        };
        if !outcome_matches_mode {
            return Err(ActivationError::OutcomeMismatch);
        }
        let phase = if outcome.is_success() {
            match spec.activation_mode() {
                ActivationMode::Test => ResourcePhase::Succeeded,
                _ => ResourcePhase::Ready,
            }
        } else {
            match source.phase {
                GenerationPhase::Ready => ResourcePhase::Degraded,
                _ => ResourcePhase::Failed,
            }
        };
        Ok(RunnerResult {
            phase,
            source_generation_preserved: !outcome.is_success(),
            audit_codes: vec![outcome],
            runner_requests: Vec::new(),
        })
    }

    /// Compute finalizer-driven retention deletions.
    pub fn retention_plan(&self, observations: &[GenerationObservation]) -> RetentionPlan {
        let mut ordered = observations
            .iter()
            .map(|row| (row.ordinal, row.name.clone()))
            .collect::<Vec<_>>();
        ordered.sort_by_key(|(ordinal, _)| *ordinal);
        let keep = ordered
            .iter()
            .rev()
            .take(self.retained_generations)
            .map(|(_, name)| name.as_str())
            .collect::<BTreeSet<_>>();
        RetentionPlan {
            delete_names: observations
                .iter()
                .filter(|row| row.phase.terminal() && !keep.contains(row.name.as_str()))
                .map(|row| row.name.clone())
                .collect(),
        }
    }
}
