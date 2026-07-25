//! Validator evidence import (spec section 12.2, work item
//! `ADR046-delivery-003`).
//!
//! `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave validate-import` records that one validator lane
//! ran against one candidate and what it returned. Three lanes report into the
//! same candidate: required GitHub CI, the heavy-gated local and host
//! validators, and - through its own tooling - the ten-role panel.
//!
//! # How raw output is kept out of Git
//!
//! Spec section 12.5 is absolute: validation command output never enters Git, a
//! generated artifact, a pull-request body, or a release archive. This module
//! makes that structural rather than advisory:
//!
//! * No option accepts output text. `--log` takes a path, and the file behind
//!   it is streamed through a SHA-256 hasher; only the digest and the byte
//!   count are recorded. The bytes are never buffered whole, never copied, and
//!   never serialized.
//! * The record is written only through
//!   [`CandidateDir`](super::storage::CandidateDir), whose state root
//!   [`StateRoot::prepare`](super::storage::StateRoot::prepare) has already
//!   proven to sit outside every repository checkout and outside every Git
//!   working tree. There is no code path that writes an evidence record
//!   anywhere else.
//!
//! # How stale evidence is rejected
//!
//! A record is addressed by the snapshot's `candidate_id`, and three checks
//! stand between an invocation and a write:
//!
//! 1. the snapshot rederives its own digests, so an edited snapshot is refused;
//! 2. the repositories are reread and rederived, so evidence collected after
//!    the wave's content moved is refused rather than filed against the
//!    superseded candidate;
//! 3. the optional `--candidate` guard lets a lane state which candidate it
//!    believes it ran against, and a mismatch is refused.
//!
//! Importing a `failed` result succeeds - recording a failure is a successful
//! import. A lane with no record at all is pending, and `wave seal` treats
//! pending and failed alike: neither permits merge.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{CliOptions, WaveCommand, WorkflowOutput},
    model::{
        CandidateId, ContentId, EVIDENCE_ARTIFACT_KIND, EvidenceResult, SnapshotSha256,
        ensure_schema, validate_bounded_string, validate_identifier, validate_program_wave,
    },
    snapshot::{self, WaveSnapshot},
    storage::{CandidateDir, EVIDENCE_DIR, StateRoot},
};

/// Chunk size used to stream a validator log through the hasher.
const LOG_CHUNK_BYTES: usize = 64 * 1024;

/// Which of the concurrent lanes of spec section 12.2 produced a record.
///
/// This is the one canonical lane ABI: `wave validate-import` writes these
/// variants and `wave seal` reads them, so the two stages can never disagree
/// on the on-disk vocabulary again.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceLane {
    /// Required GitHub CI (the Layer-1 `check` rollup).
    GithubCi,
    /// The heavy-gated local and host validators run by the integrator.
    LocalHost,
    /// The ten-role panel. Its authoritative evidence is the record set;
    /// a lane record for the panel is accepted but never a substitute.
    Panel,
}

/// Every lane, in the order section 12.2 lists them.
pub const EVIDENCE_LANES: [EvidenceLane; 3] = [
    EvidenceLane::GithubCi,
    EvidenceLane::LocalHost,
    EvidenceLane::Panel,
];

/// Lanes that must each carry at least one passing imported result before a
/// seal. The panel is not here: its authoritative evidence is the ten
/// unanimous records, which the seal verifies directly.
pub const REQUIRED_EVIDENCE_LANES: [EvidenceLane; 2] =
    [EvidenceLane::GithubCi, EvidenceLane::LocalHost];

impl EvidenceLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GithubCi => "github-ci",
            Self::LocalHost => "local-host",
            Self::Panel => "panel",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        EVIDENCE_LANES
            .into_iter()
            .find(|lane| lane.as_str() == value)
            .ok_or_else(|| {
                DeliveryError::usage(format!(
                    "--lane must be one of github-ci, local-host, panel; found {value:?}"
                ))
            })
    }
}

fn parse_result(value: &str) -> Result<EvidenceResult> {
    match value {
        "passed" => Ok(EvidenceResult::Passed),
        "failed" => Ok(EvidenceResult::Failed),
        other => Err(DeliveryError::usage(format!(
            "--result must be passed or failed; found {other:?}"
        ))),
    }
}

/// Digest and size of a validator log, recorded in place of its content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutputDigest {
    pub sha256: String,
    pub bytes: u64,
}

/// One validator lane's command and result, bound to one candidate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub snapshot_sha256: SnapshotSha256,
    pub lane: EvidenceLane,
    /// Lane-unique validation identifier, for example `test-integration`.
    pub validation: String,
    pub result: EvidenceResult,
    pub imported_at_unix: u64,
    /// Command line that was run. Never its output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Digest of the validator's output. The output itself is never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputDigest>,
    /// External locator, for example a CI run URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
}

impl EvidenceRecord {
    /// Relative path this record occupies inside the candidate directory.
    pub fn relative_path(&self) -> PathBuf {
        Path::new(EVIDENCE_DIR)
            .join(self.lane.as_str())
            .join(format!("{}.json", self.validation))
    }

    pub fn validate(&self) -> Result<()> {
        if self.artifact_kind != EVIDENCE_ARTIFACT_KIND {
            return Err(DeliveryError::new(format!(
                "expected artifact kind {EVIDENCE_ARTIFACT_KIND}, found {:?}",
                self.artifact_kind
            )));
        }
        ensure_schema(self.schema_version, "validation evidence")?;
        validate_program_wave(&self.program, &self.wave)?;
        validate_identifier(&self.validation, "validation")?;
        if let Some(command) = &self.command {
            validate_single_line(command, "--command")?;
        }
        if let Some(locator) = &self.locator {
            validate_single_line(locator, "--locator")?;
        }
        Ok(())
    }

    /// Rejects evidence that is not bound to the given snapshot digests.
    ///
    /// The `snapshot_sha256` comparison is the load-bearing one. A
    /// history-only rebase preserves `candidate_id` and `content_id` by
    /// design, so those two alone would silently carry a stale lane result
    /// across a rebase; `snapshot_sha256` moves with the base and head object
    /// IDs and is what forces the re-import.
    pub fn ensure_bound(
        &self,
        candidate_id: &CandidateId,
        content_id: &ContentId,
        snapshot_sha256: &SnapshotSha256,
    ) -> Result<()> {
        if &self.candidate_id != candidate_id
            || &self.content_id != content_id
            || &self.snapshot_sha256 != snapshot_sha256
        {
            return Err(DeliveryError::new(format!(
                "validator evidence for {:?} on lane {} is bound to a stale snapshot; a rebase \
                 preserves the panel record but never a lane result, so every lane reruns and \
                 re-imports against the current snapshot",
                self.validation,
                self.lane.as_str()
            )));
        }
        Ok(())
    }

    /// Rejects a lane result that is anything other than a pass.
    pub fn ensure_passed(&self) -> Result<()> {
        if self.result != EvidenceResult::Passed {
            return Err(DeliveryError::new(format!(
                "validator evidence for {:?} on lane {} did not pass",
                self.validation,
                self.lane.as_str()
            )));
        }
        Ok(())
    }
}

/// Routes `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave validate-import`.
pub fn run(args: &[String]) -> Result<WorkflowOutput> {
    let mut request = ImportRequest::parse(args)?;
    let checkouts = request.checkout_roots()?;
    let root = StateRoot::prepare(
        &checkouts.values().cloned().collect::<Vec<_>>(),
        request.state_dir.as_deref(),
    )?;
    request.snapshot_path = root.resolve_artifact_ref(&request.snapshot_path);
    run_with_root(&request, &root)
}

fn run_with_root(request: &ImportRequest, root: &StateRoot) -> Result<WorkflowOutput> {
    let supplied = snapshot::read_file(&request.snapshot_path)?;
    if let Some(expected) = &request.candidate
        && expected != &supplied.candidate_id
    {
        return Err(DeliveryError::new(format!(
            "evidence is bound to candidate {expected}, but the snapshot is candidate {}; \
             the wave has been re-snapshotted and this lane must rerun",
            supplied.candidate_id
        )));
    }

    let candidate = root.existing_candidate(supplied.wave(), &supplied.candidate_id)?;
    let sealed = snapshot::read(&candidate)?.ok_or_else(|| {
        DeliveryError::new(format!(
            "candidate {} holds no snapshot; run wave snapshot first",
            supplied.candidate_id
        ))
    })?;
    if sealed != supplied {
        return Err(DeliveryError::new(format!(
            "the supplied snapshot is not the one sealed for candidate {}; \
             import against the candidate's current snapshot",
            supplied.candidate_id
        )));
    }

    let current = snapshot::rederive(&sealed, &request.checkout_roots()?)?;
    if current.content_id != sealed.content_id || current.candidate_id != sealed.candidate_id {
        return Err(DeliveryError::new(format!(
            "the repositories now integrate to candidate {}, not the snapshot's candidate {}; \
             evidence for superseded content is refused, so re-snapshot and rerun this lane",
            current.candidate_id, sealed.candidate_id
        )));
    }
    if current.snapshot_sha256 != sealed.snapshot_sha256 {
        return Err(DeliveryError::new(format!(
            "candidate {} kept its address but its commit history moved, so the sealed snapshot \
             digest is stale; a history-only rebase preserves the panel record but never a lane \
             result, so re-snapshot in place and rerun this lane against the current snapshot",
            sealed.candidate_id
        )));
    }

    let record = request.record(&sealed)?;
    let path = import(&candidate, &record)?;
    WorkflowOutput::ok(WaveCommand::ValidateImport)
        .with_digests(&sealed.digests())
        .with_artifact(&candidate, &path)
}

/// Writes one evidence record into the candidate directory.
pub fn import(candidate: &CandidateDir, record: &EvidenceRecord) -> Result<PathBuf> {
    record.validate()?;
    if candidate.candidate_id() != &record.candidate_id {
        return Err(DeliveryError::new(
            "candidate directory does not address this evidence record's candidate ID",
        ));
    }
    let relative = record.relative_path();
    candidate.write_json(&relative, record)?;
    candidate.resolve(&relative)
}

/// Reads every evidence record filed against one candidate from the canonical
/// nested layout `evidence/<lane>/<validation>.json`, validated and bound to
/// the given snapshot digests.
///
/// This is the single reader both `wave validate-import` (through
/// [`read_all`]) and `wave seal` consume, so the two stages read exactly what
/// the writer wrote. The layout is enforced strictly: the only entries allowed
/// directly under `evidence/` are the known lane directories, and the only
/// entries allowed inside a lane directory are `<validation>.json` records
/// whose recorded validation matches the file name. A stray file, an unknown
/// lane, or a record filed under the wrong lane or name is a rejection, which
/// keeps raw validator output structurally unable to masquerade as evidence.
pub fn read_lane_records(
    candidate: &CandidateDir,
    candidate_id: &CandidateId,
    content_id: &ContentId,
    snapshot_sha256: &SnapshotSha256,
) -> Result<Vec<EvidenceRecord>> {
    let evidence_root = Path::new(EVIDENCE_DIR);
    if !candidate.resolve(evidence_root)?.is_dir() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in candidate.list(evidence_root)? {
        let entry_name = entry
            .to_str()
            .ok_or_else(|| DeliveryError::new("evidence entry name is not UTF-8"))?
            .to_owned();
        let lane = EvidenceLane::parse(&entry_name).map_err(|_| {
            DeliveryError::new(format!(
                "evidence directory holds {entry_name:?}, which is not a known validator lane; \
                 it is not an evidence record and raw output never lives under evidence"
            ))
        })?;
        let lane_dir = evidence_root.join(lane.as_str());
        if !candidate.resolve(&lane_dir)?.is_dir() {
            return Err(DeliveryError::new(format!(
                "evidence lane entry {entry_name:?} is not a directory"
            )));
        }
        for file in candidate.list(&lane_dir)? {
            let file_name = file
                .to_str()
                .ok_or_else(|| DeliveryError::new("evidence record file name is not UTF-8"))?
                .to_owned();
            if !file_name.ends_with(".json") {
                return Err(DeliveryError::new(format!(
                    "validator lane {} holds {file_name:?}, which is not an evidence record",
                    lane.as_str()
                )));
            }
            let record: EvidenceRecord = candidate.read_json(lane_dir.join(&file_name))?;
            record.validate()?;
            if record.lane != lane {
                return Err(DeliveryError::new(format!(
                    "evidence record filed under lane {} declares lane {}",
                    lane.as_str(),
                    record.lane.as_str()
                )));
            }
            if file_name != format!("{}.json", record.validation) {
                return Err(DeliveryError::new(format!(
                    "evidence record {file_name:?} in lane {} declares validation {:?}, so its \
                     file name does not address its own content",
                    lane.as_str(),
                    record.validation
                )));
            }
            record.ensure_bound(candidate_id, content_id, snapshot_sha256)?;
            records.push(record);
        }
    }
    Ok(records)
}

/// Reads the imported validator evidence bound to the given snapshot digests
/// and enforces the seal-time lane invariants against it: at least one record,
/// every record a pass, and every required lane present.
///
/// Both `wave seal` and `merge-eligibility` call this against the *current*
/// snapshot digest, so a history-only rebase - which moves `snapshot_sha256`
/// and therefore unbinds every prior lane record through
/// [`EvidenceRecord::ensure_bound`] - is caught identically at seal time and
/// at the merge gate: the candidate is eligible only once every lane has rerun
/// and re-imported against the new snapshot. Panel evidence is unaffected
/// because it binds `candidate_id`/`content_id`, which a rebase preserves.
pub fn require_passing_lanes(
    candidate: &CandidateDir,
    candidate_id: &CandidateId,
    content_id: &ContentId,
    snapshot_sha256: &SnapshotSha256,
) -> Result<Vec<EvidenceRecord>> {
    let records = read_lane_records(candidate, candidate_id, content_id, snapshot_sha256)?;
    if records.is_empty() {
        return Err(DeliveryError::new(
            "no validator evidence bound to the current snapshot; every lane must rerun and \
             re-import against the current history before this candidate is eligible",
        ));
    }
    let mut present = BTreeSet::new();
    for record in &records {
        record.ensure_passed()?;
        present.insert(record.lane);
    }
    for lane in REQUIRED_EVIDENCE_LANES {
        if !present.contains(&lane) {
            return Err(DeliveryError::new(format!(
                "validator lane {} has no passing result bound to the current snapshot; a \
                 pending or stale lane never permits a seal or a merge",
                lane.as_str()
            )));
        }
    }
    Ok(records)
}

/// Reads every evidence record filed against one candidate, ordered by lane
/// and validation identifier.
pub fn read_all(candidate: &CandidateDir, snapshot: &WaveSnapshot) -> Result<Vec<EvidenceRecord>> {
    read_lane_records(
        candidate,
        &snapshot.candidate_id,
        &snapshot.content_id,
        &snapshot.snapshot_sha256,
    )
}

/// Parsed `wave validate-import` invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportRequest {
    snapshot_path: PathBuf,
    validation: String,
    result: EvidenceResult,
    lane: EvidenceLane,
    checkouts: BTreeMap<String, PathBuf>,
    command: Option<String>,
    log_path: Option<PathBuf>,
    locator: Option<String>,
    candidate: Option<CandidateId>,
    state_dir: Option<PathBuf>,
}

impl ImportRequest {
    fn parse(args: &[String]) -> Result<Self> {
        let mut options = CliOptions::parse(args)?;
        let snapshot_path = options.required_path("--snapshot")?;
        let validation = options.required_string("--validation")?;
        let result = parse_result(&options.required_string("--result")?)?;
        let checkouts = options.repository_roots()?;
        let lane = match options.optional_string("--lane")? {
            Some(value) => EvidenceLane::parse(&value)?,
            None => EvidenceLane::LocalHost,
        };
        if lane == EvidenceLane::Panel {
            return Err(DeliveryError::usage(
                "--lane panel is not importable: the panel lane's evidence is the ten unanimous \
                 records produced by wave panel-attest, not a validator import; import github-ci \
                 or local-host",
            ));
        }
        let command = options.optional_string("--command")?;
        let log_path = options.optional_path("--log")?;
        let locator = options.optional_string("--locator")?;
        let candidate = options
            .optional_string("--candidate")?
            .map(CandidateId::parse)
            .transpose()?;
        let state_dir = options.optional_path("--state-dir")?;
        options.finish()?;

        validate_identifier(&validation, "validation")?;
        if let Some(command) = &command {
            validate_single_line(command, "--command")?;
        }
        if let Some(locator) = &locator {
            validate_single_line(locator, "--locator")?;
        }
        Ok(Self {
            snapshot_path,
            validation,
            result,
            lane,
            checkouts,
            command,
            log_path,
            locator,
            candidate,
            state_dir,
        })
    }

    /// Git working-tree roots of every declared checkout, used both to keep
    /// delivery state outside them and to rederive the candidate.
    fn checkout_roots(&self) -> Result<BTreeMap<String, PathBuf>> {
        self.checkouts
            .iter()
            .map(|(id, root)| Ok((id.clone(), std::fs::canonicalize(root)?)))
            .collect()
    }

    fn record(&self, snapshot: &WaveSnapshot) -> Result<EvidenceRecord> {
        let record = EvidenceRecord {
            artifact_kind: EVIDENCE_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            program: snapshot.program().to_owned(),
            wave: snapshot.wave().to_owned(),
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            lane: self.lane,
            validation: self.validation.clone(),
            result: self.result,
            imported_at_unix: now_unix(),
            command: self.command.clone(),
            output: self
                .log_path
                .as_deref()
                .map(digest_without_retaining)
                .transpose()?,
            locator: self.locator.clone(),
        };
        record.validate()?;
        Ok(record)
    }
}

/// Streams a validator log through a hasher and keeps only its digest.
///
/// This is the whole of the module's contact with validator output: the bytes
/// are read a chunk at a time into a reused buffer, hashed, and dropped. No
/// caller can obtain them, so no artifact can carry them.
fn digest_without_retaining(path: &Path) -> Result<OutputDigest> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        // Name the artifact role, never the absolute log path: this diagnostic
        // reaches operator stderr and CI logs, and the log path is operator
        // supplied and routinely absolute.
        return Err(DeliveryError::new(
            "validator log is not a regular file".to_owned(),
        ));
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0_u8; LOG_CHUNK_BYTES];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
        bytes += read as u64;
    }
    Ok(OutputDigest {
        sha256: render(hasher.finalize()),
        bytes,
    })
}

fn render(digest: impl IntoIterator<Item = u8>) -> String {
    use std::fmt::Write as _;
    let mut rendered = String::with_capacity(64);
    for byte in digest {
        write!(&mut rendered, "{byte:02x}").expect("writing to a String cannot fail");
    }
    rendered
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// Rejects a value carrying newlines or other control characters, so a record
/// field cannot smuggle multi-line output past the digest-only contract.
fn validate_single_line(value: &str, label: &str) -> Result<()> {
    validate_bounded_string(value, label)?;
    if value.chars().any(char::is_control) {
        return Err(DeliveryError::usage(format!(
            "{label} must be a single line without control characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{
        DeliveryErrorKind,
        snapshot::tests::{GitFixture, take},
        storage::tests::{assert_no_absolute_path, repo_root},
    };

    /// Sentinel that must never appear in any file the import writes.
    const SECRET_OUTPUT: &str = "RAW-VALIDATOR-OUTPUT-SENTINEL";

    fn log_path(fixture: &GitFixture) -> PathBuf {
        let path = fixture.scratch().join("validator.log");
        std::fs::write(
            &path,
            format!("running make test-integration\n{SECRET_OUTPUT}\nok\n"),
        )
        .expect("write validator log");
        path
    }

    fn import_args(fixture: &GitFixture, snapshot: &WaveSnapshot, extra: &[&str]) -> Vec<String> {
        let snapshot_path = fixture
            .state()
            .join("W0")
            .join(snapshot.candidate_id.as_str())
            .join("snapshot.json");
        let mut args = vec![
            "--snapshot".to_owned(),
            snapshot_path.display().to_string(),
            "--validation".to_owned(),
            "test-integration".to_owned(),
            "--result".to_owned(),
            "passed".to_owned(),
            "--repo".to_owned(),
            format!("github.com/example/d2b={}", fixture.repo().display()),
        ];
        args.extend(extra.iter().map(|value| (*value).to_owned()));
        args
    }

    fn import_into(fixture: &GitFixture, args: &[String]) -> Result<WorkflowOutput> {
        let request = ImportRequest::parse(args)?;
        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        run_with_root(&request, &root)
    }

    fn files_under(path: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files_under(&path, found);
            } else {
                found.push(path);
            }
        }
    }

    #[test]
    fn a_record_is_addressed_by_candidate_lane_and_validation() {
        let fixture = GitFixture::new("evidence-address");
        let snapshot = take(&fixture);
        let output = import_into(
            &fixture,
            &import_args(&fixture, &snapshot, &["--lane", "local-host"]),
        )
        .expect("import");

        assert_eq!(output.operation.as_str(), "validate-import");
        assert_eq!(
            output.candidate_id.as_deref(),
            Some(snapshot.candidate_id.as_str())
        );
        assert_eq!(
            output.artifact.as_deref(),
            Some(
                format!(
                    "W0/{}/evidence/local-host/test-integration.json",
                    snapshot.candidate_id.as_str()
                )
                .as_str()
            ),
            "the artifact must be a state-root-relative reference, not an absolute path"
        );
        let expected = fixture
            .state()
            .join("W0")
            .join(snapshot.candidate_id.as_str())
            .join("evidence/local-host/test-integration.json");
        assert!(expected.is_file());
    }

    #[test]
    fn raw_command_output_never_reaches_a_written_file() {
        let fixture = GitFixture::new("evidence-no-raw-output");
        let snapshot = take(&fixture);
        let log = log_path(&fixture);
        import_into(
            &fixture,
            &import_args(
                &fixture,
                &snapshot,
                &[
                    "--log",
                    &log.display().to_string(),
                    "--command",
                    "make test-integration",
                ],
            ),
        )
        .expect("import");

        let mut written = Vec::new();
        files_under(&fixture.state(), &mut written);
        assert!(!written.is_empty(), "the import must have written a record");
        for path in &written {
            let contents = std::fs::read(path).expect("read written artifact");
            assert!(
                !String::from_utf8_lossy(&contents).contains(SECRET_OUTPUT),
                "{path:?} carries raw validator output"
            );
        }

        let record: EvidenceRecord = serde_json::from_slice(
            &std::fs::read(
                fixture
                    .state()
                    .join("W0")
                    .join(snapshot.candidate_id.as_str())
                    .join("evidence/local-host/test-integration.json"),
            )
            .expect("read record"),
        )
        .expect("parse record");
        let output = record.output.expect("the log digest is recorded");
        assert_eq!(output.sha256.len(), 64);
        assert_eq!(
            output.bytes,
            std::fs::metadata(&log).expect("stat log").len(),
            "the record must account for every byte it hashed"
        );

        // The repository the wave spans is untouched by the import.
        let mut tracked = Vec::new();
        files_under(&fixture.repo(), &mut tracked);
        for path in tracked {
            let contents = std::fs::read(&path).expect("read repository file");
            assert!(
                !String::from_utf8_lossy(&contents).contains(SECRET_OUTPUT),
                "{path:?} inside the checkout carries raw validator output"
            );
        }
    }

    #[test]
    fn evidence_cannot_be_written_inside_a_repository_checkout() {
        let fixture = GitFixture::new("evidence-external");
        let snapshot = take(&fixture);
        let inside = fixture.repo().join("delivery-state");
        let error = run(&import_args(
            &fixture,
            &snapshot,
            &["--state-dir", &inside.display().to_string()],
        ))
        .expect_err("state inside a checkout must be refused");
        assert!(
            error.message().contains("must not live inside"),
            "unexpected message: {error}"
        );
        assert!(!inside.exists());

        let inside_self = repo_root().join("packages/target/should-never-exist-evidence");
        let error = run(&import_args(
            &fixture,
            &snapshot,
            &["--state-dir", &inside_self.display().to_string()],
        ))
        .expect_err("state inside a Git working tree must be refused");
        assert!(
            error.message().contains("Git working tree")
                || error.message().contains("must not live inside"),
            "unexpected message: {error}"
        );
        assert!(!inside_self.exists());
    }

    #[test]
    fn evidence_for_a_stale_candidate_id_is_rejected() {
        let fixture = GitFixture::new("evidence-stale-guard");
        let snapshot = take(&fixture);
        let stale = CandidateId::parse("b".repeat(64)).expect("digest");
        let error = import_into(
            &fixture,
            &import_args(&fixture, &snapshot, &["--candidate", stale.as_str()]),
        )
        .expect_err("a stale candidate guard must be refused");
        assert!(
            error.message().contains(stale.as_str()) && error.message().contains("re-snapshot")
                || error.message().contains("must rerun"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn evidence_collected_before_a_content_change_is_rejected() {
        let fixture = GitFixture::new("evidence-stale-content");
        let snapshot = take(&fixture);
        let args = import_args(&fixture, &snapshot, &[]);
        import_into(&fixture, &args).expect("import against the current candidate");

        fixture.write("docs/spec.json", "{\"wave\":\"w0\",\"changed\":true}\n");
        fixture.commit("change integrated content");

        let error = import_into(&fixture, &args).expect_err("stale evidence must be refused");
        assert!(
            error.message().contains("superseded content"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn evidence_for_an_unknown_candidate_is_rejected() {
        let fixture = GitFixture::new("evidence-unknown");
        let snapshot = take(&fixture);
        let orphan = fixture.scratch().join("orphan-state");
        let args = import_args(&fixture, &snapshot, &[]);
        let request = ImportRequest::parse(&args).expect("parse");
        let root = StateRoot::for_tests(&orphan).expect("anchor state root");
        let error = run_with_root(&request, &root).expect_err("no snapshot for this candidate");
        assert!(
            error.message().contains("no delivery state"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn a_tampered_snapshot_is_rejected_before_anything_is_written() {
        let fixture = GitFixture::new("evidence-tampered-snapshot");
        let snapshot = take(&fixture);
        let path = fixture
            .state()
            .join("W0")
            .join(snapshot.candidate_id.as_str())
            .join("snapshot.json");
        let forged = fixture.scratch().join("forged-snapshot.json");
        let mut tampered = snapshot.clone();
        tampered.material.wave = "W1".to_owned();
        std::fs::write(
            &forged,
            serde_json::to_vec(&tampered).expect("render forged snapshot"),
        )
        .expect("write forged snapshot");
        assert!(path.is_file());

        let mut args = import_args(&fixture, &snapshot, &[]);
        args[1] = forged.display().to_string();
        assert!(import_into(&fixture, &args).is_err());
    }

    #[test]
    fn a_failed_result_is_recorded_rather_than_refused() {
        let fixture = GitFixture::new("evidence-failed");
        let snapshot = take(&fixture);
        let mut args = import_args(&fixture, &snapshot, &[]);
        args[5] = "failed".to_owned();
        import_into(&fixture, &args).expect("a failed lane is still imported");

        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let candidate = root
            .existing_candidate("W0", &snapshot.candidate_id)
            .expect("candidate");
        let records = read_all(&candidate, &snapshot).expect("read records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].result, EvidenceResult::Failed);
        assert_eq!(records[0].lane, EvidenceLane::LocalHost);
        assert_eq!(records[0].validation, "test-integration");
    }

    #[test]
    fn every_importable_lane_files_under_its_own_directory() {
        let fixture = GitFixture::new("evidence-lanes");
        let snapshot = take(&fixture);
        for lane in REQUIRED_EVIDENCE_LANES {
            import_into(
                &fixture,
                &import_args(&fixture, &snapshot, &["--lane", lane.as_str()]),
            )
            .expect("import");
        }
        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let candidate = root
            .existing_candidate("W0", &snapshot.candidate_id)
            .expect("candidate");
        let records = read_all(&candidate, &snapshot).expect("read records");
        assert_eq!(
            records.iter().map(|record| record.lane).collect::<Vec<_>>(),
            REQUIRED_EVIDENCE_LANES.to_vec()
        );
    }

    #[test]
    fn the_panel_lane_cannot_be_imported_through_validate_import() {
        let fixture = GitFixture::new("evidence-panel-lane");
        let snapshot = take(&fixture);
        let error = ImportRequest::parse(&import_args(&fixture, &snapshot, &["--lane", "panel"]))
            .expect_err("the panel lane is not importable");
        assert_eq!(error.kind(), DeliveryErrorKind::Usage);
        assert!(error.message().contains("panel-attest"), "{error}");
    }

    #[test]
    fn read_all_rejects_a_record_bound_to_another_candidate() {
        let fixture = GitFixture::new("evidence-foreign");
        let snapshot = take(&fixture);
        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let candidate = root
            .existing_candidate("W0", &snapshot.candidate_id)
            .expect("candidate");
        let mut record = ImportRequest::parse(&import_args(&fixture, &snapshot, &[]))
            .expect("parse")
            .record(&snapshot)
            .expect("record");
        import(&candidate, &record).expect("import");

        record.content_id = ContentId::parse("c".repeat(64)).expect("digest");
        candidate
            .write_json(record.relative_path(), &record)
            .expect("write foreign record");
        assert!(read_all(&candidate, &snapshot).is_err());
    }

    #[test]
    fn a_record_cannot_be_filed_under_another_candidates_directory() {
        let fixture = GitFixture::new("evidence-mismatched-dir");
        let snapshot = take(&fixture);
        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let other = CandidateId::parse("d".repeat(64)).expect("digest");
        let candidate = root.candidate("W0", &other).expect("candidate");
        let record = ImportRequest::parse(&import_args(&fixture, &snapshot, &[]))
            .expect("parse")
            .record(&snapshot)
            .expect("record");
        assert!(import(&candidate, &record).is_err());
    }

    #[test]
    fn malformed_invocations_are_usage_errors() {
        let fixture = GitFixture::new("evidence-usage");
        let snapshot = take(&fixture);
        for extra in [
            vec!["--lane", "panel"],
            vec!["--candidate", "not-a-digest"],
            vec!["--command", "make test\nrm -rf /"],
            vec!["--locator", ""],
        ] {
            let args = import_args(&fixture, &snapshot, &extra);
            assert!(
                ImportRequest::parse(&args).is_err(),
                "{extra:?} must be refused"
            );
        }

        let mut args = import_args(&fixture, &snapshot, &[]);
        args[5] = "maybe".to_owned();
        assert_eq!(
            ImportRequest::parse(&args).expect_err("bad result").kind(),
            DeliveryErrorKind::Usage
        );
    }

    #[test]
    fn a_missing_repository_mapping_is_a_usage_error() {
        let fixture = GitFixture::new("evidence-missing-repo");
        let snapshot = take(&fixture);
        let args = import_args(&fixture, &snapshot, &[]);
        let mut request = ImportRequest::parse(&args).expect("parse");
        request.checkouts.clear();
        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let error = run_with_root(&request, &root).expect_err("missing mapping");
        assert_eq!(error.kind(), DeliveryErrorKind::Usage);
    }

    #[test]
    fn a_log_digest_covers_the_whole_file() {
        let fixture = GitFixture::new("evidence-log-digest");
        let path = fixture.scratch().join("big.log");
        let contents = "x".repeat(LOG_CHUNK_BYTES * 2 + 7);
        std::fs::write(&path, &contents).expect("write log");
        let digest = digest_without_retaining(&path).expect("digest");
        assert_eq!(digest.bytes, contents.len() as u64);
        assert_eq!(
            digest.sha256,
            crate::delivery::model::sha256_bytes(contents.as_bytes())
        );
        assert!(digest_without_retaining(&fixture.scratch().join("absent.log")).is_err());
    }

    #[test]
    fn a_non_regular_validator_log_does_not_leak_its_absolute_path() {
        // A directory (not a regular file) exercises the rejection branch. The
        // diagnostic names the artifact role, never the absolute log path.
        let fixture = GitFixture::new("evidence-log-redaction");
        let dir = fixture.scratch().join("not-a-file");
        std::fs::create_dir_all(&dir).expect("create directory");
        let error =
            digest_without_retaining(&dir).expect_err("a directory must not digest as a log");
        assert_no_absolute_path(error.message(), &[fixture.scratch()]);
    }

    /// Drives the whole pipeline through the production code paths: `snapshot`
    /// writes the candidate, `validate-import` writes both validator lanes
    /// through the real writer (never a fabricated fixture), and the panel is
    /// requested and attested. The returned candidate is ready for `seal`,
    /// which consumes exactly what these stages produced. This is the
    /// end-to-end proof that the single evidence ABI closes the writer/reader
    /// gap the two divergent lane enums had opened.
    fn drive_to_sealable(
        name: &str,
    ) -> (
        GitFixture,
        StateRoot,
        CandidateDir,
        crate::delivery::panel::SnapshotView,
    ) {
        let fixture = GitFixture::new(name);
        let snapshot = take(&fixture);

        import_into(
            &fixture,
            &import_args(&fixture, &snapshot, &["--lane", "github-ci"]),
        )
        .expect("import the github-ci lane through the production writer");
        import_into(
            &fixture,
            &import_args(&fixture, &snapshot, &["--lane", "local-host"]),
        )
        .expect("import the local-host lane through the production writer");

        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let snapshot_path = fixture
            .state()
            .join("W0")
            .join(snapshot.candidate_id.as_str())
            .join("snapshot.json");
        let (candidate, view) = crate::delivery::panel::open_candidate(&root, &snapshot_path)
            .expect("open the candidate the snapshot names");

        crate::delivery::panel::request(&candidate, &view).expect("panel request");
        let files = crate::delivery::panel::tests::record_files(&view);
        let records_dir = fixture.scratch().join("panel-records");
        std::fs::create_dir_all(&records_dir).expect("panel records directory");
        for (record_name, bytes) in &files {
            std::fs::write(records_dir.join(record_name), bytes).expect("write panel record");
        }
        crate::delivery::panel::attest(&candidate, &view, &records_dir).expect("panel attest");

        (fixture, root, candidate, view)
    }

    #[test]
    fn snapshot_validate_import_panel_and_seal_complete_end_to_end() {
        let (_fixture, _root, candidate, view) = drive_to_sealable("delivery-e2e-seal");

        let output = crate::delivery::seal::seal(&candidate, &view).expect("seal the wave");
        assert_eq!(output.operation.as_str(), "seal");

        let record: crate::delivery::seal::SealRecord = candidate
            .read_json(crate::delivery::storage::SEAL_FILE)
            .expect("read the sealed record");
        record.validate().expect("the sealed record re-validates");
        assert_eq!(record.candidate_id, view.candidate_id);
        assert_eq!(record.evidence.len(), 2);
        assert_eq!(
            record.panel.records.len(),
            crate::delivery::model::PANEL_ROLES.len()
        );
        assert!(record.panel.unanimous);
    }

    #[test]
    fn a_history_only_rebase_invalidates_lane_evidence_while_the_panel_survives() {
        let (_fixture, _root, candidate, view) = drive_to_sealable("delivery-e2e-rebase");
        crate::delivery::seal::seal(&candidate, &view).expect("seal before the rebase");

        // A history-only rebase preserves candidate_id and content_id but moves
        // snapshot_sha256, so the lane evidence goes stale and the seal refuses.
        let rebased = crate::delivery::panel::tests::rebased(&view);
        let error = crate::delivery::seal::seal(&candidate, &rebased)
            .expect_err("lane evidence is stale after a rebase");
        assert!(error.message().contains("stale snapshot"), "{error}");

        // The panel record set still validates against the rebased snapshot: it
        // binds content identity, not the moved snapshot digest.
        let request = crate::delivery::panel::stored_request(&candidate, &rebased)
            .expect("the panel request survives a history-only rebase");
        let attestation = crate::delivery::panel::attested_records(&candidate, &request)
            .expect("the panel records survive a history-only rebase");
        assert!(attestation.unanimous);
    }

    #[test]
    fn structured_stdout_never_leaks_absolute_state_or_host_paths() {
        let fixture = GitFixture::new("delivery-no-path-leak");
        let snapshot = take(&fixture);

        let mut outputs = Vec::new();
        outputs.push(
            import_into(
                &fixture,
                &import_args(&fixture, &snapshot, &["--lane", "github-ci"]),
            )
            .expect("import the github-ci lane"),
        );
        outputs.push(
            import_into(
                &fixture,
                &import_args(&fixture, &snapshot, &["--lane", "local-host"]),
            )
            .expect("import the local-host lane"),
        );

        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let snapshot_path = fixture
            .state()
            .join("W0")
            .join(snapshot.candidate_id.as_str())
            .join("snapshot.json");
        let (candidate, view) = crate::delivery::panel::open_candidate(&root, &snapshot_path)
            .expect("open the candidate the snapshot names");

        outputs.push(crate::delivery::panel::request(&candidate, &view).expect("panel request"));

        let files = crate::delivery::panel::tests::record_files(&view);
        let records_dir = fixture.scratch().join("panel-records");
        std::fs::create_dir_all(&records_dir).expect("panel records directory");
        for (record_name, bytes) in &files {
            std::fs::write(records_dir.join(record_name), bytes).expect("write panel record");
        }
        outputs.push(
            crate::delivery::panel::attest(&candidate, &view, &records_dir).expect("panel attest"),
        );
        outputs.push(crate::delivery::seal::seal(&candidate, &view).expect("seal the wave"));

        let state = fixture.state();
        let state_str = state.to_string_lossy();
        for output in &outputs {
            let json = serde_json::to_string(output).expect("serialize the workflow output");
            assert!(
                !json.contains(state_str.as_ref()),
                "{} leaked the absolute state path into structured stdout: {json}",
                output.operation
            );
            assert!(
                !json.contains("/home/"),
                "{} leaked a HOME or checkout path into structured stdout: {json}",
                output.operation
            );
            assert!(
                !json.contains("/nix/store/"),
                "{} leaked a store path into structured stdout: {json}",
                output.operation
            );
            if let Some(artifact) = output.artifact.as_deref() {
                assert!(
                    !artifact.starts_with('/'),
                    "{} reported an absolute artifact key: {artifact}",
                    output.operation
                );
            }
        }
    }
}
