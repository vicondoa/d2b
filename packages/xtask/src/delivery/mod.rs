//! Delivery tooling for the wave workflow described in
//! `docs/specs/ADR-046-validation-and-delivery.md`.
//!
//! The module is the shared skeleton every delivery subcommand hangs off:
//!
//! * [`model`] owns the digest and identifier contract (`content_id`,
//!   `candidate_id`, `snapshot_sha256`) from spec section 12.1.
//! * [`storage`] owns the external, candidate-ID-addressed evidence
//!   directory from spec sections 12.2 and 12.5. It is never under Git.
//! * [`command`] owns argument parsing, the `wave` subcommand table, and
//!   dispatch.
//! * [`snapshot`], [`evidence`], [`seal`], [`eligibility`], and
//!   [`history_proof`] carry one workflow stage each.
//!
//! Stages that are not implemented yet fail closed through
//! [`DeliveryError::unimplemented`]; no delivery subcommand ever reports
//! success without doing the work its name promises.

// Delivery contract symbols are published for the workflow stages that
// consume them; a stage that has not landed yet leaves its symbols unused.
#![allow(dead_code, unused_imports)]

use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub mod command;
pub mod eligibility;
pub mod evidence;
pub mod history_proof;
pub mod model;
pub mod seal;
pub mod snapshot;
pub mod storage;
pub mod work_item_state;

use command::CliOptions;

pub use command::{WaveCommand, WorkflowOutput, dispatch};
pub use model::{
    CandidateDigests, CandidateId, CandidateMaterial, ContentId, DependencyEdge, EvidenceResult,
    Fingerprint, GitObjectFormat, RepositoryRecord, SnapshotSha256, canonical_digest,
};
pub use storage::{CandidateDir, StateRoot};

/// Reader view of the immutable candidate snapshot shared by delivery stages.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SnapshotView {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub content_id: ContentId,
    pub candidate_id: CandidateId,
    pub snapshot_sha256: SnapshotSha256,
    pub material: CandidateMaterial,
}

impl SnapshotView {
    pub fn program(&self) -> &str {
        &self.material.program
    }

    pub fn wave(&self) -> &str {
        &self.material.wave
    }

    pub fn digests(&self) -> CandidateDigests {
        CandidateDigests {
            content_id: self.content_id.clone(),
            candidate_id: self.candidate_id.clone(),
            snapshot_sha256: self.snapshot_sha256.clone(),
        }
    }

    /// The candidate's content identity, excluding commit history.
    pub fn content_identity(&self) -> (&CandidateId, &ContentId) {
        (&self.candidate_id, &self.content_id)
    }

    /// Rejects a snapshot whose recorded digests do not re-derive from its
    /// material, so a hand-edited candidate address cannot be laundered into a
    /// delivery stage.
    pub fn validate(&self, candidate: &CandidateDir) -> Result<()> {
        ensure_artifact_kind(
            &self.artifact_kind,
            model::SNAPSHOT_ARTIFACT_KIND,
            "snapshot",
        )?;
        model::ensure_schema(self.schema_version, "snapshot")?;
        let derived = self.material.digests()?;
        if derived != self.digests() {
            return Err(DeliveryError::new(
                "snapshot digests do not match the snapshot's own material; the candidate \
                 snapshot is not self-consistent",
            ));
        }
        candidate.validate_artifact_address(&self.material.wave, &self.candidate_id, "snapshot")?;
        Ok(())
    }
}

/// Resolves the delivery state root from `--state-dir` and the `--repo`
/// checkouts delivery state must stay outside of.
pub(crate) fn prepare_state(options: &mut CliOptions) -> Result<StateRoot> {
    prepare_state_with_roots(options).map(|(state, _)| state)
}

pub(crate) fn prepare_state_with_roots(
    options: &mut CliOptions,
) -> Result<(StateRoot, BTreeMap<String, PathBuf>)> {
    let state_dir = options.optional_path("--state-dir")?;
    let roots = options.repository_roots()?;
    let checkout_paths = roots.values().cloned().collect::<Vec<_>>();
    let state = StateRoot::prepare(&checkout_paths, state_dir.as_deref())?;
    Ok((state, roots))
}

pub(crate) fn ensure_artifact_kind(found: &str, expected: &str, label: &str) -> Result<()> {
    if found != expected {
        return Err(DeliveryError::new(format!(
            "{label} artifact kind must be {expected:?}, found {found:?}"
        )));
    }
    Ok(())
}

/// Reads a bounded JSON artifact from an operator-supplied path.
pub(crate) fn read_json_file<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T> {
    let bytes = fs::read(path)
        .map_err(|error| DeliveryError::environment(format!("cannot read {label}: {error}")))?;
    if bytes.len() > storage::MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "{label} exceeds {} bytes",
            storage::MAX_JSON_BYTES
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid {label}: {error}")))
}

/// Opens and validates the candidate named by a snapshot artifact reference.
pub fn open_candidate(
    state: &StateRoot,
    snapshot_path: &Path,
) -> Result<(CandidateDir, SnapshotView)> {
    let (candidate, snapshot): (CandidateDir, SnapshotView) = state.open_candidate_artifact(
        snapshot_path,
        storage::SNAPSHOT_FILE,
        "candidate snapshot",
    )?;
    snapshot.validate(&candidate)?;
    Ok((candidate, snapshot))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::delivery::{
        model::{self, fixtures},
        storage::{SNAPSHOT_FILE, tests::Scratch},
    };

    pub(crate) fn snapshot() -> SnapshotView {
        snapshot_from(fixtures::material())
    }

    pub(crate) fn snapshot_from(material: CandidateMaterial) -> SnapshotView {
        let digests = material.digests().expect("digests");
        SnapshotView {
            artifact_kind: model::SNAPSHOT_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            content_id: digests.content_id,
            candidate_id: digests.candidate_id,
            snapshot_sha256: digests.snapshot_sha256,
            material,
        }
    }

    pub(crate) fn candidate_with_snapshot(
        scratch: &Scratch,
    ) -> (StateRoot, CandidateDir, SnapshotView) {
        candidate_with_snapshot_from(scratch, fixtures::material())
    }

    pub(crate) fn candidate_with_snapshot_from(
        scratch: &Scratch,
        material: CandidateMaterial,
    ) -> (StateRoot, CandidateDir, SnapshotView) {
        let state = StateRoot::for_tests(&scratch.path.join("state")).expect("state root");
        let snapshot = snapshot_from(material);
        let candidate = state
            .candidate(snapshot.wave(), &snapshot.candidate_id)
            .expect("candidate");
        candidate
            .write_json(SNAPSHOT_FILE, &snapshot)
            .expect("write snapshot");
        (state, candidate, snapshot)
    }

    pub(crate) fn rebased(snapshot: &SnapshotView) -> SnapshotView {
        let mut rebased = snapshot.clone();
        rebased.material.repository_set[0].base_oid = fixtures::oid(5);
        rebased.material.repository_set[0].head_oid = fixtures::oid(6);
        rebased.material.repository_set[0].expected_pull_requests[0].head_oid = fixtures::oid(6);
        let digests = rebased.material.digests().expect("digests");
        assert_eq!(digests.candidate_id, snapshot.candidate_id);
        assert_eq!(digests.content_id, snapshot.content_id);
        assert_ne!(digests.snapshot_sha256, snapshot.snapshot_sha256);
        rebased.snapshot_sha256 = digests.snapshot_sha256;
        assert_eq!(
            rebased.material.digests().expect("rebased digests"),
            rebased.digests(),
            "rebased snapshot remains self-consistent"
        );
        rebased
    }
}

/// Schema version stamped into every delivery artifact and workflow result.
///
/// Bumped to 2 when the snapshot began binding the complete expected
/// pull-request set per repository (`expected_pull_requests`), then to 3 when
/// the removed lifecycle stages left the delivery operation domain. Each change
/// means older artifacts can no longer be read, and every downstream consumer
/// must notice the contract moved.
pub const DELIVERY_SCHEMA_VERSION: u32 = 3;

pub type Result<T> = std::result::Result<T, DeliveryError>;

/// Failure classes the delivery CLI maps onto distinct exit codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryErrorKind {
    /// The invocation itself was malformed: unknown subcommand, missing or
    /// repeated option, unparsable value.
    Usage,
    /// The subcommand exists in the contract but its implementation has not
    /// landed yet. Always fails closed.
    Unimplemented,
    /// Input was well-formed but violated a delivery invariant.
    Invalid,
    /// The environment could not satisfy the request.
    Environment,
}

impl DeliveryErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Unimplemented => "unimplemented",
            Self::Invalid => "invalid",
            Self::Environment => "environment",
        }
    }

    /// Process exit code for this failure class.
    ///
    /// Each class maps to a distinct code drawn from the BSD `sysexits.h`
    /// range so a caller can branch on the reason without parsing stderr:
    ///
    /// | Class           | Code | `sysexits.h` name |
    /// | --------------- | ---- | ----------------- |
    /// | `Usage`         | `64` | `EX_USAGE`        |
    /// | `Invalid`       | `65` | `EX_DATAERR`      |
    /// | `Unimplemented` | `69` | `EX_UNAVAILABLE`  |
    /// | `Environment`   | `72` | `EX_OSFILE`       |
    ///
    /// The four codes are distinct and all nonzero, so success is never
    /// confused with a failure and no two classes share a code.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Usage => 64,
            Self::Invalid => 65,
            Self::Unimplemented => 69,
            Self::Environment => 72,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryError {
    kind: DeliveryErrorKind,
    message: String,
}

impl DeliveryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::of(DeliveryErrorKind::Invalid, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::of(DeliveryErrorKind::Usage, message)
    }

    pub fn environment(message: impl Into<String>) -> Self {
        Self::of(DeliveryErrorKind::Environment, message)
    }

    /// Fail-closed marker for a contract subcommand whose implementation has
    /// not landed yet.
    pub fn unimplemented(command: &str, work_item: &str) -> Self {
        Self::of(
            DeliveryErrorKind::Unimplemented,
            format!(
                "delivery wave {command} is not yet implemented \
                 (work item {work_item}); it fails closed rather than \
                 reporting an unearned success"
            ),
        )
    }

    pub fn of(kind: DeliveryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> DeliveryErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for DeliveryError {}

impl From<std::io::Error> for DeliveryError {
    fn from(error: std::io::Error) -> Self {
        Self::of(
            DeliveryErrorKind::Environment,
            format!("I/O error: {error}"),
        )
    }
}

impl From<serde_json::Error> for DeliveryError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(format!("JSON error: {error}"))
    }
}

/// Entry point for `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery <args...>`.
///
/// Renders the workflow result as one JSON object on stdout, or a diagnostic
/// on stderr with the failure class' nonzero exit code.
pub fn run_cli(args: &[String]) -> std::process::ExitCode {
    match dispatch(args) {
        Ok(output) => match serde_json::to_string(&output) {
            Ok(json) => {
                println!("{json}");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("delivery failed: cannot render result: {error}");
                std::process::ExitCode::from(DeliveryErrorKind::Invalid.exit_code())
            }
        },
        Err(error) => {
            eprintln!("delivery failed [{}]: {error}", error.kind().as_str());
            std::process::ExitCode::from(error.kind().exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_class_exits_nonzero() {
        for kind in [
            DeliveryErrorKind::Usage,
            DeliveryErrorKind::Unimplemented,
            DeliveryErrorKind::Invalid,
            DeliveryErrorKind::Environment,
        ] {
            assert_ne!(kind.exit_code(), 0, "{kind:?} must not exit zero");
        }
    }

    #[test]
    fn each_failure_class_maps_to_a_distinct_sysexits_code() {
        // The exact public contract: a caller branches on these codes, so both
        // the individual values and their mutual distinctness are load-bearing.
        assert_eq!(DeliveryErrorKind::Usage.exit_code(), 64);
        assert_eq!(DeliveryErrorKind::Invalid.exit_code(), 65);
        assert_eq!(DeliveryErrorKind::Unimplemented.exit_code(), 69);
        assert_eq!(DeliveryErrorKind::Environment.exit_code(), 72);

        let codes = [
            DeliveryErrorKind::Usage,
            DeliveryErrorKind::Unimplemented,
            DeliveryErrorKind::Invalid,
            DeliveryErrorKind::Environment,
        ]
        .map(DeliveryErrorKind::exit_code);
        let unique = codes
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), codes.len(), "exit codes must be distinct");
        for code in codes {
            assert!(
                (64..=78).contains(&code),
                "{code} is outside the sysexits range"
            );
        }
    }

    #[test]
    fn unimplemented_error_names_its_command_and_work_item() {
        let error = DeliveryError::unimplemented("seal", "ADR046-delivery-006");
        assert_eq!(error.kind(), DeliveryErrorKind::Unimplemented);
        assert!(error.message().contains("seal"));
        assert!(error.message().contains("ADR046-delivery-006"));
    }
}
