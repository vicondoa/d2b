use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{ObservedCheckState, PullRequestStatus, PullRequestStatusSource, RepositoryProbe},
    evidence::{EvidenceProvenance, verify_evidence_in_context},
    model::{
        EvidenceResult, Fingerprint, HISTORY_PROOF_ARTIFACT_KIND, MAX_CHECKS, PANEL_ROLES,
        PanelRole, RepositoryBinding, RequiredCheck, RequiredValidation, SEAL_ARTIFACT_KIND,
        StackNode, WaveSnapshot, ensure_schema, validate_bounded_string, validate_identifier,
        validate_sha256,
    },
    panel::read_stored_panel,
    snapshot::{CurrentVerification, SnapshotContext, load_snapshot_context, verify_pr_identity},
    storage::{ensure_external_path, read_verified_json, verify_json_digest, write_immutable_json},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WaveSeal {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub candidate_id: String,
    pub content_id: String,
    pub snapshot_sha256: String,
    pub repository_set: Vec<RepositoryBinding>,
    pub live_pull_requests: Vec<PullRequestStatus>,
    pub validation_payloads: Vec<ValidationPayloadBinding>,
    pub panel_payloads: Vec<PanelPayloadBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationPayloadBinding {
    pub id: String,
    pub sha256: String,
    pub github_attested: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelPayloadBinding {
    pub role: PanelRole,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryProof {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub old_candidate_id: String,
    pub new_candidate_id: String,
    pub old_content_id: String,
    pub new_content_id: String,
    pub old_snapshot_sha256: String,
    pub new_snapshot_sha256: String,
    pub old_seal_sha256: String,
    pub old_repositories: Vec<RepositoryBinding>,
    pub new_repositories: Vec<RepositoryBinding>,
    pub old_stack: Vec<StackNode>,
    pub new_stack: Vec<StackNode>,
    pub unchanged_required_validations: Vec<RequiredValidation>,
    pub unchanged_required_checks: Vec<RequiredCheck>,
    pub unchanged_generated_artifacts: Vec<Fingerprint>,
    pub unchanged_dependency_fingerprints: Vec<Fingerprint>,
    pub unchanged_contract_fingerprints: Vec<Fingerprint>,
    pub reused_panel_payloads: Vec<PanelPayloadBinding>,
    pub fresh_ci_required: bool,
}

pub fn construct_seal<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
) -> Result<PathBuf> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    let validation_payloads = collect_validation_payloads(&context)?;
    let panel_payloads = collect_panel_payloads(&context)?;
    let live_pull_requests = collect_live_prs(&context.snapshot, status_source)?;
    let seal = WaveSeal {
        artifact_kind: SEAL_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: context.snapshot.program.clone(),
        wave: context.snapshot.wave.clone(),
        candidate_id: context.snapshot.candidate_id.clone(),
        content_id: context.snapshot.content_id.clone(),
        snapshot_sha256: context.digest.clone(),
        repository_set: context.snapshot.repository_bindings(),
        live_pull_requests,
        validation_payloads,
        panel_payloads,
    };
    validate_seal(&seal, &context)?;
    let path = context.layout.seal();
    write_immutable_json(&path, &seal)?;
    Ok(path)
}

pub fn verify_seal<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
) -> Result<WaveSeal> {
    let (context, seal) = load_seal_context(
        probe,
        repository_roots,
        seal_path,
        CurrentVerification::ExactRefs,
    )?;
    let current = collect_live_prs(&context.snapshot, status_source)?;
    if current != seal.live_pull_requests {
        return Err(DeliveryError::new(
            "live PR/check authority changed from the wave seal",
        ));
    }
    verify_seal_payloads(&context, &seal)?;
    verify_json_digest(seal_path)?;
    Ok(seal)
}

pub(crate) fn verify_seal_recorded<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
) -> Result<(SnapshotContext, WaveSeal)> {
    let (context, seal) = load_seal_context(
        probe,
        repository_roots,
        seal_path,
        CurrentVerification::RecordedObjects,
    )?;
    verify_seal_payloads(&context, &seal)?;
    verify_json_digest(seal_path)?;
    Ok((context, seal))
}

pub fn construct_history_proof<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
) -> Result<PathBuf> {
    let (old_context, old_seal) = verify_seal_recorded(probe, repository_roots, old_seal_path)?;
    let new_context = load_snapshot_context(
        probe,
        repository_roots,
        new_snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    verify_history_only_equivalence(&old_context.snapshot, &new_context.snapshot)?;
    if old_context.snapshot.candidate_id == new_context.snapshot.candidate_id {
        return Err(DeliveryError::new(
            "history proof requires distinct old and new candidate IDs",
        ));
    }
    let proof = HistoryProof {
        artifact_kind: HISTORY_PROOF_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: new_context.snapshot.program.clone(),
        wave: new_context.snapshot.wave.clone(),
        old_candidate_id: old_context.snapshot.candidate_id.clone(),
        new_candidate_id: new_context.snapshot.candidate_id.clone(),
        old_content_id: old_context.snapshot.content_id.clone(),
        new_content_id: new_context.snapshot.content_id.clone(),
        old_snapshot_sha256: old_context.digest,
        new_snapshot_sha256: new_context.digest,
        old_seal_sha256: verify_json_digest(old_seal_path)?,
        old_repositories: old_seal.repository_set,
        new_repositories: new_context.snapshot.repository_bindings(),
        old_stack: old_context.snapshot.stack.clone(),
        new_stack: new_context.snapshot.stack.clone(),
        unchanged_required_validations: new_context.snapshot.required_validations.clone(),
        unchanged_required_checks: new_context.snapshot.required_checks.clone(),
        unchanged_generated_artifacts: new_context.snapshot.generated_artifacts.clone(),
        unchanged_dependency_fingerprints: new_context.snapshot.dependency_fingerprints.clone(),
        unchanged_contract_fingerprints: new_context.snapshot.contract_fingerprints.clone(),
        reused_panel_payloads: old_seal.panel_payloads,
        fresh_ci_required: true,
    };
    validate_history_proof_values(&proof)?;
    let path = new_context.layout.history_proof();
    write_immutable_json(&path, &proof)?;
    Ok(path)
}

pub fn verify_history_proof<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
) -> Result<(WaveSnapshot, WaveSeal, HistoryProof)> {
    let (context, seal, proof) = verify_history_proof_context(
        probe,
        repository_roots,
        old_seal_path,
        new_snapshot_path,
        proof_path,
    )?;
    Ok((context.snapshot, seal, proof))
}

pub(crate) fn verify_history_proof_context<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
) -> Result<(SnapshotContext, WaveSeal, HistoryProof)> {
    let (old_context, old_seal) = verify_seal_recorded(probe, repository_roots, old_seal_path)?;
    let new_context = load_snapshot_context(
        probe,
        repository_roots,
        new_snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    let (proof, _proof_digest): (HistoryProof, String) = read_verified_json(proof_path)?;
    validate_history_proof_values(&proof)?;
    verify_history_only_equivalence(&old_context.snapshot, &new_context.snapshot)?;
    let expected_path = new_context.layout.history_proof();
    if super::storage::absolute_path(proof_path)? != super::storage::absolute_path(&expected_path)?
    {
        return Err(DeliveryError::new(
            "history proof is outside the new candidate directory",
        ));
    }
    if proof.program != new_context.snapshot.program
        || proof.wave != new_context.snapshot.wave
        || proof.old_candidate_id != old_context.snapshot.candidate_id
        || proof.new_candidate_id != new_context.snapshot.candidate_id
        || proof.old_content_id != old_context.snapshot.content_id
        || proof.new_content_id != new_context.snapshot.content_id
        || proof.old_snapshot_sha256 != old_context.digest
        || proof.new_snapshot_sha256 != new_context.digest
        || proof.old_seal_sha256 != verify_json_digest(old_seal_path)?
        || proof.old_repositories != old_seal.repository_set
        || proof.new_repositories != new_context.snapshot.repository_bindings()
        || proof.old_stack != old_context.snapshot.stack
        || proof.new_stack != new_context.snapshot.stack
        || proof.unchanged_required_validations != new_context.snapshot.required_validations
        || proof.unchanged_required_checks != new_context.snapshot.required_checks
        || proof.unchanged_generated_artifacts != new_context.snapshot.generated_artifacts
        || proof.unchanged_dependency_fingerprints != new_context.snapshot.dependency_fingerprints
        || proof.unchanged_contract_fingerprints != new_context.snapshot.contract_fingerprints
        || proof.reused_panel_payloads != old_seal.panel_payloads
        || !proof.fresh_ci_required
    {
        return Err(DeliveryError::new(
            "history proof does not exactly bind the old seal and new verified snapshot",
        ));
    }
    Ok((new_context, old_seal, proof))
}

pub fn verify_history_only_equivalence(
    sealed: &WaveSnapshot,
    candidate: &WaveSnapshot,
) -> Result<()> {
    sealed.validate()?;
    candidate.validate()?;
    if sealed.program != candidate.program
        || sealed.wave != candidate.wave
        || sealed.content_id != candidate.content_id
        || sealed
            .repository_set
            .iter()
            .map(|repository| {
                (
                    &repository.id,
                    repository.object_format,
                    &repository.integration_tree_oid,
                )
            })
            .collect::<Vec<_>>()
            != candidate
                .repository_set
                .iter()
                .map(|repository| {
                    (
                        &repository.id,
                        repository.object_format,
                        &repository.integration_tree_oid,
                    )
                })
                .collect::<Vec<_>>()
        || sealed.required_validations != candidate.required_validations
        || sealed.required_checks != candidate.required_checks
        || sealed.generated_artifacts != candidate.generated_artifacts
        || sealed.dependency_fingerprints != candidate.dependency_fingerprints
        || sealed.contract_fingerprints != candidate.contract_fingerprints
    {
        return Err(DeliveryError::new(
            "candidate is not history-only: content, dependencies, contracts, repository set, or required gates changed",
        ));
    }
    Ok(())
}

fn load_seal_context<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
    verification: CurrentVerification,
) -> Result<(SnapshotContext, WaveSeal)> {
    if seal_path.file_name().and_then(|name| name.to_str()) != Some("seal.json") {
        return Err(DeliveryError::new("seal path must end in seal.json"));
    }
    let candidate = seal_path
        .parent()
        .ok_or_else(|| DeliveryError::new("seal path has no candidate directory"))?;
    let snapshot_path = candidate.join("snapshot.json");
    let context = load_snapshot_context(probe, repository_roots, &snapshot_path, verification)?;
    ensure_external_path(seal_path, &context.external_exclusions)?;
    if super::storage::absolute_path(seal_path)?
        != super::storage::absolute_path(&context.layout.seal())?
    {
        return Err(DeliveryError::new(
            "seal is outside its candidate directory",
        ));
    }
    let (seal, _seal_digest): (WaveSeal, String) = read_verified_json(seal_path)?;
    validate_seal(&seal, &context)?;
    Ok((context, seal))
}

fn validate_seal(seal: &WaveSeal, context: &SnapshotContext) -> Result<()> {
    if seal.artifact_kind != SEAL_ARTIFACT_KIND {
        return Err(DeliveryError::new("invalid wave seal artifact_kind"));
    }
    ensure_schema(seal.schema_version, "wave seal")?;
    validate_identifier(&seal.program, "program")?;
    validate_identifier(&seal.wave, "wave")?;
    validate_sha256(&seal.candidate_id, "seal candidate ID")?;
    validate_sha256(&seal.content_id, "seal content ID")?;
    validate_sha256(&seal.snapshot_sha256, "seal snapshot digest")?;
    if seal.program != context.snapshot.program
        || seal.wave != context.snapshot.wave
        || seal.candidate_id != context.snapshot.candidate_id
        || seal.content_id != context.snapshot.content_id
        || seal.snapshot_sha256 != context.digest
        || seal.repository_set != context.snapshot.repository_bindings()
    {
        return Err(DeliveryError::new(
            "seal identity does not exactly match its snapshot",
        ));
    }
    if seal.live_pull_requests.len() != context.snapshot.stack.len() {
        return Err(DeliveryError::new("seal live PR binding set is incomplete"));
    }
    let mut pr_keys = BTreeSet::new();
    for status in &seal.live_pull_requests {
        if !pr_keys.insert((status.repository.as_str(), status.number)) {
            return Err(DeliveryError::new("seal repeats a live PR binding"));
        }
        let node = context
            .snapshot
            .stack
            .iter()
            .find(|node| node.repository == status.repository && node.pr_number == status.number)
            .ok_or_else(|| DeliveryError::new("seal contains an unexpected live PR"))?;
        verify_pr_identity(node, status)?;
        verify_required_checks(&context.snapshot, node, status)?;
    }
    validate_payload_binding_sets(
        &context.snapshot,
        &seal.validation_payloads,
        &seal.panel_payloads,
    )
}

fn validate_payload_binding_sets(
    snapshot: &WaveSnapshot,
    validations: &[ValidationPayloadBinding],
    panels: &[PanelPayloadBinding],
) -> Result<()> {
    let expected_validations = snapshot
        .required_validations
        .iter()
        .map(|validation| validation.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen_validations = BTreeSet::new();
    for payload in validations {
        validate_identifier(&payload.id, "validation id")?;
        validate_sha256(&payload.sha256, "validation payload digest")?;
        if !seen_validations.insert(payload.id.as_str()) {
            return Err(DeliveryError::new("seal repeats a validation payload"));
        }
    }
    if seen_validations != expected_validations
        || !validations.windows(2).all(|pair| pair[0] < pair[1])
    {
        return Err(DeliveryError::new(
            "seal validation payload set is incomplete or unsorted",
        ));
    }
    let expected_roles = PANEL_ROLES.into_iter().collect::<BTreeSet<_>>();
    let mut seen_roles = BTreeSet::new();
    for payload in panels {
        validate_sha256(&payload.sha256, "panel payload digest")?;
        if !seen_roles.insert(payload.role) {
            return Err(DeliveryError::new("seal repeats a panel role"));
        }
    }
    if seen_roles != expected_roles || !panels.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(DeliveryError::new(
            "seal panel payload set is incomplete or unsorted",
        ));
    }
    Ok(())
}

fn verify_seal_payloads(context: &SnapshotContext, seal: &WaveSeal) -> Result<()> {
    let validations = collect_validation_payloads(context)?;
    if validations != seal.validation_payloads {
        return Err(DeliveryError::new(
            "seal validation payload bindings changed",
        ));
    }
    let panels = collect_panel_payloads(context)?;
    if panels != seal.panel_payloads {
        return Err(DeliveryError::new("seal panel payload bindings changed"));
    }
    Ok(())
}

fn collect_live_prs<S: PullRequestStatusSource>(
    snapshot: &WaveSnapshot,
    status_source: &S,
) -> Result<Vec<PullRequestStatus>> {
    let mut statuses = Vec::with_capacity(snapshot.stack.len());
    for node in &snapshot.stack {
        let status = status_source.status(&node.repository, node.pr_number)?;
        verify_pr_identity(node, &status)?;
        if status.state == super::model::PullRequestState::Open && status.merge_state != "CLEAN" {
            return Err(DeliveryError::new(format!(
                "open PR {}#{} merge state is not CLEAN",
                node.repository, node.pr_number
            )));
        }
        verify_required_checks(snapshot, node, &status)?;
        statuses.push(status);
    }
    statuses.sort_by(|left, right| {
        left.repository
            .cmp(&right.repository)
            .then_with(|| left.number.cmp(&right.number))
    });
    Ok(statuses)
}

pub(crate) fn verify_required_checks(
    snapshot: &WaveSnapshot,
    node: &super::model::StackNode,
    status: &PullRequestStatus,
) -> Result<()> {
    let required = snapshot
        .required_checks
        .iter()
        .filter(|check| check.node == node.id)
        .collect::<Vec<_>>();
    if required.is_empty() {
        return Err(DeliveryError::new(format!(
            "stack node {} has no authoritative required checks",
            node.id
        )));
    }
    if status.checks.len() > MAX_CHECKS {
        return Err(DeliveryError::new(
            "live PR check set exceeds the supported bound",
        ));
    }
    validate_bounded_string(&status.merge_state, "PR merge state")?;
    let mut names = BTreeSet::new();
    for check in &status.checks {
        validate_bounded_string(&check.name, "observed check name")?;
        validate_bounded_string(&check.status, "observed check status")?;
        validate_bounded_string(&check.conclusion, "observed check conclusion")?;
        check.publisher.validate()?;
        if !names.insert(check.name.as_str()) {
            return Err(DeliveryError::new(format!(
                "duplicate same-name publisher for check {}",
                check.name
            )));
        }
        if check.commit_oid != node.head_oid {
            return Err(DeliveryError::new(format!(
                "check {} is associated with a different commit",
                check.name
            )));
        }
        let listed = required
            .iter()
            .any(|required| required.name == check.name && required.publisher == check.publisher);
        if !listed && check.state != ObservedCheckState::Successful {
            return Err(DeliveryError::new(format!(
                "unknown or unlisted check {} is failing or pending",
                check.name
            )));
        }
    }
    for required in required {
        let observed = status
            .checks
            .iter()
            .filter(|check| check.name == required.name)
            .collect::<Vec<_>>();
        match observed.as_slice() {
            [check]
                if check.publisher == required.publisher
                    && check.state == ObservedCheckState::Successful
                    && check.commit_oid == node.head_oid => {}
            [] => {
                return Err(DeliveryError::new(format!(
                    "required check {} is missing",
                    required.name
                )));
            }
            [check] if check.publisher != required.publisher => {
                return Err(DeliveryError::new(format!(
                    "required check {} has the wrong app/workflow publisher",
                    required.name
                )));
            }
            [_] => {
                return Err(DeliveryError::new(format!(
                    "required check {} is not successful",
                    required.name
                )));
            }
            _ => {
                return Err(DeliveryError::new(format!(
                    "required check {} is ambiguous",
                    required.name
                )));
            }
        }
    }
    Ok(())
}

fn collect_validation_payloads(context: &SnapshotContext) -> Result<Vec<ValidationPayloadBinding>> {
    let directory = context.layout.evidence_dir();
    ensure_exact_json_set(
        &directory,
        context
            .snapshot
            .required_validations
            .iter()
            .map(|validation| validation.id.as_str()),
        "validation evidence",
    )?;
    let mut payloads = Vec::new();
    for required in &context.snapshot.required_validations {
        let path = directory.join(format!("{}.json", required.id));
        let record = verify_evidence_in_context(context, &path)?;
        if record.result != EvidenceResult::Passed {
            return Err(DeliveryError::new(format!(
                "validation evidence {} is not passed",
                required.id
            )));
        }
        payloads.push(ValidationPayloadBinding {
            id: required.id.clone(),
            sha256: verify_json_digest(&path)?,
            github_attested: matches!(
                record.provenance,
                EvidenceProvenance::GithubAttestation { .. }
            ),
        });
    }
    payloads.sort();
    Ok(payloads)
}

fn collect_panel_payloads(context: &SnapshotContext) -> Result<Vec<PanelPayloadBinding>> {
    let records = read_stored_panel(context)?;
    let mut payloads = Vec::new();
    for record in records {
        if !record.signoff {
            return Err(DeliveryError::new(format!(
                "panel role {} has findings",
                record.role.as_str()
            )));
        }
        let path = context
            .layout
            .panel_dir()
            .join(format!("{}.json", record.role.as_str()));
        payloads.push(PanelPayloadBinding {
            role: record.role,
            sha256: verify_json_digest(&path)?,
        });
    }
    payloads.sort();
    Ok(payloads)
}

fn ensure_exact_json_set<'a>(
    directory: &Path,
    expected: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<()> {
    let expected = expected.map(str::to_owned).collect::<BTreeSet<_>>();
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        DeliveryError::new(format!(
            "cannot inspect {label} directory {}: {error}",
            directory.display()
        ))
    })?;
    if !metadata.file_type().is_dir() {
        return Err(DeliveryError::new(format!(
            "{label} path is not a directory"
        )));
    }
    let mut actual = BTreeSet::new();
    let mut entries = 0_usize;
    for entry in fs::read_dir(directory)? {
        entries += 1;
        if entries > expected.len().saturating_mul(3).saturating_add(10) {
            return Err(DeliveryError::new(format!(
                "{label} directory contains too many entries"
            )));
        }
        if actual.len() > expected.len() {
            return Err(DeliveryError::new(format!(
                "{label} directory contains too many artifacts"
            )));
        }
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        if !entry.file_type()?.is_file() {
            return Err(DeliveryError::new(format!(
                "{label} JSON is not a regular file: {}",
                path.display()
            )));
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| DeliveryError::new(format!("{label} filename is not UTF-8")))?;
        actual.insert(stem.to_owned());
    }
    if actual != expected {
        return Err(DeliveryError::new(format!(
            "{label} set does not exactly match snapshot requirements"
        )));
    }
    Ok(())
}

fn validate_history_proof_values(proof: &HistoryProof) -> Result<()> {
    if proof.artifact_kind != HISTORY_PROOF_ARTIFACT_KIND {
        return Err(DeliveryError::new("invalid history proof artifact_kind"));
    }
    ensure_schema(proof.schema_version, "history proof")?;
    validate_identifier(&proof.program, "program")?;
    validate_identifier(&proof.wave, "wave")?;
    for (value, label) in [
        (&proof.old_candidate_id, "old candidate ID"),
        (&proof.new_candidate_id, "new candidate ID"),
        (&proof.old_content_id, "old content ID"),
        (&proof.new_content_id, "new content ID"),
        (&proof.old_snapshot_sha256, "old snapshot digest"),
        (&proof.new_snapshot_sha256, "new snapshot digest"),
        (&proof.old_seal_sha256, "old seal digest"),
    ] {
        validate_sha256(value, label)?;
    }
    if proof.old_candidate_id == proof.new_candidate_id
        || proof.old_content_id != proof.new_content_id
        || !proof.fresh_ci_required
        || proof.reused_panel_payloads.len() != PANEL_ROLES.len()
    {
        return Err(DeliveryError::new(
            "history proof does not encode distinct history, identical content, reused panel, and fresh CI",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{
        command::{ObservedCheck, ObservedCheckState},
        model::{
            CheckPublisher, CheckPublisherKind, GitObjectFormat, PullRequestState, RequiredCheck,
            StackNode,
        },
    };

    fn check_snapshot() -> (WaveSnapshot, StackNode, PullRequestStatus) {
        let node = StackNode {
            id: "node".to_owned(),
            repository: "github.com/example/d2b".to_owned(),
            pr_number: 1,
            expected_base_ref: "main".to_owned(),
            expected_base_oid: "a".repeat(40),
            head_ref: "feature".to_owned(),
            head_oid: "b".repeat(40),
            head_tree_oid: "c".repeat(40),
            prospective_merge_tree_oid: "c".repeat(40),
            prospective_content_id: "d".repeat(64),
            snapshot_state: PullRequestState::Open,
            depends_on: vec![],
        };
        let publisher = CheckPublisher {
            kind: CheckPublisherKind::CheckRun,
            app_slug: "github-actions".to_owned(),
            app_id: 15368,
            workflow: "Layer 1".to_owned(),
            workflow_id: 321,
        };
        let snapshot = WaveSnapshot {
            artifact_kind: super::super::model::SNAPSHOT_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            program: "adr0045".to_owned(),
            wave: "w1".to_owned(),
            candidate_id: "0".repeat(64),
            content_id: "0".repeat(64),
            authority: super::super::model::AuthorityBinding {
                repository: "github.com/example/d2b".to_owned(),
                ref_name: "feature".to_owned(),
                commit_oid: "b".repeat(40),
                tree_oid: "c".repeat(40),
                manifest_path: "delivery.json".to_owned(),
                manifest_blob_oid: "e".repeat(40),
                manifest_sha256: "f".repeat(64),
            },
            repository_set: vec![super::super::model::RepositoryRecord {
                id: "github.com/example/d2b".to_owned(),
                object_format: GitObjectFormat::Sha1,
                trunk_ref: "main".to_owned(),
                trunk_oid: "a".repeat(40),
                trunk_tree_oid: "a".repeat(40),
                integration_ref: "feature".to_owned(),
                integration_oid: "b".repeat(40),
                integration_tree_oid: "c".repeat(40),
                stack_graph_sha256: "1".repeat(64),
            }],
            stack: vec![node.clone()],
            required_validations: vec![],
            required_checks: vec![RequiredCheck {
                node: "node".to_owned(),
                name: "check".to_owned(),
                publisher: publisher.clone(),
            }],
            generated_artifacts: vec![],
            dependency_fingerprints: vec![],
            contract_fingerprints: vec![],
        };
        let status = PullRequestStatus {
            repository: node.repository.clone(),
            number: 1,
            state: PullRequestState::Open,
            merge_state: "CLEAN".to_owned(),
            base_ref: node.expected_base_ref.clone(),
            base_oid: node.expected_base_oid.clone(),
            head_repository: node.repository.clone(),
            head_ref: node.head_ref.clone(),
            head_oid: node.head_oid.clone(),
            checks: vec![ObservedCheck {
                name: "check".to_owned(),
                publisher,
                status: "COMPLETED".to_owned(),
                conclusion: "SUCCESS".to_owned(),
                state: ObservedCheckState::Successful,
                commit_oid: node.head_oid.clone(),
            }],
        };
        (snapshot, node, status)
    }

    #[test]
    fn wrong_app_and_extra_failed_check_fail_closed() {
        let (snapshot, node, mut status) = check_snapshot();
        status.checks[0].publisher.app_slug = "wrong-app".to_owned();
        let error = verify_required_checks(&snapshot, &node, &status).expect_err("wrong app");
        assert!(error.to_string().contains("wrong app/workflow"));

        let (_, _, mut status) = check_snapshot();
        status.checks.push(super::super::command::ObservedCheck {
            name: "extra".to_owned(),
            publisher: CheckPublisher {
                kind: CheckPublisherKind::CheckRun,
                app_slug: "other".to_owned(),
                app_id: 42,
                workflow: "Other".to_owned(),
                workflow_id: 99,
            },
            status: "COMPLETED".to_owned(),
            conclusion: "FAILURE".to_owned(),
            state: ObservedCheckState::Failed,
            commit_oid: node.head_oid.clone(),
        });
        let error = verify_required_checks(&snapshot, &node, &status).expect_err("extra failed");
        assert!(error.to_string().contains("unknown or unlisted"));
    }
}
