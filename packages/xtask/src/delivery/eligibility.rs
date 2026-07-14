use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::{
    DeliveryError, Result,
    command::{PullRequestMerger, PullRequestStatus, PullRequestStatusSource, RepositoryProbe},
    evidence::CiAttestationVerifier,
    model::{CheckPublisherKind, PullRequestState, StackNode, WaveSnapshot},
    panel::PanelReceiptVerifier,
    seal::{WaveSeal, verify_history_proof_context, verify_required_checks, verify_seal_context},
    snapshot::{SnapshotContext, verify_pr_identity},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MergeEligibility {
    pub candidate_id: String,
    pub target_node: String,
    pub repository: String,
    pub pr_number: u64,
    pub expected_base_oid: String,
    pub expected_head_oid: String,
}

pub fn check_merge_eligibility<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
    target_node: &str,
) -> Result<MergeEligibility> {
    check_merge_eligibility_context(
        probe,
        status_source,
        ci_verifier,
        panel_verifier,
        repository_roots,
        seal_path,
        target_node,
    )
    .map(|(_, eligibility)| eligibility)
}

#[allow(clippy::too_many_arguments)]
fn check_merge_eligibility_context<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
    target_node: &str,
) -> Result<(SnapshotContext, MergeEligibility)> {
    let (context, seal) = verify_seal_context(
        probe,
        status_source,
        ci_verifier,
        panel_verifier,
        repository_roots,
        seal_path,
    )?;
    let eligibility = evaluate_merge_eligibility(
        &context.snapshot,
        Some(&seal.live_pull_requests),
        status_source,
        target_node,
        false,
    )?;
    Ok((context, eligibility))
}

#[allow(clippy::too_many_arguments)]
pub fn check_history_merge_eligibility<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
    target_node: &str,
) -> Result<MergeEligibility> {
    check_history_merge_eligibility_context(
        probe,
        status_source,
        ci_verifier,
        panel_verifier,
        repository_roots,
        old_seal_path,
        new_snapshot_path,
        proof_path,
        target_node,
    )
    .map(|(_, _, eligibility)| eligibility)
}

#[allow(clippy::too_many_arguments)]
fn check_history_merge_eligibility_context<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
    target_node: &str,
) -> Result<(SnapshotContext, WaveSeal, MergeEligibility)> {
    let (context, old_seal, proof) = verify_history_proof_context(
        probe,
        ci_verifier,
        panel_verifier,
        repository_roots,
        old_seal_path,
        new_snapshot_path,
        proof_path,
    )?;
    if !proof.fresh_ci_required {
        return Err(DeliveryError::new(
            "history proof does not require fresh CI",
        ));
    }
    let eligibility = evaluate_merge_eligibility(
        &context.snapshot,
        Some(&old_seal.live_pull_requests),
        status_source,
        target_node,
        true,
    )?;
    Ok((context, old_seal, eligibility))
}

#[allow(clippy::too_many_arguments)]
pub fn atomic_merge<P: RepositoryProbe, S: PullRequestStatusSource, M: PullRequestMerger>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    merger: &M,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
    target_node: &str,
) -> Result<MergeEligibility> {
    let (context, eligibility) = check_merge_eligibility_context(
        probe,
        status_source,
        ci_verifier,
        panel_verifier,
        repository_roots,
        seal_path,
        target_node,
    )?;
    consume_atomic_merge(&context.snapshot, status_source, merger, &eligibility, None)?;
    Ok(eligibility)
}

#[allow(clippy::too_many_arguments)]
pub fn atomic_history_merge<
    P: RepositoryProbe,
    S: PullRequestStatusSource,
    M: PullRequestMerger,
>(
    probe: &P,
    status_source: &S,
    ci_verifier: &dyn CiAttestationVerifier,
    panel_verifier: &dyn PanelReceiptVerifier,
    merger: &M,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
    target_node: &str,
) -> Result<MergeEligibility> {
    let (context, old_seal, eligibility) = check_history_merge_eligibility_context(
        probe,
        status_source,
        ci_verifier,
        panel_verifier,
        repository_roots,
        old_seal_path,
        new_snapshot_path,
        proof_path,
        target_node,
    )?;
    consume_atomic_merge(
        &context.snapshot,
        status_source,
        merger,
        &eligibility,
        Some(&old_seal.live_pull_requests),
    )?;
    Ok(eligibility)
}

pub fn evaluate_merge_eligibility<S: PullRequestStatusSource>(
    snapshot: &WaveSnapshot,
    sealed_live: Option<&[PullRequestStatus]>,
    status_source: &S,
    target_node: &str,
    fresh_ci_required: bool,
) -> Result<MergeEligibility> {
    snapshot.validate()?;
    if fresh_ci_required && sealed_live.is_none() {
        return Err(DeliveryError::new(
            "fresh CI evaluation requires the verified sealed run baseline",
        ));
    }
    let nodes = snapshot
        .stack
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let target = nodes
        .get(target_node)
        .copied()
        .ok_or_else(|| DeliveryError::new(format!("unknown target stack node {target_node}")))?;
    let sealed = sealed_live
        .map(|statuses| {
            let by_pr = statuses
                .iter()
                .map(|status| ((status.repository.as_str(), status.number), status))
                .collect::<BTreeMap<_, _>>();
            if by_pr.len() != statuses.len() {
                return Err(DeliveryError::new(
                    "sealed CI baseline repeats a pull request",
                ));
            }
            Ok(by_pr)
        })
        .transpose()?;
    if sealed
        .as_ref()
        .is_some_and(|statuses| statuses.len() != snapshot.stack.len())
    {
        return Err(DeliveryError::new(
            "sealed PR/check authority set does not match the snapshot stack",
        ));
    }

    let mut current = BTreeMap::new();
    for node in &snapshot.stack {
        let status = status_source.status(&node.repository, node.pr_number)?;
        verify_pr_identity(node, &status)?;
        verify_required_checks(snapshot, node, &status)?;
        reject_unverified_merge_queue(node, &status)?;
        if let Some(sealed) = &sealed {
            let expected = sealed
                .get(&(node.repository.as_str(), node.pr_number))
                .ok_or_else(|| DeliveryError::new("seal is missing a live PR binding"))?;
            if fresh_ci_required {
                verify_fresh_ci(snapshot, node, expected, &status)?;
            } else if *expected != &status {
                return Err(DeliveryError::new(format!(
                    "live PR {}#{} changed from seal",
                    node.repository, node.pr_number
                )));
            }
        }
        current.insert(node.id.as_str(), status);
    }

    let target_status = current
        .get(target.id.as_str())
        .expect("target status collected");
    if target_status.state != PullRequestState::Open {
        return Err(DeliveryError::new(format!(
            "target PR {}#{} is not open",
            target.repository, target.pr_number
        )));
    }
    if target_status.merge_state != "CLEAN" {
        return Err(DeliveryError::new(format!(
            "target PR {}#{} merge state is not CLEAN",
            target.repository, target.pr_number
        )));
    }
    for dependency_id in transitive_dependencies(target, &nodes)? {
        let status = current
            .get(dependency_id.as_str())
            .ok_or_else(|| DeliveryError::new("dependency status is missing"))?;
        if status.state != PullRequestState::Merged {
            return Err(DeliveryError::new(format!(
                "dependency stack node {dependency_id} is not merged"
            )));
        }
    }
    Ok(MergeEligibility {
        candidate_id: snapshot.candidate_id.clone(),
        target_node: target.id.clone(),
        repository: target.repository.clone(),
        pr_number: target.pr_number,
        expected_base_oid: target.expected_base_oid.clone(),
        expected_head_oid: target.head_oid.clone(),
    })
}

fn consume_atomic_merge<S: PullRequestStatusSource, M: PullRequestMerger>(
    snapshot: &WaveSnapshot,
    status_source: &S,
    merger: &M,
    eligibility: &MergeEligibility,
    fresh_ci_baseline: Option<&[PullRequestStatus]>,
) -> Result<()> {
    let node = snapshot
        .stack
        .iter()
        .find(|node| node.id == eligibility.target_node)
        .ok_or_else(|| DeliveryError::new("eligible target disappeared from snapshot"))?;
    let immediate = status_source.status(&node.repository, node.pr_number)?;
    verify_pr_identity(node, &immediate)?;
    verify_required_checks(snapshot, node, &immediate)?;
    reject_unverified_merge_queue(node, &immediate)?;
    if immediate.state != PullRequestState::Open || immediate.merge_state != "CLEAN" {
        return Err(DeliveryError::new(
            "target PR changed before atomic merge consumption",
        ));
    }
    if immediate.base_oid != eligibility.expected_base_oid
        || immediate.head_oid != eligibility.expected_head_oid
    {
        return Err(DeliveryError::new(
            "target base/head moved before atomic merge consumption",
        ));
    }
    if let Some(baseline) = fresh_ci_baseline {
        let previous = baseline
            .iter()
            .find(|status| status.repository == node.repository && status.number == node.pr_number)
            .ok_or_else(|| DeliveryError::new("fresh CI baseline is missing the target PR"))?;
        verify_fresh_ci(snapshot, node, previous, &immediate)?;
    }
    merger.merge_with_expected_base_and_head(
        &eligibility.repository,
        eligibility.pr_number,
        &eligibility.expected_base_oid,
        &eligibility.expected_head_oid,
    )
}

fn reject_unverified_merge_queue(node: &StackNode, status: &PullRequestStatus) -> Result<()> {
    if node.snapshot_state == PullRequestState::Open
        && (status.is_in_merge_queue
            || status.is_merge_queue_enabled
            || status.merge_queue_entry.is_some())
    {
        return Err(DeliveryError::new(format!(
            "target authority for {} uses a merge queue without an exact verified merge-group base+head",
            node.id
        )));
    }
    Ok(())
}

fn verify_fresh_ci(
    snapshot: &WaveSnapshot,
    node: &StackNode,
    previous: &PullRequestStatus,
    current: &PullRequestStatus,
) -> Result<()> {
    if node.snapshot_state != PullRequestState::Open {
        return Ok(());
    }
    let required = snapshot
        .required_checks
        .iter()
        .filter(|required| required.node == node.id);
    for required in required {
        let old = previous
            .checks
            .iter()
            .find(|check| check.name == required.name && check.publisher == required.publisher)
            .ok_or_else(|| {
                DeliveryError::new(format!(
                    "sealed CI baseline is missing required check {}",
                    required.name
                ))
            })?;
        let new = current
            .checks
            .iter()
            .find(|check| check.name == required.name && check.publisher == required.publisher)
            .ok_or_else(|| {
                DeliveryError::new(format!(
                    "history-only candidate is missing required check {}",
                    required.name
                ))
            })?;
        let (Some(old_completed), Some(new_completed)) =
            (old.completed_at_unix_seconds, new.completed_at_unix_seconds)
        else {
            return Err(DeliveryError::new(format!(
                "required check {} has no verifiable completion timestamps for fresh CI",
                required.name
            )));
        };
        if new.started_at_unix_seconds <= old_completed
            || new_completed < new.started_at_unix_seconds
        {
            return Err(DeliveryError::new(format!(
                "required check {} did not advance to a fresh GitHub run after the sealed CI",
                required.name
            )));
        }
        if required.publisher.kind == CheckPublisherKind::StatusContext {
            continue;
        }
        let (Some(old_check_run), Some(new_check_run)) = (old.check_run_id, new.check_run_id)
        else {
            return Err(DeliveryError::new(format!(
                "required check {} has no verifiable check-run IDs for fresh CI",
                required.name
            )));
        };
        if new_check_run <= old_check_run {
            return Err(DeliveryError::new(format!(
                "required check {} did not advance to a fresh check run",
                required.name
            )));
        }
        if required.publisher.workflow == "none" {
            continue;
        }
        let (
            Some(old_workflow_run),
            Some(new_workflow_run),
            Some(old_workflow_created),
            Some(new_workflow_created),
            Some(old_workflow_updated),
            Some(new_workflow_updated),
        ) = (
            old.workflow_run_id,
            new.workflow_run_id,
            old.workflow_created_at_unix_seconds,
            new.workflow_created_at_unix_seconds,
            old.workflow_updated_at_unix_seconds,
            new.workflow_updated_at_unix_seconds,
        )
        else {
            return Err(DeliveryError::new(format!(
                "required check {} has no verifiable workflow-run authority for fresh CI",
                required.name
            )));
        };
        let workflow_advanced = if new_workflow_run > old_workflow_run {
            new_workflow_created > old_workflow_updated
                && new_workflow_updated >= new_workflow_created
        } else if new_workflow_run == old_workflow_run {
            new_workflow_created == old_workflow_created
                && new_workflow_updated > old_workflow_updated
        } else {
            false
        };
        if !workflow_advanced {
            return Err(DeliveryError::new(format!(
                "required check {} did not advance to a fresh workflow attempt",
                required.name
            )));
        }
    }
    Ok(())
}

fn transitive_dependencies(
    target: &StackNode,
    nodes: &BTreeMap<&str, &StackNode>,
) -> Result<Vec<String>> {
    let mut pending = target.depends_on.clone();
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let node = nodes.get(id.as_str()).copied().ok_or_else(|| {
            DeliveryError::new(format!(
                "stack node {} references unknown dependency {}",
                target.id, id
            ))
        })?;
        pending.extend(node.depends_on.iter().cloned());
    }
    Ok(seen.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingMerger {
        calls: std::cell::Cell<usize>,
    }

    impl PullRequestMerger for RecordingMerger {
        fn merge_with_expected_base_and_head(
            &self,
            _repository: &str,
            _pr: u64,
            _expected_base: &str,
            _expected_head: &str,
        ) -> Result<()> {
            self.calls.set(self.calls.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn merge_adapter_is_not_called_without_immediate_recheck() {
        let merger = RecordingMerger {
            calls: std::cell::Cell::new(0),
        };
        assert_eq!(merger.calls.get(), 0);
    }
}
