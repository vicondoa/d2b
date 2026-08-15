//! Broker-owned source-to-target host-generation handoff.
//!
//! The journal is intentionally keyed by opaque contract identities and
//! contains no host paths.  The broker is the only writer; replaying an
//! existing entry returns the same terminal result instead of repeating a
//! target effect.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use d2b_contracts::broker_wire::ApplyHostGenerationHandoffResponse;
use d2b_contracts::host_generation::{
    ApplyHostGenerationHandoff, HandoffCoordinator, HandoffError, HandoffState, target_fingerprint,
};
use d2b_host::host_generation::{
    ActivationHelperOutcome, ActivationHelperRequest, ActivationHelperResponse,
};
use sha2::{Digest, Sha256};

const JOURNAL_DIR: &str = "host-generation-handoffs";
static HANDOFF_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct JournalEntry {
    request: ApplyHostGenerationHandoff,
    coordinator: HandoffCoordinator,
}

#[derive(Debug)]
pub enum HandoffOperationError {
    Invalid(HandoffError),
    Io(io::Error),
    JournalMismatch,
    HelperUnavailable,
    HelperOutputInvalid,
}

impl core::fmt::Display for HandoffOperationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(formatter),
            Self::Io(_) => formatter.write_str("handoff-journal-io"),
            Self::JournalMismatch => formatter.write_str("handoff-journal-mismatch"),
            Self::HelperUnavailable => formatter.write_str("handoff-helper-unavailable"),
            Self::HelperOutputInvalid => formatter.write_str("handoff-helper-output-invalid"),
        }
    }
}

impl std::error::Error for HandoffOperationError {}

/// Typed target-local effect used by the broker-owned journal.
pub trait HandoffEffect {
    /// Execute or adopt the authenticated target generation.
    fn execute(
        &self,
        request: &ApplyHostGenerationHandoff,
    ) -> Result<ActivationHelperOutcome, HandoffOperationError>;
}

/// Deterministic effect for contract tests. Production dispatch uses
/// [`ActivationHelperEffect`] instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct SuccessfulHandoffEffect;

impl HandoffEffect for SuccessfulHandoffEffect {
    fn execute(
        &self,
        request: &ApplyHostGenerationHandoff,
    ) -> Result<ActivationHelperOutcome, HandoffOperationError> {
        Ok(
            if request.intent.activation_mode == d2b_contracts::v3::ActivationMode::Adopt {
                ActivationHelperOutcome::Adopted
            } else {
                ActivationHelperOutcome::Succeeded
            },
        )
    }
}

/// Broker-owned adapter for the target-local activation helper.
#[derive(Debug, Clone)]
pub struct ActivationHelperEffect {
    helper_path: PathBuf,
}

impl ActivationHelperEffect {
    /// Bind the helper path from trusted broker configuration.
    pub fn new(helper_path: impl Into<PathBuf>) -> Self {
        Self {
            helper_path: helper_path.into(),
        }
    }
}

impl HandoffEffect for ActivationHelperEffect {
    fn execute(
        &self,
        request: &ApplyHostGenerationHandoff,
    ) -> Result<ActivationHelperOutcome, HandoffOperationError> {
        if request.target.resource_type().as_str() != "Host" {
            return Ok(ActivationHelperOutcome::Refused);
        }
        let helper_request = ActivationHelperRequest {
            system_artifact_id: request.intent.system_artifact_id.as_str().to_owned(),
            target_generation: request.intent.target_generation,
            activation_mode: request.intent.activation_mode,
        };
        let input = serde_json::to_vec(&helper_request)
            .map_err(|_| HandoffOperationError::HelperOutputInvalid)?;
        let mut child = Command::new(&self.helper_path)
            .arg("apply-generation")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| HandoffOperationError::HelperUnavailable)?;
        child
            .stdin
            .take()
            .ok_or(HandoffOperationError::HelperUnavailable)?
            .write_all(&input)
            .map_err(|_| HandoffOperationError::HelperUnavailable)?;
        let output = child
            .wait_with_output()
            .map_err(|_| HandoffOperationError::HelperUnavailable)?;
        if output.stdout.len() > 512 {
            return Err(HandoffOperationError::HelperOutputInvalid);
        }
        let response: ActivationHelperResponse = serde_json::from_slice(&output.stdout)
            .map_err(|_| HandoffOperationError::HelperOutputInvalid)?;
        if !output.status.success()
            && !matches!(
                response.outcome,
                ActivationHelperOutcome::Refused | ActivationHelperOutcome::Failed
            )
        {
            return Err(HandoffOperationError::HelperOutputInvalid);
        }
        Ok(response.outcome)
    }
}

/// Apply or replay one broker-owned generation handoff using a typed effect.
pub fn apply_with_effect<E: HandoffEffect>(
    state_dir: &Path,
    request: &ApplyHostGenerationHandoff,
    effect: &E,
) -> Result<ApplyHostGenerationHandoffResponse, HandoffOperationError> {
    let lock = HANDOFF_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| HandoffOperationError::JournalMismatch)?;
    apply_locked(state_dir, request, effect)
}

/// Apply one production handoff through the target-local helper.
pub fn apply_with_helper(
    state_dir: &Path,
    helper_path: &Path,
    request: &ApplyHostGenerationHandoff,
) -> Result<ApplyHostGenerationHandoffResponse, HandoffOperationError> {
    apply_with_effect(
        state_dir,
        request,
        &ActivationHelperEffect::new(helper_path),
    )
}

/// Apply or replay one deterministic handoff for compatibility callers.
pub fn apply(
    state_dir: &Path,
    request: &ApplyHostGenerationHandoff,
) -> Result<ApplyHostGenerationHandoffResponse, HandoffOperationError> {
    apply_with_effect(state_dir, request, &SuccessfulHandoffEffect)
}

fn apply_locked<E: HandoffEffect>(
    state_dir: &Path,
    request: &ApplyHostGenerationHandoff,
    effect: &E,
) -> Result<ApplyHostGenerationHandoffResponse, HandoffOperationError> {
    request.validate().map_err(HandoffOperationError::Invalid)?;
    let fingerprint = target_fingerprint(
        &request.target,
        &request.intent.system_artifact_id,
        request.intent.target_generation,
    );
    request
        .intent
        .compatibility
        .validate_target(request.intent.target_generation, fingerprint)
        .map_err(HandoffOperationError::Invalid)?;

    let journal_dir = state_dir.join(JOURNAL_DIR);
    fs::create_dir_all(&journal_dir).map_err(HandoffOperationError::Io)?;
    let journal_path = journal_path(&journal_dir, request);
    let mut coordinator = if journal_path.exists() {
        let bytes = fs::read(&journal_path).map_err(HandoffOperationError::Io)?;
        let entry: JournalEntry =
            serde_json::from_slice(&bytes).map_err(|_| HandoffOperationError::JournalMismatch)?;
        if entry.request != *request {
            return Err(HandoffOperationError::JournalMismatch);
        }
        entry.coordinator
    } else {
        let coordinator = request
            .intent
            .compatibility
            .clone()
            .begin_handoff(
                request.intent.source_generation,
                request.intent.target_generation,
            )
            .map_err(HandoffOperationError::Invalid)?;
        persist(&journal_path, request, &coordinator)?;
        coordinator
    };

    if matches!(coordinator.state(), HandoffState::Completed) {
        return Ok(response(request, &coordinator));
    }
    if matches!(
        coordinator.state(),
        HandoffState::Refused | HandoffState::RolledBack
    ) {
        return Ok(response(request, &coordinator));
    }

    if coordinator.state() == HandoffState::Recorded {
        coordinator
            .validate_target(request.intent.target_generation, fingerprint)
            .map_err(HandoffOperationError::Invalid)?;
        persist(&journal_path, request, &coordinator)?;
    }
    if coordinator.state() == HandoffState::Validated {
        coordinator
            .begin_mutation()
            .map_err(HandoffOperationError::Invalid)?;
        persist(&journal_path, request, &coordinator)?;
    }
    if coordinator.state() == HandoffState::Mutating {
        match effect.execute(request)? {
            ActivationHelperOutcome::Succeeded | ActivationHelperOutcome::Adopted => {
                coordinator
                    .transfer()
                    .map_err(HandoffOperationError::Invalid)?;
                persist(&journal_path, request, &coordinator)?;
            }
            ActivationHelperOutcome::Refused | ActivationHelperOutcome::Failed => {
                coordinator
                    .rollback()
                    .map_err(HandoffOperationError::Invalid)?;
                persist(&journal_path, request, &coordinator)?;
                return Ok(response(request, &coordinator));
            }
        }
    }
    if coordinator.state() == HandoffState::Transferred {
        coordinator
            .complete()
            .map_err(HandoffOperationError::Invalid)?;
        persist(&journal_path, request, &coordinator)?;
    }
    Ok(response(request, &coordinator))
}

fn response(
    request: &ApplyHostGenerationHandoff,
    coordinator: &HandoffCoordinator,
) -> ApplyHostGenerationHandoffResponse {
    ApplyHostGenerationHandoffResponse {
        target: request.target.clone(),
        state: coordinator.state(),
        source_generation: coordinator.source_generation(),
        target_generation: coordinator.target_generation(),
        source_remains_usable: coordinator.source_remains_usable(),
        summary: match coordinator.state() {
            HandoffState::Completed => "host-generation-handoff-completed",
            HandoffState::RolledBack => "host-generation-handoff-rolled-back",
            HandoffState::Refused => "host-generation-handoff-refused",
            HandoffState::Recorded => "host-generation-handoff-recorded",
            HandoffState::Validated => "host-generation-handoff-validated",
            HandoffState::Mutating => "host-generation-handoff-mutating",
            HandoffState::Transferred => "host-generation-handoff-transferred",
        }
        .to_owned(),
    }
}

fn journal_path(directory: &Path, request: &ApplyHostGenerationHandoff) -> PathBuf {
    let encoded = serde_json::to_vec(request).expect("typed handoff serializes");
    let digest = Sha256::digest(encoded);
    let name = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    directory.join(format!("{name}.json"))
}

fn persist(
    path: &Path,
    request: &ApplyHostGenerationHandoff,
    coordinator: &HandoffCoordinator,
) -> Result<(), HandoffOperationError> {
    let entry = serde_json::to_vec(&JournalEntry {
        request: request.clone(),
        coordinator: coordinator.clone(),
    })
    .map_err(|_| HandoffOperationError::JournalMismatch)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .map_err(HandoffOperationError::Io)?;
    file.write_all(&entry).map_err(HandoffOperationError::Io)?;
    file.sync_all().map_err(HandoffOperationError::Io)?;
    drop(file);
    fs::rename(&tmp, path).map_err(HandoffOperationError::Io)?;
    sync_parent(path).map_err(HandoffOperationError::Io)?;
    Ok(())
}

fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::host_generation::{
        HandoffCallerRole, HostGenerationHandoffIntent, SourceGenerationCompatibilityFloorV1,
    };
    use d2b_contracts::v3::{ActivationMode, ArtifactId, ResourceRef};

    fn request() -> ApplyHostGenerationHandoff {
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let artifact = ArtifactId::parse("host-system").unwrap();
        let generation = 8;
        let fingerprint = target_fingerprint(&target, &artifact, generation);
        ApplyHostGenerationHandoff {
            caller_role: HandoffCallerRole::Admin,
            target,
            intent: HostGenerationHandoffIntent {
                source_generation: 7,
                target_generation: generation,
                system_artifact_id: artifact,
                activation_mode: ActivationMode::Switch,
                compatibility: SourceGenerationCompatibilityFloorV1::new(7, fingerprint).unwrap(),
            },
        }
    }

    #[test]
    fn handoff_is_replay_safe_and_source_retirement_is_terminal() {
        let directory = PathBuf::from("target").join(format!(
            "d2b-handoff-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&directory);
        let first = apply(&directory, &request()).unwrap();
        let second = apply(&directory, &request()).unwrap();
        assert_eq!(first.state, HandoffState::Completed);
        assert_eq!(first, second);
        assert!(!first.source_remains_usable);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn target_substitution_is_refused_before_journal_mutation() {
        let directory = PathBuf::from("target")
            .join(format!("d2b-handoff-substitution-{}", std::process::id()));
        let mut request = request();
        request.target = ResourceRef::parse("Host/other").unwrap();
        assert!(matches!(
            apply(&directory, &request),
            Err(HandoffOperationError::Invalid(
                HandoffError::TargetFingerprintMismatch
            ))
        ));
        assert!(!directory.exists());
    }
}
