use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{ObservedCheckState, PullRequestStatus, PullRequestStatusSource, RepositoryProbe},
    evidence::{CiAttestationVerifier, EvidenceProvenance, verify_evidence_in_context},
    model::{
        CheckPublisherKind, EvidenceResult, Fingerprint, HISTORY_PROOF_ARTIFACT_KIND, MAX_CHECKS,
        PANEL_ROLES, PanelRole, PullRequestState, RepositoryBinding, RepositoryRecord,
        RequiredCheck, RequiredValidation, SEAL_ARTIFACT_KIND, StackNode, WaveSnapshot,
        ensure_schema, validate_bounded_string, validate_identifier, validate_sha256,
    },
    panel::{PanelReceiptVerifier, read_stored_panel},
    snapshot::{CurrentVerification, SnapshotContext, load_snapshot_context, verify_pr_identity},
    storage::ensure_external_path,
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
    pub receipt_sha256: String,
    pub signature_sha256: String,
    pub trust_root_sha256: String,
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
    pub transitioned_at_unix_seconds: u64,
    pub transition_kind: HistoryTransitionKind,
    pub old_repositories: Vec<RepositoryRecord>,
    pub new_repositories: Vec<RepositoryRecord>,
    pub repository_transitions: Vec<RepositoryTransition>,
    pub old_stack: Vec<StackNode>,
    pub new_stack: Vec<StackNode>,
    pub stack_transitions: Vec<StackTransition>,
    pub unchanged_required_validations: Vec<RequiredValidation>,
    pub unchanged_required_checks: Vec<RequiredCheck>,
    pub unchanged_generated_artifacts: Vec<Fingerprint>,
    pub unchanged_dependency_fingerprints: Vec<Fingerprint>,
    pub unchanged_contract_fingerprints: Vec<Fingerprint>,
    pub reused_panel_payloads: Vec<PanelPayloadBinding>,
    pub fresh_ci_required: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryTransitionKind {
    CommitHistory,
    MergedStackProgression,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryTransition {
    pub repository: String,
    pub old_base_oid: String,
    pub old_base_tree_oid: String,
    pub new_base_oid: String,
    pub new_base_tree_oid: String,
    pub old_base_to_head_diff_sha256: String,
    pub new_base_to_head_diff_sha256: String,
    pub old_generated_diff_sha256: String,
    pub new_generated_diff_sha256: String,
    pub old_dependency_diff_sha256: String,
    pub new_dependency_diff_sha256: String,
    pub old_contract_diff_sha256: String,
    pub new_contract_diff_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackTransition {
    pub node: String,
    pub old_state: PullRequestState,
    pub new_state: PullRequestState,
    pub old_base_ref: String,
    pub old_base_oid: String,
    pub new_base_ref: String,
    pub new_base_oid: String,
    pub old_head_oid: String,
    pub new_head_oid: String,
    pub old_head_tree_oid: String,
    pub new_head_tree_oid: String,
    pub old_merge_commit_oid: Option<String>,
    pub new_merge_commit_oid: Option<String>,
    pub old_merge_commit_tree_oid: Option<String>,
    pub new_merge_commit_tree_oid: Option<String>,
}

pub fn construct_seal<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
) -> Result<PathBuf> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    let validation_payloads = collect_validation_payloads(&context, ci_verifier)?;
    let panel_payloads = collect_panel_payloads(&context, panel_verifier)?;
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
    context.layout.write_candidate_json("seal.json", &seal)?;
    Ok(path)
}

pub fn verify_seal<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
) -> Result<WaveSeal> {
    verify_seal_context(
        probe,
        status_source,
        ci_verifier,
        panel_verifier,
        repository_roots,
        seal_path,
    )
    .map(|(_, seal)| seal)
}

pub(crate) fn verify_seal_context<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
) -> Result<(SnapshotContext, WaveSeal)> {
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
    verify_seal_payloads(&context, &seal, ci_verifier, panel_verifier)?;
    context.layout.verify_candidate_digest("seal.json")?;
    Ok((context, seal))
}

pub(crate) fn verify_seal_recorded<P: RepositoryProbe>(
    probe: &P,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
) -> Result<(SnapshotContext, WaveSeal)> {
    let (context, seal) = load_seal_context(
        probe,
        repository_roots,
        seal_path,
        CurrentVerification::RecordedObjects,
    )?;
    verify_seal_payloads(&context, &seal, ci_verifier, panel_verifier)?;
    context.layout.verify_candidate_digest("seal.json")?;
    Ok((context, seal))
}

pub fn construct_history_proof<P: RepositoryProbe>(
    probe: &P,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
) -> Result<PathBuf> {
    let (old_context, old_seal) = verify_seal_recorded(
        probe,
        ci_verifier,
        panel_verifier,
        repository_roots,
        old_seal_path,
    )?;
    let new_context = load_snapshot_context(
        probe,
        repository_roots,
        new_snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    let transition_kind =
        classify_history_transition(&old_context.snapshot, &new_context.snapshot)?;
    verify_history_git_ancestry(
        probe,
        &old_context.snapshot,
        &new_context.snapshot,
        &new_context.repository_roots,
        transition_kind,
    )?;
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
        old_seal_sha256: old_context.layout.verify_candidate_digest("seal.json")?,
        transitioned_at_unix_seconds: now_unix_seconds()?,
        transition_kind,
        old_repositories: old_context.snapshot.repository_set.clone(),
        new_repositories: new_context.snapshot.repository_set.clone(),
        repository_transitions: repository_transitions(
            &old_context.snapshot,
            &new_context.snapshot,
        )?,
        old_stack: old_context.snapshot.stack.clone(),
        new_stack: new_context.snapshot.stack.clone(),
        stack_transitions: stack_transitions(&old_context.snapshot, &new_context.snapshot)?,
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
    new_context
        .layout
        .write_candidate_json("history-proof.json", &proof)?;
    Ok(path)
}

pub fn verify_history_proof<P: RepositoryProbe>(
    probe: &P,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
) -> Result<(WaveSnapshot, WaveSeal, HistoryProof)> {
    let (context, seal, proof) = verify_history_proof_context(
        probe,
        ci_verifier,
        panel_verifier,
        repository_roots,
        old_seal_path,
        new_snapshot_path,
        proof_path,
    )?;
    Ok((context.snapshot, seal, proof))
}

pub(crate) fn verify_history_proof_context<P: RepositoryProbe>(
    probe: &P,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
) -> Result<(SnapshotContext, WaveSeal, HistoryProof)> {
    let (old_context, old_seal) = verify_seal_recorded(
        probe,
        ci_verifier,
        panel_verifier,
        repository_roots,
        old_seal_path,
    )?;
    let new_context = load_snapshot_context(
        probe,
        repository_roots,
        new_snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    let (proof, _proof_digest): (HistoryProof, String) = new_context
        .layout
        .read_candidate_json("history-proof.json")?;
    validate_history_proof_values(&proof)?;
    let transition_kind =
        classify_history_transition(&old_context.snapshot, &new_context.snapshot)?;
    verify_history_git_ancestry(
        probe,
        &old_context.snapshot,
        &new_context.snapshot,
        &new_context.repository_roots,
        transition_kind,
    )?;
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
        || proof.old_seal_sha256 != old_context.layout.verify_candidate_digest("seal.json")?
        || proof.transition_kind != transition_kind
        || proof.old_repositories != old_context.snapshot.repository_set
        || proof.new_repositories != new_context.snapshot.repository_set
        || proof.repository_transitions
            != repository_transitions(&old_context.snapshot, &new_context.snapshot)?
        || proof.old_stack != old_context.snapshot.stack
        || proof.new_stack != new_context.snapshot.stack
        || proof.stack_transitions
            != stack_transitions(&old_context.snapshot, &new_context.snapshot)?
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
    classify_history_transition(sealed, candidate).map(|_| ())
}

fn classify_history_transition(
    sealed: &WaveSnapshot,
    candidate: &WaveSnapshot,
) -> Result<HistoryTransitionKind> {
    sealed.validate()?;
    candidate.validate()?;
    if sealed.program != candidate.program
        || sealed.wave != candidate.wave
        || sealed.content_id != candidate.content_id
        || sealed.panel_trust_root_sha256 != candidate.panel_trust_root_sha256
        || sealed.authority.repository != candidate.authority.repository
        || sealed.authority.ref_name != candidate.authority.ref_name
        || sealed.authority.tree_oid != candidate.authority.tree_oid
        || sealed.authority.manifest_path != candidate.authority.manifest_path
        || sealed.authority.manifest_blob_oid != candidate.authority.manifest_blob_oid
        || sealed.authority.manifest_sha256 != candidate.authority.manifest_sha256
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
    let old_repositories = sealed
        .repository_set
        .iter()
        .map(|repository| (repository.id.as_str(), repository))
        .collect::<BTreeMap<_, _>>();
    let new_repositories = candidate
        .repository_set
        .iter()
        .map(|repository| (repository.id.as_str(), repository))
        .collect::<BTreeMap<_, _>>();
    if old_repositories.keys().collect::<Vec<_>>() != new_repositories.keys().collect::<Vec<_>>() {
        return Err(DeliveryError::new(
            "candidate repository set changed from the sealed candidate",
        ));
    }
    for (id, old) in &old_repositories {
        let new = new_repositories
            .get(id)
            .copied()
            .expect("repository sets were compared");
        if old.object_format != new.object_format
            || old.trunk_ref != new.trunk_ref
            || old.integration_ref != new.integration_ref
            || old.integration_tree_oid != new.integration_tree_oid
        {
            return Err(DeliveryError::new(format!(
                "repository {id} final tree or repository policy changed"
            )));
        }
    }

    let bases_unchanged = old_repositories.iter().all(|(id, old)| {
        let new = new_repositories
            .get(id)
            .copied()
            .expect("repository sets were compared");
        old.trunk_oid == new.trunk_oid
            && old.trunk_tree_oid == new.trunk_tree_oid
            && old.base_to_head_diff_sha256 == new.base_to_head_diff_sha256
            && old.generated_diff_sha256 == new.generated_diff_sha256
            && old.dependency_diff_sha256 == new.dependency_diff_sha256
            && old.contract_diff_sha256 == new.contract_diff_sha256
    });
    if bases_unchanged {
        verify_commit_history_progression(sealed, candidate)?;
        return Ok(HistoryTransitionKind::CommitHistory);
    }

    verify_merged_progression(sealed, candidate)?;
    Ok(HistoryTransitionKind::MergedStackProgression)
}

fn verify_commit_history_progression(
    sealed: &WaveSnapshot,
    candidate: &WaveSnapshot,
) -> Result<()> {
    let old_nodes = sealed
        .stack
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let new_nodes = candidate
        .stack
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    if old_nodes.keys().collect::<Vec<_>>() != new_nodes.keys().collect::<Vec<_>>() {
        return Err(DeliveryError::new(
            "commit-history transition changed the configured stack node set",
        ));
    }
    for (id, old) in old_nodes {
        let new = new_nodes
            .get(id)
            .copied()
            .expect("stack node sets were compared");
        if old.repository != new.repository
            || old.pr_number != new.pr_number
            || old.expected_base_ref != new.expected_base_ref
            || old.head_ref != new.head_ref
            || old.head_tree_oid != new.head_tree_oid
            || old.merge_commit_oid != new.merge_commit_oid
            || old.merge_commit_tree_oid != new.merge_commit_tree_oid
            || old.prospective_merge_tree_oid != new.prospective_merge_tree_oid
            || old.snapshot_state != new.snapshot_state
            || old.depends_on != new.depends_on
        {
            return Err(DeliveryError::new(format!(
                "commit-history transition changed node {id} content, state, or policy"
            )));
        }
    }
    Ok(())
}

fn verify_merged_progression(sealed: &WaveSnapshot, candidate: &WaveSnapshot) -> Result<()> {
    let old_nodes = sealed
        .stack
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let new_nodes = candidate
        .stack
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    if old_nodes.keys().collect::<Vec<_>>() != new_nodes.keys().collect::<Vec<_>>() {
        return Err(DeliveryError::new(
            "merged progression changed the configured stack node set",
        ));
    }
    let mut transitioned = BTreeSet::new();
    for (id, old) in &old_nodes {
        let new = new_nodes
            .get(id)
            .copied()
            .expect("stack node sets were compared");
        if old.repository != new.repository
            || old.pr_number != new.pr_number
            || old.head_ref != new.head_ref
            || old.head_tree_oid != new.head_tree_oid
            || old.depends_on != new.depends_on
        {
            return Err(DeliveryError::new(format!(
                "merged progression changed node {id} identity or content"
            )));
        }
        match (old.snapshot_state, new.snapshot_state) {
            (PullRequestState::Open, PullRequestState::Merged) => {
                if old.head_oid != new.head_oid
                    || new.merge_commit_tree_oid.as_deref()
                        != Some(old.prospective_merge_tree_oid.as_str())
                {
                    return Err(DeliveryError::new(format!(
                        "merged node {id} changed head or has the wrong merge tree while transitioning"
                    )));
                }
                transitioned.insert(*id);
            }
            (left, right) if left == right => {
                if old.merge_commit_oid != new.merge_commit_oid
                    || old.merge_commit_tree_oid != new.merge_commit_tree_oid
                {
                    return Err(DeliveryError::new(format!(
                        "node {id} changed merge commit authority without transitioning"
                    )));
                }
            }
            _ => {
                return Err(DeliveryError::new(format!(
                    "node {id} has an unsupported history transition"
                )));
            }
        }
    }
    if transitioned.is_empty() {
        return Err(DeliveryError::new(
            "base movement is not explained by an open-to-merged stack transition",
        ));
    }
    for old_repository in &sealed.repository_set {
        let new_repository = candidate
            .repository_set
            .iter()
            .find(|repository| repository.id == old_repository.id)
            .expect("repository sets were compared");
        let old_open = sealed
            .stack
            .iter()
            .filter(|node| {
                node.repository == old_repository.id
                    && node.snapshot_state == PullRequestState::Open
            })
            .collect::<Vec<_>>();
        let transitioned_prefix = old_open
            .iter()
            .take_while(|node| transitioned.contains(node.id.as_str()))
            .count();
        if old_open
            .iter()
            .skip(transitioned_prefix)
            .any(|node| transitioned.contains(node.id.as_str()))
        {
            return Err(DeliveryError::new(format!(
                "repository {} merged nodes are not a contiguous stack prefix",
                old_repository.id
            )));
        }
        if old_repository.trunk_oid == new_repository.trunk_oid {
            if transitioned_prefix != 0 {
                return Err(DeliveryError::new(format!(
                    "repository {} reports merged nodes without advancing its base",
                    old_repository.id
                )));
            }
            continue;
        }
        if transitioned_prefix == 0
            || old_open[transitioned_prefix - 1].prospective_merge_tree_oid
                != new_repository.trunk_tree_oid
        {
            return Err(DeliveryError::new(format!(
                "repository {} base move is not the sealed prospective tree of its merged prefix",
                old_repository.id
            )));
        }
    }
    for (id, old) in &old_nodes {
        let new = new_nodes
            .get(id)
            .copied()
            .expect("stack node sets were compared");
        if old.snapshot_state == PullRequestState::Open
            && new.snapshot_state == PullRequestState::Open
            && (old.expected_base_oid != new.expected_base_oid
                || old.expected_base_ref != new.expected_base_ref)
        {
            let retargeted_from_merged = old
                .depends_on
                .iter()
                .filter(|dependency| transitioned.contains(dependency.as_str()))
                .any(|dependency| old.expected_base_oid == old_nodes[dependency.as_str()].head_oid);
            let repository = candidate
                .repository_set
                .iter()
                .find(|repository| repository.id == new.repository)
                .expect("node repository is present");
            if !retargeted_from_merged
                || new.expected_base_ref != repository.trunk_ref
                || new.expected_base_oid != repository.trunk_oid
            {
                return Err(DeliveryError::new(format!(
                    "active node {id} retarget is not explained by its merged predecessor"
                )));
            }
        }
    }
    Ok(())
}

fn verify_history_git_ancestry<P: RepositoryProbe>(
    probe: &P,
    sealed: &WaveSnapshot,
    candidate: &WaveSnapshot,
    roots: &BTreeMap<String, PathBuf>,
    transition: HistoryTransitionKind,
) -> Result<()> {
    if transition != HistoryTransitionKind::MergedStackProgression {
        return Ok(());
    }
    for old_repository in &sealed.repository_set {
        let new_repository = candidate
            .repository_set
            .iter()
            .find(|repository| repository.id == old_repository.id)
            .ok_or_else(|| DeliveryError::new("history repository set is incomplete"))?;
        if old_repository.trunk_oid == new_repository.trunk_oid {
            continue;
        }
        let root = roots
            .get(&old_repository.id)
            .ok_or_else(|| DeliveryError::new("history repository checkout is missing"))?;
        if !probe.is_ancestor(root, &old_repository.trunk_oid, &new_repository.trunk_oid)? {
            return Err(DeliveryError::new(format!(
                "repository {} new base does not descend from the sealed base",
                old_repository.id
            )));
        }
        for old_node in sealed.stack.iter().filter(|node| {
            node.repository == old_repository.id
                && node.snapshot_state == PullRequestState::Open
                && candidate.stack.iter().any(|new_node| {
                    new_node.id == node.id && new_node.snapshot_state == PullRequestState::Merged
                })
        }) {
            let new_node = candidate
                .stack
                .iter()
                .find(|node| node.id == old_node.id)
                .expect("transitioned node is present");
            let merge_commit = new_node.merge_commit_oid.as_ref().ok_or_else(|| {
                DeliveryError::new(format!(
                    "merged stack node {} has no exact merge commit",
                    old_node.id
                ))
            })?;
            if !probe.is_ancestor(root, &old_repository.trunk_oid, merge_commit)?
                || !probe.is_ancestor(root, merge_commit, &new_repository.trunk_oid)?
            {
                return Err(DeliveryError::new(format!(
                    "merged stack node {} merge commit is outside the base progression",
                    old_node.id
                )));
            }
        }
    }
    Ok(())
}

fn repository_transitions(
    sealed: &WaveSnapshot,
    candidate: &WaveSnapshot,
) -> Result<Vec<RepositoryTransition>> {
    let mut transitions = Vec::with_capacity(sealed.repository_set.len());
    for old in &sealed.repository_set {
        let new = candidate
            .repository_set
            .iter()
            .find(|repository| repository.id == old.id)
            .ok_or_else(|| DeliveryError::new("history repository transition is incomplete"))?;
        transitions.push(RepositoryTransition {
            repository: old.id.clone(),
            old_base_oid: old.trunk_oid.clone(),
            old_base_tree_oid: old.trunk_tree_oid.clone(),
            new_base_oid: new.trunk_oid.clone(),
            new_base_tree_oid: new.trunk_tree_oid.clone(),
            old_base_to_head_diff_sha256: old.base_to_head_diff_sha256.clone(),
            new_base_to_head_diff_sha256: new.base_to_head_diff_sha256.clone(),
            old_generated_diff_sha256: old.generated_diff_sha256.clone(),
            new_generated_diff_sha256: new.generated_diff_sha256.clone(),
            old_dependency_diff_sha256: old.dependency_diff_sha256.clone(),
            new_dependency_diff_sha256: new.dependency_diff_sha256.clone(),
            old_contract_diff_sha256: old.contract_diff_sha256.clone(),
            new_contract_diff_sha256: new.contract_diff_sha256.clone(),
        });
    }
    Ok(transitions)
}

fn stack_transitions(
    sealed: &WaveSnapshot,
    candidate: &WaveSnapshot,
) -> Result<Vec<StackTransition>> {
    let mut transitions = Vec::with_capacity(sealed.stack.len());
    for old in &sealed.stack {
        let new = candidate
            .stack
            .iter()
            .find(|node| node.id == old.id)
            .ok_or_else(|| DeliveryError::new("history stack transition is incomplete"))?;
        transitions.push(StackTransition {
            node: old.id.clone(),
            old_state: old.snapshot_state,
            new_state: new.snapshot_state,
            old_base_ref: old.expected_base_ref.clone(),
            old_base_oid: old.expected_base_oid.clone(),
            new_base_ref: new.expected_base_ref.clone(),
            new_base_oid: new.expected_base_oid.clone(),
            old_head_oid: old.head_oid.clone(),
            new_head_oid: new.head_oid.clone(),
            old_head_tree_oid: old.head_tree_oid.clone(),
            new_head_tree_oid: new.head_tree_oid.clone(),
            old_merge_commit_oid: old.merge_commit_oid.clone(),
            new_merge_commit_oid: new.merge_commit_oid.clone(),
            old_merge_commit_tree_oid: old.merge_commit_tree_oid.clone(),
            new_merge_commit_tree_oid: new.merge_commit_tree_oid.clone(),
        });
    }
    Ok(transitions)
}

fn now_unix_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| DeliveryError::new("system time is before the Unix epoch"))
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
    let (seal, _seal_digest): (WaveSeal, String) =
        context.layout.read_candidate_json("seal.json")?;
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
        validate_sha256(&payload.receipt_sha256, "panel receipt digest")?;
        validate_sha256(&payload.signature_sha256, "panel signature digest")?;
        validate_sha256(&payload.trust_root_sha256, "panel trust-root digest")?;
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

fn verify_seal_payloads(
    context: &SnapshotContext,
    seal: &WaveSeal,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
) -> Result<()> {
    let validations = collect_validation_payloads(context, ci_verifier)?;
    if validations != seal.validation_payloads {
        return Err(DeliveryError::new(
            "seal validation payload bindings changed",
        ));
    }
    let panels = collect_panel_payloads(context, panel_verifier)?;
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
        if status.state == PullRequestState::Open
            && (status.is_in_merge_queue
                || status.is_merge_queue_enabled
                || status.merge_queue_entry.is_some())
        {
            return Err(DeliveryError::new(format!(
                "open PR {}#{} uses a merge queue without exact merge-group authority",
                node.repository, node.pr_number
            )));
        }
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
        validate_observed_check_authority(check)?;
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
        if !listed
            && check.state != ObservedCheckState::Successful
            && check.state != ObservedCheckState::Skipped
        {
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
            // A required check that GitHub reports as SKIPPED is never
            // treated as satisfied: only unlisted (optional) skipped checks
            // are accepted. Report this distinctly from a generic
            // not-successful failure so operators can tell a skipped
            // required gate apart from a genuinely failing one.
            [check] if check.state == ObservedCheckState::Skipped => {
                return Err(DeliveryError::new(format!(
                    "required check {} is skipped",
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

fn validate_observed_check_authority(check: &super::command::ObservedCheck) -> Result<()> {
    match check.publisher.kind {
        CheckPublisherKind::CheckRun => {
            let Some(check_run_id) = check.check_run_id else {
                return Err(DeliveryError::new(format!(
                    "check run {} has no database ID",
                    check.name
                )));
            };
            let Some(completed) = check.completed_at_unix_seconds else {
                return Err(DeliveryError::new(format!(
                    "successful check run {} has no completion timestamp",
                    check.name
                )));
            };
            if check_run_id == 0
                || check.started_at_unix_seconds == 0
                || completed < check.started_at_unix_seconds
            {
                return Err(DeliveryError::new(format!(
                    "check run {} has invalid run identity or timestamps",
                    check.name
                )));
            }
            match (
                check.publisher.workflow_id,
                check.workflow_run_id,
                check.workflow_created_at_unix_seconds,
                check.workflow_updated_at_unix_seconds,
            ) {
                (0, None, None, None) => {}
                (workflow_id, Some(run_id), Some(created), Some(updated))
                    if workflow_id != 0 && run_id != 0 && created != 0 && updated >= created => {}
                _ => {
                    return Err(DeliveryError::new(format!(
                        "check run {} has incomplete workflow-run authority",
                        check.name
                    )));
                }
            }
        }
        CheckPublisherKind::StatusContext => {
            if check.check_run_id.is_some()
                || check.workflow_run_id.is_some()
                || check.workflow_created_at_unix_seconds.is_some()
                || check.workflow_updated_at_unix_seconds.is_some()
                || check.started_at_unix_seconds == 0
                || check.completed_at_unix_seconds != Some(check.started_at_unix_seconds)
            {
                return Err(DeliveryError::new(format!(
                    "status context {} has invalid publisher authority",
                    check.name
                )));
            }
        }
    }
    Ok(())
}

fn collect_validation_payloads(
    context: &SnapshotContext,
    verifier: &dyn CiAttestationVerifier,
) -> Result<Vec<ValidationPayloadBinding>> {
    ensure_exact_candidate_json_set(
        context,
        Path::new("validation"),
        context
            .snapshot
            .required_validations
            .iter()
            .map(|validation| validation.id.as_str()),
        "validation evidence",
    )?;
    let mut payloads = Vec::new();
    for required in &context.snapshot.required_validations {
        let path = context
            .layout
            .evidence_dir()
            .join(format!("{}.json", required.id));
        let record = verify_evidence_in_context(context, &path, verifier)?;
        if record.result != EvidenceResult::Passed {
            return Err(DeliveryError::new(format!(
                "validation evidence {} is not passed",
                required.id
            )));
        }
        payloads.push(ValidationPayloadBinding {
            id: required.id.clone(),
            sha256: context.layout.verify_candidate_digest(
                Path::new("validation").join(format!("{}.json", required.id)),
            )?,
            github_attested: matches!(
                record.provenance,
                EvidenceProvenance::GithubAttestation { .. }
            ),
        });
    }
    payloads.sort();
    Ok(payloads)
}

fn collect_panel_payloads(
    context: &SnapshotContext,
    verifier: &dyn PanelReceiptVerifier,
) -> Result<Vec<PanelPayloadBinding>> {
    ensure_exact_candidate_json_set(
        context,
        Path::new("panel"),
        PANEL_ROLES.iter().map(|role| role.as_str()),
        "panel evidence",
    )?;
    let records = read_stored_panel(context, verifier)?;
    let mut payloads = Vec::new();
    for record in records {
        if !record.claims.signoff {
            return Err(DeliveryError::new(format!(
                "panel role {} has findings",
                record.claims.role.as_str()
            )));
        }
        payloads.push(PanelPayloadBinding {
            role: record.claims.role,
            receipt_sha256: record.receipt_sha256,
            signature_sha256: record.signature_sha256,
            trust_root_sha256: record.trust_root_sha256,
        });
    }
    payloads.sort();
    Ok(payloads)
}

fn ensure_exact_candidate_json_set<'a>(
    context: &SnapshotContext,
    directory: &Path,
    expected: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<()> {
    let expected = expected.map(str::to_owned).collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    let mut entries = 0_usize;
    for name in context.layout.list_candidate_directory(directory)? {
        entries += 1;
        if entries > expected.len().saturating_mul(4).saturating_add(16) {
            return Err(DeliveryError::new(format!(
                "{label} directory contains too many entries"
            )));
        }
        if actual.len() > expected.len() {
            return Err(DeliveryError::new(format!(
                "{label} directory contains too many artifacts"
            )));
        }
        let path = Path::new(&name);
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
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
        || proof.transitioned_at_unix_seconds == 0
        || proof.transitioned_at_unix_seconds > now_unix_seconds()?.saturating_add(300)
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
            observed_base_oid: "a".repeat(40),
            head_ref: "feature".to_owned(),
            head_oid: "b".repeat(40),
            head_tree_oid: "c".repeat(40),
            merge_commit_oid: None,
            merge_commit_tree_oid: None,
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
            panel_trust_root_sha256: "a".repeat(64),
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
                base_to_head_diff_sha256: "1".repeat(64),
                generated_diff_sha256: "2".repeat(64),
                dependency_diff_sha256: "3".repeat(64),
                contract_diff_sha256: "4".repeat(64),
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
            merge_commit_oid: None,
            merge_commit_tree_oid: None,
            merge_base_oid: None,
            is_in_merge_queue: false,
            is_merge_queue_enabled: false,
            merge_queue_entry: None,
            checks: vec![ObservedCheck {
                name: "check".to_owned(),
                publisher,
                check_run_id: Some(1),
                workflow_run_id: Some(2),
                status: "COMPLETED".to_owned(),
                conclusion: "SUCCESS".to_owned(),
                state: ObservedCheckState::Successful,
                commit_oid: node.head_oid.clone(),
                started_at_unix_seconds: 1,
                completed_at_unix_seconds: Some(2),
                workflow_created_at_unix_seconds: Some(1),
                workflow_updated_at_unix_seconds: Some(2),
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
            check_run_id: Some(3),
            workflow_run_id: Some(4),
            status: "COMPLETED".to_owned(),
            conclusion: "FAILURE".to_owned(),
            state: ObservedCheckState::Failed,
            commit_oid: node.head_oid.clone(),
            started_at_unix_seconds: 1,
            completed_at_unix_seconds: Some(2),
            workflow_created_at_unix_seconds: Some(1),
            workflow_updated_at_unix_seconds: Some(2),
        });
        let error = verify_required_checks(&snapshot, &node, &status).expect_err("extra failed");
        assert!(error.to_string().contains("unknown or unlisted"));
    }

    /// An unlisted (optional) check that GitHub reports SKIPPED — for example
    /// WeezTerm's optional `release`/`dev-release` jobs — must not block
    /// snapshot/seal: it is retained in the evidence but does not fail
    /// `verify_required_checks`.
    #[test]
    fn unlisted_skipped_check_is_accepted() {
        let (snapshot, node, mut status) = check_snapshot();
        status.checks.push(super::super::command::ObservedCheck {
            name: "release".to_owned(),
            publisher: CheckPublisher {
                kind: CheckPublisherKind::CheckRun,
                app_slug: "other".to_owned(),
                app_id: 42,
                workflow: "Other".to_owned(),
                workflow_id: 99,
            },
            check_run_id: Some(5),
            workflow_run_id: Some(6),
            status: "COMPLETED".to_owned(),
            conclusion: "SKIPPED".to_owned(),
            state: ObservedCheckState::Skipped,
            commit_oid: node.head_oid.clone(),
            started_at_unix_seconds: 1,
            completed_at_unix_seconds: Some(2),
            workflow_created_at_unix_seconds: Some(1),
            workflow_updated_at_unix_seconds: Some(2),
        });
        verify_required_checks(&snapshot, &node, &status)
            .expect("unlisted skipped check must not fail closed");
        // The skipped check is still present (retained) in the evidence.
        assert!(
            status
                .checks
                .iter()
                .any(|check| check.name == "release" && check.state == ObservedCheckState::Skipped)
        );
    }

    /// Unlisted checks that are pending, failing, neutral, or cancelled must
    /// still fail closed; only SKIPPED gets the lenient optional-check
    /// treatment.
    #[test]
    fn unlisted_non_skipped_non_success_checks_still_fail_closed() {
        for conclusion in ["NEUTRAL", "CANCELLED"] {
            let (snapshot, node, mut status) = check_snapshot();
            status.checks.push(super::super::command::ObservedCheck {
                name: "extra".to_owned(),
                publisher: CheckPublisher {
                    kind: CheckPublisherKind::CheckRun,
                    app_slug: "other".to_owned(),
                    app_id: 42,
                    workflow: "Other".to_owned(),
                    workflow_id: 99,
                },
                check_run_id: Some(3),
                workflow_run_id: Some(4),
                status: "COMPLETED".to_owned(),
                conclusion: conclusion.to_owned(),
                state: ObservedCheckState::Failed,
                commit_oid: node.head_oid.clone(),
                started_at_unix_seconds: 1,
                completed_at_unix_seconds: Some(2),
                workflow_created_at_unix_seconds: Some(1),
                workflow_updated_at_unix_seconds: Some(2),
            });
            let error = verify_required_checks(&snapshot, &node, &status)
                .expect_err(&format!("unlisted {conclusion} must fail closed"));
            assert!(error.to_string().contains("unknown or unlisted"));
        }

        // An unlisted pending (not yet completed) check must also fail closed.
        let (snapshot, node, mut status) = check_snapshot();
        status.checks.push(super::super::command::ObservedCheck {
            name: "extra-pending".to_owned(),
            publisher: CheckPublisher {
                kind: CheckPublisherKind::CheckRun,
                app_slug: "other".to_owned(),
                app_id: 42,
                workflow: "Other".to_owned(),
                workflow_id: 99,
            },
            check_run_id: Some(7),
            workflow_run_id: Some(8),
            status: "IN_PROGRESS".to_owned(),
            conclusion: "NONE".to_owned(),
            state: ObservedCheckState::Pending,
            commit_oid: node.head_oid.clone(),
            started_at_unix_seconds: 1,
            // `verify_required_checks` alone (not the full GitHub parser) is
            // under test here, so a completion timestamp is supplied to
            // isolate the pending-state invariant from the unrelated
            // check-run-authority timestamp requirement exercised elsewhere.
            completed_at_unix_seconds: Some(2),
            workflow_created_at_unix_seconds: Some(1),
            workflow_updated_at_unix_seconds: Some(2),
        });
        let error =
            verify_required_checks(&snapshot, &node, &status).expect_err("unlisted pending");
        assert!(error.to_string().contains("unknown or unlisted"));
    }

    /// A *required* check that GitHub reports SKIPPED must still fail: only
    /// unlisted/optional skipped checks are accepted.
    #[test]
    fn required_check_skipped_still_fails_closed() {
        let (snapshot, node, mut status) = check_snapshot();
        status.checks[0].status = "COMPLETED".to_owned();
        status.checks[0].conclusion = "SKIPPED".to_owned();
        status.checks[0].state = ObservedCheckState::Skipped;
        let error =
            verify_required_checks(&snapshot, &node, &status).expect_err("required skipped");
        assert!(error.to_string().contains("is skipped"));
    }
}
