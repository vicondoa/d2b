//! Ten-role panel request and attestation (spec section 12.3, work item
//! `ADR046-delivery-005`).
//!
//! `panel-request` writes the candidate-bound request naming exactly the ten
//! roles in [`PANEL_ROLES`] and the required provider, model, and reasoning
//! effort from [`PANEL_PROVIDER_POLICY`], [`PANEL_MODEL_POLICY`], and
//! [`PANEL_REASONING_EFFORT_POLICY`].
//!
//! `panel-attest` validates a directory holding exactly one strict record per
//! role, each bound to the same `candidate_id`, `content_id`, and
//! `snapshot_sha256` as the request, then imports the accepted records into
//! the candidate directory so [`seal`](super::seal) reads them from
//! candidate-addressed state rather than from an operator-supplied path.
//!
//! `signoff` is true if and only if `recommendations` is empty, and unanimous
//! ten-of-ten with zero recommendations is the only passing state. A finding
//! requires a content change, which creates a new snapshot and invalidates
//! every prior record for the wave, so there is deliberately no override, no
//! force flag, and no partial-pass verdict.
//!
//! A history-only rebase is the one thing that does not invalidate a panel.
//! Spec section 12.6 preserves the review because the reviewed content is
//! provably unchanged, and [`stored_request`] implements that by matching a
//! stored request on content identity rather than on the full digest triple.
//! Validator evidence takes the opposite rule; [`seal`](super::seal) explains
//! the asymmetry.
//!
//! Provider, model, and reasoning-effort fields are spec-defined record data
//! and exist only inside the external delivery-state directory. Section 12.5
//! keeps them out of Git, a pull-request body, and a release archive; that is
//! structurally enforced here because every write goes through
//! [`CandidateDir`], which refuses any destination outside the external state
//! root, and because no record content is ever rendered to stdout.

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{CliOptions, WaveCommand, WorkflowOutput},
    model::{
        CandidateDigests, CandidateId, CandidateMaterial, ContentId,
        PANEL_ATTESTATION_ARTIFACT_KIND, PANEL_MODEL_POLICY, PANEL_PROVIDER_POLICY,
        PANEL_REASONING_EFFORT_POLICY, PANEL_REQUEST_ARTIFACT_KIND, PANEL_ROLES, PanelRole,
        SNAPSHOT_ARTIFACT_KIND, SnapshotSha256, ensure_schema, sha256_bytes,
        validate_bounded_string, validate_identifier, validate_program_wave, validate_sha256,
    },
    storage::{
        CandidateDir, MAX_JSON_BYTES, PANEL_DIR, PANEL_REQUEST_FILE, SNAPSHOT_FILE, StateRoot,
    },
};

/// Upper bound on findings carried by one record. A record is a verdict, not a
/// transcript; anything larger is a malformed artifact rather than a review.
const MAX_RECOMMENDATIONS: usize = 64;

/// Every panel record file is named after the role that produced it, so a
/// mislabeled or duplicated role is refused by name as well as by content.
pub fn record_file_name(role: PanelRole) -> String {
    format!("{}.json", role.as_str())
}

/// Reader view of the immutable candidate snapshot.
///
/// The snapshot writer is work item `ADR046-delivery-002`, so this view is
/// deliberately tolerant of fields it does not know: it reads the digests and
/// the material, and re-derives the digests from the material to prove the two
/// agree. Everything downstream binds to the derived value.
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
    ///
    /// `content_id` and `candidate_id` are digests over content-only material,
    /// so equality of this pair is itself the byte-identical content proof a
    /// history-only rebase needs. `snapshot_sha256` is deliberately not part
    /// of it; that value covers the base and head object IDs and is what
    /// detects the rebase.
    pub fn content_identity(&self) -> (&CandidateId, &ContentId) {
        (&self.candidate_id, &self.content_id)
    }

    /// Rejects a snapshot whose recorded digests do not re-derive from its own
    /// material, so a hand-edited candidate address cannot be laundered into
    /// the panel or seal lanes.
    pub fn validate(&self, candidate: &CandidateDir) -> Result<()> {
        ensure_artifact_kind(&self.artifact_kind, SNAPSHOT_ARTIFACT_KIND, "snapshot")?;
        ensure_schema(self.schema_version, "snapshot")?;
        let derived = self.material.digests()?;
        if derived != self.digests() {
            return Err(DeliveryError::new(
                "snapshot digests do not match the snapshot's own material; the candidate \
                 snapshot is not self-consistent",
            ));
        }
        candidate.validate_artifact_address(self.wave(), &self.candidate_id, "snapshot")?;
        Ok(())
    }
}

/// The candidate-bound panel request written by `panel-request`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelRequest {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub snapshot_sha256: SnapshotSha256,
    pub provider: String,
    pub model_version: String,
    pub reasoning_effort: String,
    pub roles: Vec<PanelRole>,
    pub record_artifact_kind: String,
    pub record_schema_version: u32,
    pub record_files: Vec<String>,
}

impl PanelRequest {
    pub fn for_snapshot(snapshot: &SnapshotView) -> Self {
        Self {
            artifact_kind: PANEL_REQUEST_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            program: snapshot.program().to_owned(),
            wave: snapshot.wave().to_owned(),
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            provider: PANEL_PROVIDER_POLICY.to_owned(),
            model_version: PANEL_MODEL_POLICY.to_owned(),
            reasoning_effort: PANEL_REASONING_EFFORT_POLICY.to_owned(),
            roles: PANEL_ROLES.to_vec(),
            record_artifact_kind: PANEL_ATTESTATION_ARTIFACT_KIND.to_owned(),
            record_schema_version: DELIVERY_SCHEMA_VERSION,
            record_files: PANEL_ROLES.iter().copied().map(record_file_name).collect(),
        }
    }

    pub fn digests(&self) -> CandidateDigests {
        CandidateDigests {
            content_id: self.content_id.clone(),
            candidate_id: self.candidate_id.clone(),
            snapshot_sha256: self.snapshot_sha256.clone(),
        }
    }

    /// The content identity this request was issued against.
    pub fn content_identity(&self) -> (&CandidateId, &ContentId) {
        (&self.candidate_id, &self.content_id)
    }

    /// Rejects a request that no longer names the roster or the binding this
    /// build enforces, so an old or hand-edited request cannot lower the bar
    /// for the records attested against it.
    pub fn validate(&self) -> Result<()> {
        ensure_artifact_kind(
            &self.artifact_kind,
            PANEL_REQUEST_ARTIFACT_KIND,
            "panel request",
        )?;
        ensure_schema(self.schema_version, "panel request")?;
        ensure_artifact_kind(
            &self.record_artifact_kind,
            PANEL_ATTESTATION_ARTIFACT_KIND,
            "panel record",
        )?;
        ensure_schema(self.record_schema_version, "panel record")?;
        validate_program_wave(&self.program, &self.wave)?;
        ensure_panel_binding(&self.provider, &self.model_version, &self.reasoning_effort)?;
        if self.roles != PANEL_ROLES {
            return Err(DeliveryError::new(
                "panel request must name exactly the ten-role default roster, in order",
            ));
        }
        let expected = PANEL_ROLES
            .iter()
            .copied()
            .map(record_file_name)
            .collect::<Vec<_>>();
        if self.record_files != expected {
            return Err(DeliveryError::new(
                "panel request must name one record file per role, each named after its role",
            ));
        }
        Ok(())
    }
}

/// One role's strict panel record, exactly as spec section 12.3 shapes it.
///
/// `deny_unknown_fields` plus every field being mandatory is what makes this a
/// strict fourteen-field record: a missing field, an extra field, or a
/// misspelled field is a rejection rather than a default.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelRecord {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub role: PanelRole,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub snapshot_sha256: SnapshotSha256,
    pub model_version: String,
    pub provider: String,
    pub reasoning_effort: String,
    pub run_id: String,
    pub receipt_locator: String,
    pub output_sha256: String,
    pub signoff: bool,
    pub recommendations: Vec<String>,
}

impl PanelRecord {
    /// Validates one record against the request it answers.
    fn validate(&self, request: &PanelRequest) -> Result<()> {
        let role = self.role.as_str();
        ensure_artifact_kind(
            &self.artifact_kind,
            PANEL_ATTESTATION_ARTIFACT_KIND,
            "panel record",
        )?;
        ensure_schema(self.schema_version, "panel record")?;
        ensure_panel_binding(&self.provider, &self.model_version, &self.reasoning_effort)
            .map_err(|error| DeliveryError::new(format!("panel record {role}: {error}")))?;
        if self.candidate_id != request.candidate_id
            || self.content_id != request.content_id
            || self.snapshot_sha256 != request.snapshot_sha256
        {
            return Err(DeliveryError::new(format!(
                "panel record {role} is bound to a different candidate than the panel request; \
                 a content change invalidates every prior record and requires a new snapshot"
            )));
        }
        validate_identifier(&self.run_id, "panel record run identifier")?;
        validate_receipt_locator(&self.receipt_locator, &self.provider)?;
        validate_sha256(&self.output_sha256, "panel record output digest")?;
        if self.recommendations.len() > MAX_RECOMMENDATIONS {
            return Err(DeliveryError::new(format!(
                "panel record {role} carries more than {MAX_RECOMMENDATIONS} recommendations"
            )));
        }
        for recommendation in &self.recommendations {
            validate_bounded_string(recommendation, "panel recommendation")?;
        }
        if self.signoff != self.recommendations.is_empty() {
            return Err(DeliveryError::new(format!(
                "panel record {role} is inconsistent: signoff is true if and only if \
                 recommendations is empty"
            )));
        }
        Ok(())
    }
}

/// One accepted record's provenance, as bound into the seal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttestedRecord {
    pub role: PanelRole,
    pub file: String,
    pub sha256: String,
    pub run_id: String,
}

/// The result of validating a complete record set.
///
/// It exists only in a passing state: [`validate_record_set`] returns an error
/// for anything short of unanimous ten-of-ten, so `unanimous` is always true
/// on a value that exists. It is carried explicitly so the sealed artifact
/// states the property it was sealed on.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelAttestation {
    pub roles: Vec<PanelRole>,
    pub records: Vec<AttestedRecord>,
    pub unanimous: bool,
}

impl PanelAttestation {
    /// Re-checks a deserialized attestation, for readers that did not build it
    /// themselves.
    pub fn validate(&self) -> Result<()> {
        if self.roles != PANEL_ROLES || self.records.len() != PANEL_ROLES.len() {
            return Err(DeliveryError::new(
                "panel attestation must cover exactly the ten-role default roster",
            ));
        }
        for (role, record) in PANEL_ROLES.iter().zip(&self.records) {
            if record.role != *role || record.file != record_file_name(*role) {
                return Err(DeliveryError::new(
                    "panel attestation records are not in roster order",
                ));
            }
            validate_sha256(&record.sha256, "panel record digest")?;
            validate_identifier(&record.run_id, "panel record run identifier")?;
        }
        if !self.unanimous {
            return Err(DeliveryError::new(
                "panel attestation is not unanimous; ten of ten signoffs are required",
            ));
        }
        Ok(())
    }
}

/// One record file as read from disk: its name and its exact bytes.
///
/// Bytes are kept verbatim so the digest bound into the seal is the digest of
/// the file the panel produced, not of a re-serialization.
pub(crate) type RecordFile = (String, Vec<u8>);

/// Validates a complete record set against its request.
///
/// This is the full rejection matrix. Every branch below is a distinct way a
/// record set fails; there is no path that returns a partial pass.
pub fn validate_record_set(
    candidate: &CandidateDir,
    request: &PanelRequest,
    files: &[RecordFile],
) -> Result<PanelAttestation> {
    request.validate()?;
    candidate.validate_artifact_address(
        &request.wave,
        &request.candidate_id,
        "panel request and record set",
    )?;
    if files.len() != PANEL_ROLES.len() {
        return Err(DeliveryError::new(format!(
            "panel needs exactly {} records, one per role, found {}",
            PANEL_ROLES.len(),
            files.len()
        )));
    }

    let mut parsed = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        let role = PANEL_ROLES
            .iter()
            .copied()
            .find(|role| record_file_name(*role) == *name)
            .ok_or_else(|| {
                DeliveryError::new(format!(
                    "panel record file {name:?} is not named after a role on the ten-role roster"
                ))
            })?;
        let record: PanelRecord = serde_json::from_slice(bytes).map_err(|error| {
            DeliveryError::new(format!(
                "panel record {name:?} is not a strict record: {error}"
            ))
        })?;
        if record.role != role {
            return Err(DeliveryError::new(format!(
                "panel record file {name:?} carries role {:?}",
                record.role.as_str()
            )));
        }
        record.validate(request)?;
        parsed.push((role, name.clone(), record, sha256_bytes(bytes)));
    }

    parsed.sort_by_key(|(role, ..)| *role);
    let roles = parsed.iter().map(|(role, ..)| *role).collect::<Vec<_>>();
    let mut roster = PANEL_ROLES.to_vec();
    roster.sort();
    if roles != roster {
        return Err(DeliveryError::new(
            "panel records must cover every role on the ten-role roster exactly once",
        ));
    }

    ensure_distinct(
        parsed
            .iter()
            .map(|(_, _, record, _)| record.run_id.as_str()),
        "run identifier",
    )?;
    ensure_distinct(
        parsed
            .iter()
            .map(|(_, _, record, _)| record.receipt_locator.as_str()),
        "receipt locator",
    )?;
    ensure_distinct(
        parsed
            .iter()
            .map(|(_, _, record, _)| record.output_sha256.as_str()),
        "output digest",
    )?;

    let findings = parsed
        .iter()
        .filter(|(_, _, record, _)| !record.signoff)
        .count();
    if findings > 0 {
        return Err(DeliveryError::new(format!(
            "panel is not unanimous: {findings} of {} roles returned findings; the wave takes a \
             content change, a new snapshot, and a fresh panel",
            PANEL_ROLES.len()
        )));
    }

    let records = PANEL_ROLES
        .iter()
        .map(|role| {
            let (_, file, record, sha256) = parsed
                .iter()
                .find(|(candidate, ..)| candidate == role)
                .expect("every roster role is present");
            AttestedRecord {
                role: *role,
                file: file.clone(),
                sha256: sha256.clone(),
                run_id: record.run_id.clone(),
            }
        })
        .collect();

    Ok(PanelAttestation {
        roles: PANEL_ROLES.to_vec(),
        records,
        unanimous: true,
    })
}

/// Reads the accepted records back out of candidate-addressed state.
///
/// [`seal`](super::seal) uses this rather than re-reading the operator's
/// directory, so the seal binds the records the attestation accepted.
pub fn attested_records(
    candidate: &CandidateDir,
    request: &PanelRequest,
) -> Result<PanelAttestation> {
    let names = candidate.list(PANEL_DIR).map_err(|error| {
        DeliveryError::new(format!(
            "no attested panel records for this candidate; run panel-attest first ({error})"
        ))
    })?;
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let name = name
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| DeliveryError::new("panel record file name is not UTF-8"))?;
        let bytes = candidate.read_bytes(Path::new(PANEL_DIR).join(&name))?;
        files.push((name, bytes));
    }
    validate_record_set(candidate, request, &files)
}

/// Reads and validates the panel request stored for a candidate.
///
/// The request is matched on content identity, not on the full digest triple.
/// A panel reviews content, and spec section 12.6 lets a history-only rebase
/// preserve that review: `candidate_id` and `content_id` are digests over
/// content-only material, so their equality is the byte-identical proof the
/// reuse rests on. `snapshot_sha256` moves with the base and head object IDs
/// and is deliberately not compared here - comparing it would force a fresh
/// ten-role panel after every rebase, which is exactly what section 12.6
/// exists to avoid.
///
/// Validator evidence takes the opposite rule; see
/// [`seal`](super::seal) for why the two classes are asymmetric.
pub fn stored_request(candidate: &CandidateDir, snapshot: &SnapshotView) -> Result<PanelRequest> {
    let request: PanelRequest = candidate.read_json(PANEL_REQUEST_FILE).map_err(|error| {
        DeliveryError::new(format!(
            "no panel request for this candidate; run panel-request first ({error})"
        ))
    })?;
    request.validate()?;
    candidate.validate_artifact_address(
        &request.wave,
        &request.candidate_id,
        "stored panel request",
    )?;
    if request.content_identity() != snapshot.content_identity() {
        return Err(DeliveryError::new(
            "the stored panel request reviewed different content than this snapshot; a content \
             change requires a new snapshot and a fresh panel",
        ));
    }
    Ok(request)
}

/// Opens the candidate directory named by a snapshot artifact reference.
///
/// The candidate address (wave and candidate id) is derived from the
/// reference itself and the snapshot is read through the candidate's pinned
/// directory descriptor (see [`StateRoot::open_candidate_artifact`]), so no
/// supplied path is read and there is no separate canonicalize-and-compare. A
/// reference that does not resolve to a `<wave>/<candidate>/snapshot.json`
/// inside external delivery state fails closed. That is what keeps every later
/// stage reading candidate-addressed state instead of an arbitrary operator
/// path.
pub fn open_candidate(
    state: &StateRoot,
    snapshot_path: &Path,
) -> Result<(CandidateDir, SnapshotView)> {
    let (candidate, snapshot): (CandidateDir, SnapshotView) =
        state.open_candidate_artifact(snapshot_path, SNAPSHOT_FILE, "candidate snapshot")?;
    snapshot.validate(&candidate)?;
    Ok((candidate, snapshot))
}

/// `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave panel-request`.
pub fn run_request(args: &[String]) -> Result<WorkflowOutput> {
    let (state, snapshot_path) = parse_snapshot_invocation(args)?;
    let (candidate, snapshot) = open_candidate(&state, &snapshot_path)?;
    request(&candidate, &snapshot)
}

/// Writes the candidate-bound ten-role request.
pub fn request(candidate: &CandidateDir, snapshot: &SnapshotView) -> Result<WorkflowOutput> {
    let request = PanelRequest::for_snapshot(snapshot);
    request.validate()?;
    candidate.validate_artifact_address(&request.wave, &request.candidate_id, "panel request")?;
    candidate.write_json(PANEL_REQUEST_FILE, &request)?;
    WorkflowOutput::ok(WaveCommand::PanelRequest)
        .with_digests(&snapshot.digests())
        .with_artifact(candidate, &candidate.panel_request_path())
}

/// `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave panel-attest`.
pub fn run_attest(args: &[String]) -> Result<WorkflowOutput> {
    let (state, snapshot_path, records_dir) = parse_attest_invocation(args)?;
    let (candidate, snapshot) = open_candidate(&state, &snapshot_path)?;
    attest(&candidate, &snapshot, &records_dir)
}

/// Validates a directory of records and imports the accepted set.
pub fn attest(
    candidate: &CandidateDir,
    snapshot: &SnapshotView,
    records_dir: &Path,
) -> Result<WorkflowOutput> {
    let request = stored_request(candidate, snapshot)?;
    let files = read_record_dir(records_dir)?;
    let attestation = validate_record_set(candidate, &request, &files)?;

    for (name, bytes) in &files {
        candidate.write_bytes(Path::new(PANEL_DIR).join(name), bytes)?;
    }
    let imported = candidate
        .list(PANEL_DIR)?
        .into_iter()
        .filter_map(|name| name.to_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let expected = attestation
        .records
        .iter()
        .map(|record| record.file.clone())
        .collect::<BTreeSet<_>>();
    if imported.iter().cloned().collect::<BTreeSet<_>>() != expected {
        return Err(DeliveryError::new(
            "the candidate's panel directory holds entries outside the attested record set; \
             remove the stale entries and re-attest",
        ));
    }

    WorkflowOutput::ok(WaveCommand::PanelAttest)
        .with_digests(&snapshot.digests())
        .with_artifact(candidate, &candidate.panel_dir())
}

/// Reads every record file from an operator-supplied directory.
///
/// The directory holds records and nothing else: a subdirectory, a symlink, a
/// dotfile, or a non-JSON file is a rejection, so an unnoticed extra file
/// cannot dilute the ten-record requirement.
fn read_record_dir(dir: &Path) -> Result<Vec<RecordFile>> {
    let metadata = fs::symlink_metadata(dir)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(DeliveryError::new("panel record path is not a directory"));
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| DeliveryError::new("panel record file name is not UTF-8"))?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || !file_type.is_file() || !name.ends_with(".json") {
            return Err(DeliveryError::new(format!(
                "panel record directory holds {name:?}, which is not a regular record file"
            )));
        }
        files.push((name, read_file_limited(&entry.path(), "panel record")?));
    }
    files.sort();
    Ok(files)
}

/// Parses the `--snapshot`, `--repo`, and `--state-dir` options every
/// candidate-bound stage shares.
pub(crate) fn parse_snapshot_invocation(args: &[String]) -> Result<(StateRoot, PathBuf)> {
    let mut options = CliOptions::parse(args)?;
    let snapshot_path = options.required_path("--snapshot")?;
    let state = prepare_state(&mut options)?;
    options.finish()?;
    let snapshot_path = state.resolve_artifact_ref(&snapshot_path);
    Ok((state, snapshot_path))
}

fn parse_attest_invocation(args: &[String]) -> Result<(StateRoot, PathBuf, PathBuf)> {
    let mut options = CliOptions::parse(args)?;
    let snapshot_path = options.required_path("--snapshot")?;
    let records_dir = options.required_path("--records")?;
    let state = prepare_state(&mut options)?;
    options.finish()?;
    let snapshot_path = state.resolve_artifact_ref(&snapshot_path);
    Ok((state, snapshot_path, records_dir))
}

/// Resolves the delivery state root from `--state-dir` and the `--repo`
/// checkouts delivery state must stay outside of.
pub(crate) fn prepare_state(options: &mut CliOptions) -> Result<StateRoot> {
    let state_dir = options.optional_path("--state-dir")?;
    let roots = options
        .repository_roots()?
        .into_values()
        .collect::<Vec<_>>();
    StateRoot::prepare(&roots, state_dir.as_deref())
}

pub(crate) fn ensure_artifact_kind(found: &str, expected: &str, label: &str) -> Result<()> {
    if found != expected {
        return Err(DeliveryError::new(format!(
            "{label} artifact kind must be {expected:?}, found {found:?}"
        )));
    }
    Ok(())
}

fn ensure_panel_binding(provider: &str, model: &str, reasoning_effort: &str) -> Result<()> {
    if provider != PANEL_PROVIDER_POLICY
        || model != PANEL_MODEL_POLICY
        || reasoning_effort != PANEL_REASONING_EFFORT_POLICY
    {
        return Err(DeliveryError::new(
            "panel binding must match the provider, model, and reasoning effort this wave's \
             panel request pins",
        ));
    }
    Ok(())
}

fn validate_receipt_locator(locator: &str, provider: &str) -> Result<()> {
    validate_bounded_string(locator, "panel receipt locator")?;
    let scheme = format!("{provider}://");
    if !locator.starts_with(&scheme) || locator.chars().any(char::is_control) {
        return Err(DeliveryError::new(
            "panel receipt locator must address the bound provider and hold no control \
             characters",
        ));
    }
    Ok(())
}

fn ensure_distinct<'a>(values: impl Iterator<Item = &'a str>, label: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(DeliveryError::new(format!(
                "two panel records share one {label}; each role's provenance must be distinct"
            )));
        }
    }
    Ok(())
}

/// Reads a bounded JSON artifact from an operator-supplied path.
pub(crate) fn read_json_file<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T> {
    let bytes = read_file_limited(path, label)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid {label}: {error}")))
}

fn read_file_limited(path: &Path, label: &str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DeliveryError::new(format!("{label} is not a regular file")));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_JSON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "{label} exceeds {MAX_JSON_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::delivery::{
        DeliveryErrorKind,
        model::fixtures,
        storage::{
            SNAPSHOT_FILE,
            tests::{Scratch, assert_no_absolute_path, repo_root},
        },
    };

    pub(crate) fn snapshot() -> SnapshotView {
        snapshot_from(fixtures::material())
    }

    /// A snapshot view over a caller-supplied material, so tests can seal a
    /// wave whose expected pull-request set is not the single-slice fixture
    /// default (for example a stacked same-repository chain).
    pub(crate) fn snapshot_from(material: CandidateMaterial) -> SnapshotView {
        let digests = material.digests().expect("digests");
        SnapshotView {
            artifact_kind: SNAPSHOT_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            content_id: digests.content_id,
            candidate_id: digests.candidate_id,
            snapshot_sha256: digests.snapshot_sha256,
            material,
        }
    }

    /// Creates the candidate directory and writes the snapshot the way
    /// `wave snapshot` will.
    pub(crate) fn candidate_with_snapshot(
        scratch: &Scratch,
    ) -> (StateRoot, CandidateDir, SnapshotView) {
        candidate_with_snapshot_from(scratch, fixtures::material())
    }

    /// Like [`candidate_with_snapshot`], but binds a caller-supplied material.
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

    pub(crate) fn record(role: PanelRole, snapshot: &SnapshotView) -> PanelRecord {
        PanelRecord {
            artifact_kind: PANEL_ATTESTATION_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            role,
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            model_version: PANEL_MODEL_POLICY.to_owned(),
            provider: PANEL_PROVIDER_POLICY.to_owned(),
            reasoning_effort: PANEL_REASONING_EFFORT_POLICY.to_owned(),
            run_id: format!("run-{}", role.as_str()),
            receipt_locator: format!(
                "{PANEL_PROVIDER_POLICY}://runs/run-{}/{}",
                role.as_str(),
                role.as_str()
            ),
            output_sha256: sha256_bytes(role.as_str().as_bytes()),
            signoff: true,
            recommendations: Vec::new(),
        }
    }

    pub(crate) fn record_files(snapshot: &SnapshotView) -> Vec<RecordFile> {
        PANEL_ROLES
            .iter()
            .map(|role| {
                (
                    record_file_name(*role),
                    serde_json::to_vec(&record(*role, snapshot)).expect("record"),
                )
            })
            .collect()
    }

    /// Writes a record set into an operator-style directory.
    pub(crate) fn write_record_dir(scratch: &Scratch, files: &[RecordFile]) -> PathBuf {
        let dir = scratch.path.join("records");
        fs::create_dir_all(&dir).expect("records directory");
        for (name, bytes) in files {
            fs::write(dir.join(name), bytes).expect("write record");
        }
        dir
    }

    fn requested(candidate: &CandidateDir, snapshot: &SnapshotView) -> PanelRequest {
        request(candidate, snapshot).expect("panel request");
        stored_request(candidate, snapshot).expect("stored request")
    }

    #[test]
    fn a_refused_panel_read_names_the_label_not_the_path() {
        // Point a bounded read at a directory. The read refuses it, and the
        // diagnostic - which reaches operator stderr and CI logs verbatim -
        // must name the semantic label only, never the absolute path.
        let scratch = Scratch::new("panel-read-redaction");
        let decoy = scratch.path.join("panel-record");
        std::fs::create_dir_all(&decoy).expect("create the decoy directory");
        let error = read_file_limited(&decoy, "panel record")
            .expect_err("a directory must not read as a record file");
        let message = error.message();
        assert_no_absolute_path(message, &[&scratch.path, &decoy]);
        assert!(
            message.contains("panel record"),
            "the diagnostic must name the semantic label: {message}"
        );
    }

    #[test]
    fn a_request_binds_the_candidate_the_roster_and_the_model() {
        let scratch = Scratch::new("panel-request");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);

        let output = request(&candidate, &snapshot).expect("panel request");
        assert_eq!(output.operation.as_str(), "panel-request");
        assert_eq!(
            output.candidate_id.as_deref(),
            Some(snapshot.candidate_id.as_str())
        );
        assert_eq!(
            output.artifact.as_deref(),
            Some(format!("W0/{}/panel-request.json", snapshot.candidate_id.as_str()).as_str()),
            "the artifact must be a state-root-relative reference, not an absolute path"
        );

        let stored = stored_request(&candidate, &snapshot).expect("stored request");
        assert_eq!(stored.roles, PANEL_ROLES.to_vec());
        assert_eq!(stored.provider, PANEL_PROVIDER_POLICY);
        assert_eq!(stored.model_version, PANEL_MODEL_POLICY);
        assert_eq!(stored.reasoning_effort, PANEL_REASONING_EFFORT_POLICY);
        assert_eq!(stored.digests(), snapshot.digests());
    }

    #[test]
    fn a_request_never_leaves_the_external_state_directory() {
        let scratch = Scratch::new("panel-request-location");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        request(&candidate, &snapshot).expect("panel request");
        assert!(candidate.panel_request_path().starts_with(&scratch.path));

        let inside_repository = StateRoot::prepare(&[], Some(&repo_root().join("delivery-state")));
        assert!(
            inside_repository.is_err(),
            "delivery state must never resolve inside a checkout"
        );
    }

    #[test]
    fn ten_valid_records_attest_and_import() {
        let scratch = Scratch::new("panel-attest");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);
        let files = record_files(&snapshot);
        let dir = write_record_dir(&scratch, &files);

        let output = attest(&candidate, &snapshot, &dir).expect("attest");
        assert_eq!(output.operation.as_str(), "panel-attest");
        assert_eq!(candidate.list(PANEL_DIR).expect("panel dir").len(), 10);

        let request = stored_request(&candidate, &snapshot).expect("request");
        let attestation = attested_records(&candidate, &request).expect("attested");
        assert!(attestation.unanimous);
        assert_eq!(attestation.records.len(), 10);
        attestation.validate().expect("round trip");
    }

    #[test]
    fn attestation_requires_a_request_first() {
        let scratch = Scratch::new("panel-attest-no-request");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let dir = write_record_dir(&scratch, &record_files(&snapshot));
        let error = attest(&candidate, &snapshot, &dir).expect_err("no request");
        assert!(error.message().contains("panel-request"), "{error}");
    }

    /// Builds the snapshot a history-only rebase produces: the same content,
    /// so the same `candidate_id` and `content_id`, on moved commits, so a
    /// different `snapshot_sha256`.
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

    #[test]
    fn a_history_only_rebase_reuses_the_stored_request_and_records() {
        let scratch = Scratch::new("panel-rebase-reuse");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);
        let dir = write_record_dir(&scratch, &record_files(&snapshot));
        attest(&candidate, &snapshot, &dir).expect("attest");

        let rebased = rebased(&snapshot);
        let request = stored_request(&candidate, &rebased).expect("request survives the rebase");
        let attestation = attested_records(&candidate, &request).expect("records survive");
        assert!(attestation.unanimous);
        assert_eq!(attestation.records.len(), PANEL_ROLES.len());
    }

    #[test]
    fn a_content_change_does_not_reuse_the_stored_request() {
        let scratch = Scratch::new("panel-content-change");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);

        let mut changed = snapshot.clone();
        changed.material.repository_set[0].integration_tree_oid = fixtures::oid(9);
        let digests = changed.material.digests().expect("digests");
        assert_ne!(digests.candidate_id, snapshot.candidate_id);
        changed.candidate_id = digests.candidate_id;
        changed.content_id = digests.content_id;
        changed.snapshot_sha256 = digests.snapshot_sha256;

        let error = stored_request(&candidate, &changed).expect_err("content changed");
        assert!(error.message().contains("different content"), "{error}");
    }

    fn reject(mutate: impl FnOnce(&mut Vec<RecordFile>, &SnapshotView)) -> DeliveryError {
        let scratch = Scratch::new("panel-reject");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let request = PanelRequest::for_snapshot(&snapshot);
        let mut files = record_files(&snapshot);
        mutate(&mut files, &snapshot);
        validate_record_set(&candidate, &request, &files).expect_err("record set must be rejected")
    }

    fn rewrite(files: &mut [RecordFile], role: PanelRole, mutate: impl FnOnce(&mut PanelRecord)) {
        let name = record_file_name(role);
        let entry = files
            .iter_mut()
            .find(|(file, _)| *file == name)
            .expect("role present");
        let mut record: PanelRecord = serde_json::from_slice(&entry.1).expect("record");
        mutate(&mut record);
        entry.1 = serde_json::to_vec(&record).expect("record");
    }

    #[test]
    fn a_complete_unanimous_set_is_accepted() {
        let scratch = Scratch::new("panel-complete-set");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let request = PanelRequest::for_snapshot(&snapshot);
        let attestation = validate_record_set(&candidate, &request, &record_files(&snapshot))
            .expect("unanimous set");
        assert_eq!(attestation.roles, PANEL_ROLES.to_vec());
        assert!(attestation.unanimous);
    }

    #[test]
    fn a_missing_role_is_rejected() {
        let error = reject(|files, _| {
            files.retain(|(name, _)| *name != record_file_name(PanelRole::Kernel));
        });
        assert!(error.message().contains("exactly 10 records"), "{error}");
    }

    #[test]
    fn an_extra_record_is_rejected() {
        let error = reject(|files, snapshot| {
            files.push((
                "extra.json".to_owned(),
                serde_json::to_vec(&record(PanelRole::Rust, snapshot)).expect("record"),
            ));
        });
        assert_eq!(error.kind(), DeliveryErrorKind::Invalid);
    }

    #[test]
    fn a_duplicated_role_is_rejected() {
        let error = reject(|files, snapshot| {
            let name = record_file_name(PanelRole::Docs);
            let entry = files
                .iter_mut()
                .find(|(file, _)| *file == name)
                .expect("role present");
            let mut duplicate = record(PanelRole::Rust, snapshot);
            duplicate.run_id = "run-duplicate".to_owned();
            duplicate.receipt_locator =
                format!("{PANEL_PROVIDER_POLICY}://runs/run-duplicate/rust");
            entry.1 = serde_json::to_vec(&duplicate).expect("record");
        });
        assert!(error.message().contains("carries role"), "{error}");
    }

    #[test]
    fn a_wrong_model_provider_or_reasoning_effort_is_rejected() {
        for mutate in [
            (|record: &mut PanelRecord| record.model_version = "other-model".to_owned())
                as fn(&mut PanelRecord),
            |record: &mut PanelRecord| record.provider = "other-provider".to_owned(),
            |record: &mut PanelRecord| record.reasoning_effort = "medium".to_owned(),
        ] {
            let error = reject(|files, _| rewrite(files, PanelRole::Security, mutate));
            assert!(error.message().contains("panel binding"), "{error}");
        }
    }

    #[test]
    fn a_record_bound_to_another_candidate_is_rejected() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Rust, |record| {
                record.candidate_id = CandidateId::parse("b".repeat(64)).expect("digest");
            });
        });
        assert!(error.message().contains("different candidate"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Rust, |record| {
                record.content_id = ContentId::parse("c".repeat(64)).expect("digest");
            });
        });
        assert!(error.message().contains("different candidate"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Rust, |record| {
                record.snapshot_sha256 = SnapshotSha256::parse("d".repeat(64)).expect("digest");
            });
        });
        assert!(error.message().contains("different candidate"), "{error}");
    }

    #[test]
    fn duplicate_run_provenance_is_rejected() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Test, |record| {
                record.run_id = "run-software".to_owned();
            });
        });
        assert!(error.message().contains("run identifier"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Test, |record| {
                record.receipt_locator =
                    format!("{PANEL_PROVIDER_POLICY}://runs/run-software/software");
            });
        });
        assert!(error.message().contains("receipt locator"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Test, |record| {
                record.output_sha256 = sha256_bytes(PanelRole::Software.as_str().as_bytes());
            });
        });
        assert!(error.message().contains("output digest"), "{error}");
    }

    #[test]
    fn an_inconsistent_signoff_is_rejected_in_both_directions() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Product, |record| {
                record.recommendations = vec!["operator error message is unclear".to_owned()];
            });
        });
        assert!(error.message().contains("if and only if"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Product, |record| {
                record.signoff = false;
            });
        });
        assert!(error.message().contains("if and only if"), "{error}");
    }

    #[test]
    fn a_finding_blocks_the_panel_even_when_the_record_is_consistent() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Observability, |record| {
                record.signoff = false;
                record.recommendations = vec!["metric label cardinality is unbounded".to_owned()];
            });
        });
        assert!(error.message().contains("not unanimous"), "{error}");
        assert!(
            !error.message().contains("cardinality"),
            "a rejection must not echo record content: {error}"
        );
    }

    #[test]
    fn a_malformed_record_is_rejected() {
        for (label, bytes) in [
            ("not json", b"{".to_vec()),
            (
                "unknown field",
                br#"{"artifact_kind":"d2b-delivery/panel-receipt","schema_version":1,"role":"rust","extra":true}"#
                    .to_vec(),
            ),
        ] {
            let error = reject(|files, _| {
                let name = record_file_name(PanelRole::Rust);
                let entry = files
                    .iter_mut()
                    .find(|(file, _)| *file == name)
                    .expect("role present");
                entry.1 = bytes;
            });
            assert_eq!(error.kind(), DeliveryErrorKind::Invalid, "{label}");
        }
    }

    #[test]
    fn a_wrong_artifact_kind_or_schema_version_is_rejected() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Nixos, |record| {
                record.artifact_kind = "d2b-delivery/wave-snapshot".to_owned();
            });
        });
        assert!(error.message().contains("artifact kind"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Nixos, |record| {
                record.schema_version = DELIVERY_SCHEMA_VERSION + 1;
            });
        });
        assert!(error.message().contains("schema version"), "{error}");
    }

    #[test]
    fn a_malformed_locator_run_identifier_or_output_digest_is_rejected() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Networking, |record| {
                record.receipt_locator = "https://example.invalid/run".to_owned();
            });
        });
        assert!(error.message().contains("receipt locator"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Networking, |record| {
                record.run_id = "RUN 001".to_owned();
            });
        });
        assert!(error.message().contains("run identifier"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Networking, |record| {
                record.output_sha256 = "not-a-digest".to_owned();
            });
        });
        assert!(error.message().contains("output digest"), "{error}");
    }

    #[test]
    fn a_request_that_weakens_the_roster_or_binding_is_rejected() {
        let scratch = Scratch::new("panel-weakened-request");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        for mutate in [
            (|request: &mut PanelRequest| {
                request.roles.retain(|role| *role != PanelRole::Kernel);
            }) as fn(&mut PanelRequest),
            |request: &mut PanelRequest| request.model_version = "other-model".to_owned(),
            |request: &mut PanelRequest| request.provider = "other-provider".to_owned(),
            |request: &mut PanelRequest| request.reasoning_effort = "low".to_owned(),
            |request: &mut PanelRequest| request.record_files.clear(),
            |request: &mut PanelRequest| request.schema_version = DELIVERY_SCHEMA_VERSION + 1,
        ] {
            let mut request = PanelRequest::for_snapshot(&snapshot);
            mutate(&mut request);
            assert!(
                validate_record_set(&candidate, &request, &record_files(&snapshot)).is_err(),
                "a weakened request must be refused"
            );
        }
    }

    #[test]
    fn a_records_directory_holding_anything_else_is_rejected() {
        let scratch = Scratch::new("panel-records-dir");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);
        let files = record_files(&snapshot);
        let dir = write_record_dir(&scratch, &files);
        fs::create_dir(dir.join("nested")).expect("nested directory");
        let error = attest(&candidate, &snapshot, &dir).expect_err("nested entry");
        assert!(
            error.message().contains("not a regular record file"),
            "{error}"
        );
    }

    #[test]
    fn a_snapshot_with_forged_digests_is_rejected() {
        let scratch = Scratch::new("panel-forged-snapshot");
        let (_state, candidate, mut snapshot) = candidate_with_snapshot(&scratch);
        snapshot.candidate_id = CandidateId::parse("e".repeat(64)).expect("digest");
        let error = snapshot.validate(&candidate).expect_err("forged digests");
        assert!(error.message().contains("self-consistent"), "{error}");
    }

    #[test]
    fn a_snapshot_copied_to_another_candidate_address_is_rejected() {
        let scratch = Scratch::new("panel-copied-snapshot");
        let (state, _candidate, snapshot) = candidate_with_snapshot(&scratch);
        let other_id = CandidateId::parse("e".repeat(64)).expect("candidate id");
        let other = state
            .candidate(snapshot.wave(), &other_id)
            .expect("second candidate");
        other
            .write_json(SNAPSHOT_FILE, &snapshot)
            .expect("copy snapshot");

        let error = open_candidate(&state, &other.snapshot_path())
            .expect_err("copied snapshot must not change candidate identity");
        assert_eq!(error.kind(), DeliveryErrorKind::Invalid);
        assert!(
            error.message().contains("delivery-state address"),
            "{error}"
        );
    }

    #[test]
    fn a_snapshot_copied_to_another_wave_address_is_rejected() {
        let scratch = Scratch::new("panel-copied-snapshot-wave");
        let (state, _candidate, snapshot) = candidate_with_snapshot(&scratch);
        let other = state
            .candidate("W1", &snapshot.candidate_id)
            .expect("second wave");
        other
            .write_json(SNAPSHOT_FILE, &snapshot)
            .expect("copy snapshot");

        let error = open_candidate(&state, &other.snapshot_path())
            .expect_err("copied snapshot must not change wave identity");
        assert_eq!(error.kind(), DeliveryErrorKind::Invalid);
        assert!(
            error.message().contains("delivery-state address"),
            "{error}"
        );
    }

    #[test]
    fn panel_records_copied_to_another_candidate_address_are_rejected() {
        let scratch = Scratch::new("panel-copied-records");
        let (state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let request = requested(&candidate, &snapshot);
        let dir = write_record_dir(&scratch, &record_files(&snapshot));
        attest(&candidate, &snapshot, &dir).expect("attest original records");

        let other_id = CandidateId::parse("e".repeat(64)).expect("candidate id");
        let other = state
            .candidate(snapshot.wave(), &other_id)
            .expect("second candidate");
        other
            .write_json(PANEL_REQUEST_FILE, &request)
            .expect("copy panel request");
        for record in candidate.list(PANEL_DIR).expect("panel records") {
            let bytes = candidate
                .read_bytes(Path::new(PANEL_DIR).join(&record))
                .expect("read record");
            other
                .write_bytes(Path::new(PANEL_DIR).join(&record), &bytes)
                .expect("copy record");
        }

        let error = attested_records(&other, &request)
            .expect_err("copied records must not change candidate identity");
        assert_eq!(error.kind(), DeliveryErrorKind::Invalid);
        assert!(
            error.message().contains("delivery-state address"),
            "{error}"
        );
    }

    #[test]
    fn a_snapshot_outside_candidate_state_is_rejected() {
        let scratch = Scratch::new("panel-foreign-snapshot");
        let (state, _candidate, snapshot) = candidate_with_snapshot(&scratch);
        let foreign = scratch.path.join("foreign-snapshot.json");
        fs::write(&foreign, serde_json::to_vec(&snapshot).expect("snapshot")).expect("write");
        let error = open_candidate(&state, &foreign).expect_err("foreign path");
        assert!(
            error.message().contains("external delivery state"),
            "{error}"
        );
    }

    #[test]
    fn the_cli_rejects_a_missing_or_stray_option() {
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            run_request(&args(&["--repo", "github.com/example/d2b=/checkout"]))
                .expect_err("missing --snapshot")
                .kind(),
            DeliveryErrorKind::Usage
        );
        assert_eq!(
            run_attest(&args(&[
                "--snapshot",
                "/state/snapshot.json",
                "--repo",
                "github.com/example/d2b=/checkout"
            ]))
            .expect_err("missing --records")
            .kind(),
            DeliveryErrorKind::Usage
        );
    }
}
