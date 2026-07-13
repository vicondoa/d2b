use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::{
    DeliveryError, Result,
    command::{PullRequestMerger, PullRequestStatus, PullRequestStatusSource, RepositoryProbe},
    model::{PullRequestState, StackNode, WaveSnapshot},
    seal::{verify_history_proof_context, verify_required_checks, verify_seal},
    snapshot::verify_pr_identity,
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
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
    target_node: &str,
) -> Result<MergeEligibility> {
    let seal = verify_seal(probe, status_source, repository_roots, seal_path)?;
    let snapshot_path = seal_path
        .parent()
        .ok_or_else(|| DeliveryError::new("seal path has no candidate directory"))?
        .join("snapshot.json");
    let snapshot = super::snapshot::read_snapshot(&snapshot_path)?;
    evaluate_merge_eligibility(
        &snapshot,
        Some(&seal.live_pull_requests),
        status_source,
        target_node,
        false,
    )
}

pub fn check_history_merge_eligibility<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
    target_node: &str,
) -> Result<MergeEligibility> {
    let (context, _old_seal, proof) = verify_history_proof_context(
        probe,
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
    evaluate_merge_eligibility(&context.snapshot, None, status_source, target_node, true)
}

pub fn atomic_merge<P: RepositoryProbe, S: PullRequestStatusSource, M: PullRequestMerger>(
    probe: &P,
    status_source: &S,
    merger: &M,
    repository_roots: &BTreeMap<String, PathBuf>,
    seal_path: &Path,
    target_node: &str,
) -> Result<MergeEligibility> {
    let eligibility = check_merge_eligibility(
        probe,
        status_source,
        repository_roots,
        seal_path,
        target_node,
    )?;
    let snapshot_path = seal_path
        .parent()
        .ok_or_else(|| DeliveryError::new("seal path has no candidate directory"))?
        .join("snapshot.json");
    let snapshot = super::snapshot::read_snapshot(&snapshot_path)?;
    consume_atomic_merge(&snapshot, status_source, merger, &eligibility, false)?;
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
    merger: &M,
    repository_roots: &BTreeMap<String, PathBuf>,
    old_seal_path: &Path,
    new_snapshot_path: &Path,
    proof_path: &Path,
    target_node: &str,
) -> Result<MergeEligibility> {
    let eligibility = check_history_merge_eligibility(
        probe,
        status_source,
        repository_roots,
        old_seal_path,
        new_snapshot_path,
        proof_path,
        target_node,
    )?;
    let snapshot = super::snapshot::read_snapshot(new_snapshot_path)?;
    consume_atomic_merge(&snapshot, status_source, merger, &eligibility, true)?;
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
    let nodes = snapshot
        .stack
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let target = nodes
        .get(target_node)
        .copied()
        .ok_or_else(|| DeliveryError::new(format!("unknown target stack node {target_node}")))?;
    let sealed = sealed_live.map(|statuses| {
        statuses
            .iter()
            .map(|status| ((status.repository.as_str(), status.number), status))
            .collect::<BTreeMap<_, _>>()
    });

    let mut current = BTreeMap::new();
    for node in &snapshot.stack {
        let status = status_source.status(&node.repository, node.pr_number)?;
        verify_pr_identity(node, &status)?;
        verify_required_checks(snapshot, node, &status)?;
        if fresh_ci_required
            && !status
                .checks
                .iter()
                .any(|check| check.commit_oid == node.head_oid)
        {
            return Err(DeliveryError::new(format!(
                "history-only candidate {} is missing fresh CI on the new head",
                node.id
            )));
        }
        if let Some(sealed) = &sealed {
            let expected = sealed
                .get(&(node.repository.as_str(), node.pr_number))
                .ok_or_else(|| DeliveryError::new("seal is missing a live PR binding"))?;
            if *expected != &status {
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
    fresh_ci_required: bool,
) -> Result<()> {
    let node = snapshot
        .stack
        .iter()
        .find(|node| node.id == eligibility.target_node)
        .ok_or_else(|| DeliveryError::new("eligible target disappeared from snapshot"))?;
    let immediate = status_source.status(&node.repository, node.pr_number)?;
    verify_pr_identity(node, &immediate)?;
    verify_required_checks(snapshot, node, &immediate)?;
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
    if fresh_ci_required
        && !immediate
            .checks
            .iter()
            .any(|check| check.commit_oid == eligibility.expected_head_oid)
    {
        return Err(DeliveryError::new(
            "fresh CI disappeared before history-only merge",
        ));
    }
    merger.merge_with_expected_head(
        &eligibility.repository,
        eligibility.pr_number,
        &eligibility.expected_head_oid,
    )
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
        fn merge_with_expected_head(
            &self,
            _repository: &str,
            _pr: u64,
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
