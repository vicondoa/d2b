//! Broker-owned source-to-target host-generation handoff.
//!
//! The journal is intentionally keyed by opaque contract identities and
//! contains no host paths.  The broker is the only writer; replaying an
//! existing entry returns the same terminal result instead of repeating a
//! target effect.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use d2b_contracts_broker::broker_wire::ApplyHostGenerationHandoffResponse;
use d2b_contracts_broker::host_generation::{
    ApplyHostGenerationHandoff, HandoffCoordinator, HandoffError, HandoffState, target_fingerprint,
};
use d2b_contracts_resource::v3::ArtifactId;
use d2b_host::host_generation::{
    ActivationArtifactValidationRequest, ActivationArtifactValidationResponse,
    ActivationHelperOutcome, ActivationHelperRequest, ActivationHelperResponse,
};
use sha2::{Digest, Sha256};

const JOURNAL_DIR: &str = "host-generation-handoffs";
/// Per-state-dir flock file serializing handoff apply/replay across
/// threads and processes (plan KD1: no surviving `std::sync::Mutex`).
const HANDOFF_LOCK_FILE: &str = ".host-generation-handoff.lock";

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
    ArtifactValidationUnavailable,
    ArtifactValidationOutputInvalid,
}

impl core::fmt::Display for HandoffOperationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(formatter),
            Self::Io(_) => formatter.write_str("handoff-journal-io"),
            Self::JournalMismatch => formatter.write_str("handoff-journal-mismatch"),
            Self::HelperUnavailable => formatter.write_str("handoff-helper-unavailable"),
            Self::HelperOutputInvalid => formatter.write_str("handoff-helper-output-invalid"),
            Self::ArtifactValidationUnavailable => {
                formatter.write_str("handoff-artifact-validation-unavailable")
            }
            Self::ArtifactValidationOutputInvalid => {
                formatter.write_str("handoff-artifact-validation-output-invalid")
            }
        }
    }
}

impl std::error::Error for HandoffOperationError {}

/// Typed target-local effect used by the broker-owned journal.
///
/// `async fn` in trait is kept (generic-bound only, never dyn); the
/// `-D warnings` build promotes the `async_fn_in_trait` lint, so the
/// trait carries a targeted allow. A future sweep may desugar to the
/// `Pin<Box<dyn Future + Send>>` executor shape.
#[allow(async_fn_in_trait)]
pub trait HandoffEffect {
    /// Execute or adopt the authenticated target generation.
    async fn execute(
        &self,
        request: &ApplyHostGenerationHandoff,
    ) -> Result<ActivationHelperOutcome, HandoffOperationError>;
}

/// Deterministic effect for contract tests. Production dispatch uses
/// [`ActivationHelperEffect`] instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct SuccessfulHandoffEffect;

impl HandoffEffect for SuccessfulHandoffEffect {
    async fn execute(
        &self,
        request: &ApplyHostGenerationHandoff,
    ) -> Result<ActivationHelperOutcome, HandoffOperationError> {
        Ok(
            if request.intent.activation_mode == d2b_contracts_resource::v3::ActivationMode::Adopt {
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
    async fn execute(
        &self,
        request: &ApplyHostGenerationHandoff,
    ) -> Result<ActivationHelperOutcome, HandoffOperationError> {
        if request.target.resource_type().as_str() != "Host" {
            return Ok(ActivationHelperOutcome::Refused);
        }
        let helper_request = ActivationHelperRequest {
            system_artifact_id: request.intent.system_artifact_id.as_str().to_owned(),
            target_generation: request.intent.target_generation,
            activation_mode: match request.intent.activation_mode {
                d2b_contracts_resource::v3::ActivationMode::Switch => {
                    d2b_contracts_resource::v3::ActivationMode::Switch
                }
                d2b_contracts_resource::v3::ActivationMode::Boot => {
                    d2b_contracts_resource::v3::ActivationMode::Boot
                }
                d2b_contracts_resource::v3::ActivationMode::Test => {
                    d2b_contracts_resource::v3::ActivationMode::Test
                }
                d2b_contracts_resource::v3::ActivationMode::Adopt => {
                    d2b_contracts_resource::v3::ActivationMode::Adopt
                }
            },
        };
        let input = serde_json::to_vec(&helper_request)
            .map_err(|_| HandoffOperationError::HelperOutputInvalid)?;
        let mut child = tokio::process::Command::new(&self.helper_path)
            .arg("apply-generation")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| HandoffOperationError::HelperUnavailable)?;
        {
            use tokio::io::AsyncWriteExt;
            let mut stdin = child
                .stdin
                .take()
                .ok_or(HandoffOperationError::HelperUnavailable)?;
            stdin
                .write_all(&input)
                .await
                .map_err(|_| HandoffOperationError::HelperUnavailable)?;
        }
        let output = child
            .wait_with_output()
            .await
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

/// Resolve and verify one private system artifact without activating it.
///
/// This is used before a host-generation handoff is activated. The helper
/// remains the sole authority for catalog, store-path, and package-digest
/// validation.
pub async fn validate_artifact_with_helper(
    helper_path: &Path,
    artifact_id: &ArtifactId,
) -> Result<bool, HandoffOperationError> {
    let input = serde_json::to_vec(&ActivationArtifactValidationRequest {
        system_artifact_id: artifact_id.clone(),
    })
    .map_err(|_| HandoffOperationError::ArtifactValidationOutputInvalid)?;
    let mut child = tokio::process::Command::new(helper_path)
        .arg("validate-artifact")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| HandoffOperationError::ArtifactValidationUnavailable)?;
    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child
            .stdin
            .take()
            .ok_or(HandoffOperationError::ArtifactValidationUnavailable)?;
        stdin
            .write_all(&input)
            .await
            .map_err(|_| HandoffOperationError::ArtifactValidationUnavailable)?;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|_| HandoffOperationError::ArtifactValidationUnavailable)?;
    if output.stdout.len() > 512 {
        return Err(HandoffOperationError::ArtifactValidationOutputInvalid);
    }
    let response: ActivationArtifactValidationResponse = serde_json::from_slice(&output.stdout)
        .map_err(|_| HandoffOperationError::ArtifactValidationOutputInvalid)?;
    Ok(output.status.success() && response.valid)
}

/// Acquire the per-state-dir handoff serialization lock.
///
/// `flock(2)` on a dedicated lock file: the held fd is the lock (plan
/// KD1 - no surviving `std::sync::Mutex`), and the same file excludes
/// concurrent writers across processes as well as threads. The fd stays
/// owned by the caller for the whole apply/replay critical section.
async fn acquire_handoff_lock(
    state_dir: &Path,
) -> Result<nix::fcntl::Flock<std::fs::File>, HandoffOperationError> {
    tokio::fs::create_dir_all(state_dir)
        .await
        .map_err(HandoffOperationError::Io)?;
    let path = state_dir.join(HANDOFF_LOCK_FILE);
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .mode(0o600)
        .write(true)
        .open(&path)
        .await
        .map_err(HandoffOperationError::Io)?;
    nix::fcntl::Flock::lock(file.into_std().await, nix::fcntl::FlockArg::LockExclusive)
        .map_err(|(_, err)| HandoffOperationError::Io(io::Error::from(err)))
}

/// Apply or replay one broker-owned generation handoff using a typed effect.
pub async fn apply_with_effect<E: HandoffEffect>(
    state_dir: &Path,
    request: &ApplyHostGenerationHandoff,
    effect: &E,
) -> Result<ApplyHostGenerationHandoffResponse, HandoffOperationError> {
    let _lock = acquire_handoff_lock(state_dir).await?;
    apply_locked(state_dir, request, effect).await
}

/// Apply one production handoff through the target-local helper.
pub async fn apply_with_helper(
    state_dir: &Path,
    helper_path: &Path,
    request: &ApplyHostGenerationHandoff,
) -> Result<ApplyHostGenerationHandoffResponse, HandoffOperationError> {
    apply_with_effect(
        state_dir,
        request,
        &ActivationHelperEffect::new(helper_path),
    )
    .await
}

/// Apply or replay one deterministic handoff for compatibility callers.
pub async fn apply(
    state_dir: &Path,
    request: &ApplyHostGenerationHandoff,
) -> Result<ApplyHostGenerationHandoffResponse, HandoffOperationError> {
    apply_with_effect(state_dir, request, &SuccessfulHandoffEffect).await
}

async fn apply_locked<E: HandoffEffect>(
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
    tokio::fs::create_dir_all(&journal_dir)
        .await
        .map_err(HandoffOperationError::Io)?;
    let journal_path = journal_path(&journal_dir, request);
    let mut coordinator = if tokio::fs::symlink_metadata(&journal_path)
        .await
        .is_ok()
    {
        let bytes = tokio::fs::read(&journal_path)
            .await
            .map_err(HandoffOperationError::Io)?;
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
            .begin_handoff(
                request.intent.source_generation,
                request.intent.target_generation,
            )
            .map_err(HandoffOperationError::Invalid)?;
        persist(&journal_path, request, &coordinator).await?;
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
        persist(&journal_path, request, &coordinator).await?;
    }
    if coordinator.state() == HandoffState::Validated {
        coordinator
            .begin_mutation()
            .map_err(HandoffOperationError::Invalid)?;
        persist(&journal_path, request, &coordinator).await?;
    }
    if coordinator.state() == HandoffState::Mutating {
        match effect.execute(request).await? {
            ActivationHelperOutcome::Succeeded | ActivationHelperOutcome::Adopted => {
                coordinator
                    .transfer()
                    .map_err(HandoffOperationError::Invalid)?;
                persist(&journal_path, request, &coordinator).await?;
            }
            ActivationHelperOutcome::Refused | ActivationHelperOutcome::Failed => {
                coordinator
                    .rollback()
                    .map_err(HandoffOperationError::Invalid)?;
                persist(&journal_path, request, &coordinator).await?;
                return Ok(response(request, &coordinator));
            }
        }
    }
    if coordinator.state() == HandoffState::Transferred {
        coordinator
            .complete()
            .map_err(HandoffOperationError::Invalid)?;
        persist(&journal_path, request, &coordinator).await?;
    }
    Ok(response(request, &coordinator))
}

fn response(
    request: &ApplyHostGenerationHandoff,
    coordinator: &HandoffCoordinator,
) -> ApplyHostGenerationHandoffResponse {
    ApplyHostGenerationHandoffResponse {
        target: d2b_contracts_resource::v3::ResourceRef::parse(
            &request.target.to_canonical_string(),
        )
        .expect("validated handoff target is a canonical resource reference"),
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

async fn persist(
    path: &Path,
    request: &ApplyHostGenerationHandoff,
    coordinator: &HandoffCoordinator,
) -> Result<(), HandoffOperationError> {
    use tokio::io::AsyncWriteExt;

    let entry = serde_json::to_vec(&JournalEntry {
        request: request.clone(),
        coordinator: coordinator.clone(),
    })
    .map_err(|_| HandoffOperationError::JournalMismatch)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .await
        .map_err(HandoffOperationError::Io)?;
    file.write_all(&entry).await.map_err(HandoffOperationError::Io)?;
    file.sync_all().await.map_err(HandoffOperationError::Io)?;
    drop(file);
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(HandoffOperationError::Io)?;
    sync_parent(path).await.map_err(HandoffOperationError::Io)?;
    Ok(())
}

async fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::File::open(parent).await?.sync_all().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use d2b_contracts_broker::host_generation::{
        HandoffCallerRole, HostGenerationHandoffIntent, SourceGenerationCompatibilityFloorV1,
    };
    use d2b_contracts_resource::v3::{ActivationMode, ArtifactId, ResourceRef};

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

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn handoff_is_replay_safe_and_source_retirement_is_terminal() {
        let directory = PathBuf::from("target").join(format!(
            "d2b-handoff-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&directory);
        let first = apply(&directory, &request()).await.unwrap();
        let second = apply(&directory, &request()).await.unwrap();
        assert_eq!(first.state, HandoffState::Completed);
        assert_eq!(first, second);
        assert!(!first.source_remains_usable);
        let _ = fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn target_substitution_is_refused_before_journal_mutation() {
        let directory = PathBuf::from("target")
            .join(format!("d2b-handoff-substitution-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut request = request();
        request.target = ResourceRef::parse("Host/other").unwrap();
        assert!(matches!(
            apply(&directory, &request).await,
            Err(HandoffOperationError::Invalid(
                HandoffError::TargetFingerprintMismatch
            ))
        ));
        // The flock needs the state dir itself, but the journal must not
        // have been touched: no `host-generation-handoffs` subtree exists.
        assert!(!directory.join(JOURNAL_DIR).exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn artifact_validation_fails_closed_when_helper_is_unavailable() {
        let helper = PathBuf::from("target")
            .join(format!("missing-activation-helper-{}", std::process::id()));
        let artifact = ArtifactId::parse("candidate-artifact").expect("artifact");
        assert!(matches!(
            validate_artifact_with_helper(&helper, &artifact).await,
            Err(HandoffOperationError::ArtifactValidationUnavailable)
        ));
    }
}
