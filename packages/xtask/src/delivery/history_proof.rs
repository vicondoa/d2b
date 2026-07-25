//! Byte-identical history proof (spec section 12.6, work item
//! `ADR046-delivery-006`).
//!
//! A history-only rebase or retarget may reuse prior panel records only when
//! the old and new histories carry byte-identical integrated content,
//! byte-identical generated artifacts, and a byte-identical dependency diff
//! and repository set. [`prove`] checks each of those clauses separately, so a
//! failure names the clause that failed rather than reporting one opaque
//! digest mismatch.
//!
//! The proof rests on [`ContentId`], which excludes commit history by
//! construction, so a rebase that preserves content reproduces it exactly.
//! `snapshot_sha256` covers base and head object IDs, so it is the value that
//! actually detects the rebase; the proof records both.
//!
//! The proof preserves the panel record only. Required CI still reruns on the
//! new history, which is why nothing here reports a check result.
//!
//! There is no `wave history-proof` subcommand. The proof is an input to
//! [`merge-eligibility`](super::eligibility), which runs it on every
//! evaluation and writes the resulting artifact into candidate-addressed
//! state. Keeping it module-only holds the operator surface to the stages
//! spec section 12.4 names and leaves no way to obtain a proof artifact that
//! was not produced by the eligibility check that consumed it.

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    model::{
        CandidateId, CandidateMaterial, ContentId, HISTORY_PROOF_ARTIFACT_KIND, SnapshotSha256,
    },
};

/// What changed between the sealed history and the current one.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryVerdict {
    /// Base and head object IDs are unchanged: the sealed snapshot is current.
    SealedHistoryCurrent,
    /// Base or head moved while every byte of integrated content stayed put.
    HistoryOnlyRebase,
}

/// One repository's base and head movement under the proof.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryHistory {
    pub repository: String,
    pub sealed_base_oid: String,
    pub sealed_head_oid: String,
    pub current_base_oid: String,
    pub current_head_oid: String,
    pub integration_tree_oid: String,
}

/// The proof artifact consumed by [`merge-eligibility`](super::eligibility).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryProof {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub verdict: HistoryVerdict,
    pub content_id: ContentId,
    pub candidate_id: CandidateId,
    pub sealed_snapshot_sha256: SnapshotSha256,
    pub current_snapshot_sha256: SnapshotSha256,
    pub repositories: Vec<RepositoryHistory>,
}

/// Verifies that `current` differs from `sealed` in commit history only.
///
/// Every clause of spec section 12.6 is checked separately:
///
/// 1. the wave identity (program and wave) is the same wave;
/// 2. the repository set is byte-identical in membership and object format;
/// 3. every repository's integrated tree object ID is byte-identical;
/// 4. generated artifacts, dependency metadata, and contract fingerprints are
///    byte-identical;
/// 5. the dependency graph is byte-identical;
/// 6. the derived `content_id` and `candidate_id` reproduce exactly.
///
/// Any content change fails, which is the invalidation rule: the wave
/// re-snapshots and both lanes rerun.
pub fn prove(sealed: &CandidateMaterial, current: &CandidateMaterial) -> Result<HistoryProof> {
    let mut sealed = sealed.clone();
    let mut current = current.clone();
    sealed.canonicalize()?;
    current.canonicalize()?;

    if sealed.program != current.program || sealed.wave != current.wave {
        return Err(DeliveryError::new(
            "history proof compares two different waves; a proof is only meaningful within one \
             wave's candidate",
        ));
    }

    let sealed_members = sealed
        .repository_set
        .iter()
        .map(|repository| (repository.id.as_str(), repository.object_format))
        .collect::<Vec<_>>();
    let current_members = current
        .repository_set
        .iter()
        .map(|repository| (repository.id.as_str(), repository.object_format))
        .collect::<Vec<_>>();
    if sealed_members != current_members {
        return Err(DeliveryError::new(
            "history proof failed: the repository set changed, which is a content change and \
             invalidates the panel",
        ));
    }

    let mut repositories = Vec::with_capacity(sealed.repository_set.len());
    for (was, now) in sealed.repository_set.iter().zip(&current.repository_set) {
        if was.integration_tree_oid != now.integration_tree_oid {
            return Err(DeliveryError::new(format!(
                "history proof failed: repository {} integrates a different tree, so the \
                 content is not byte-identical",
                now.id
            )));
        }
        repositories.push(RepositoryHistory {
            repository: now.id.clone(),
            sealed_base_oid: was.base_oid.clone(),
            sealed_head_oid: was.head_oid.clone(),
            current_base_oid: now.base_oid.clone(),
            current_head_oid: now.head_oid.clone(),
            integration_tree_oid: now.integration_tree_oid.clone(),
        });
    }

    for (label, was, now) in [
        (
            "generated artifacts",
            &sealed.generated_artifacts,
            &current.generated_artifacts,
        ),
        (
            "dependency metadata",
            &sealed.dependency_fingerprints,
            &current.dependency_fingerprints,
        ),
        (
            "contract fingerprints",
            &sealed.contract_fingerprints,
            &current.contract_fingerprints,
        ),
    ] {
        if was != now {
            return Err(DeliveryError::new(format!(
                "history proof failed: {label} are not byte-identical across the rebase"
            )));
        }
    }

    if sealed.dependency_graph != current.dependency_graph {
        return Err(DeliveryError::new(
            "history proof failed: the dependency diff is not byte-identical across the rebase",
        ));
    }

    let sealed_digests = sealed.digests()?;
    let current_digests = current.digests()?;
    if sealed_digests.content_id != current_digests.content_id
        || sealed_digests.candidate_id != current_digests.candidate_id
    {
        return Err(DeliveryError::new(
            "history proof failed: the candidate no longer reproduces its content identity",
        ));
    }

    let verdict = if repositories.iter().all(|repository| {
        repository.sealed_base_oid == repository.current_base_oid
            && repository.sealed_head_oid == repository.current_head_oid
    }) {
        HistoryVerdict::SealedHistoryCurrent
    } else {
        HistoryVerdict::HistoryOnlyRebase
    };

    Ok(HistoryProof {
        artifact_kind: HISTORY_PROOF_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: current.program.clone(),
        wave: current.wave.clone(),
        verdict,
        content_id: current_digests.content_id,
        candidate_id: current_digests.candidate_id,
        sealed_snapshot_sha256: sealed_digests.snapshot_sha256,
        current_snapshot_sha256: current_digests.snapshot_sha256,
        repositories,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::model::{
        DependencyEdge, ExpectedPullRequest, GitObjectFormat, RepositoryRecord, fixtures,
    };

    fn rebased() -> CandidateMaterial {
        let mut material = fixtures::material();
        material.repository_set[0].base_oid = fixtures::oid(5);
        material.repository_set[0].head_oid = fixtures::oid(6);
        material.repository_set[0].expected_pull_requests[0].head_oid = fixtures::oid(6);
        material
    }

    #[test]
    fn an_unchanged_candidate_proves_the_sealed_history_is_current() {
        let proof = prove(&fixtures::material(), &fixtures::material()).expect("proof");
        assert_eq!(proof.verdict, HistoryVerdict::SealedHistoryCurrent);
        assert_eq!(proof.sealed_snapshot_sha256, proof.current_snapshot_sha256);
        assert_eq!(proof.artifact_kind, HISTORY_PROOF_ARTIFACT_KIND);
    }

    #[test]
    fn a_history_only_rebase_passes_and_reuses_the_candidate_address() {
        let proof = prove(&fixtures::material(), &rebased()).expect("proof");
        assert_eq!(proof.verdict, HistoryVerdict::HistoryOnlyRebase);
        assert_ne!(proof.sealed_snapshot_sha256, proof.current_snapshot_sha256);
        let sealed = fixtures::material().digests().expect("digests");
        assert_eq!(proof.content_id, sealed.content_id);
        assert_eq!(proof.candidate_id, sealed.candidate_id);
    }

    #[test]
    fn input_ordering_does_not_change_the_proof() {
        let mut shuffled = rebased();
        shuffled.dependency_graph.reverse();
        shuffled.generated_artifacts.reverse();
        let ordered = prove(&fixtures::material(), &rebased()).expect("proof");
        let reordered = prove(&fixtures::material(), &shuffled).expect("proof");
        assert_eq!(ordered, reordered);
    }

    #[test]
    fn a_changed_integrated_tree_fails_the_proof() {
        let mut changed = rebased();
        changed.repository_set[0].integration_tree_oid = fixtures::oid(9);
        let error = prove(&fixtures::material(), &changed).expect_err("content change");
        assert!(error.message().contains("different tree"), "{error}");
    }

    #[test]
    fn a_changed_generated_artifact_fails_the_proof() {
        let mut changed = rebased();
        changed.generated_artifacts[0].sha256 = fixtures::fingerprint("x", "x", 42).sha256;
        let error = prove(&fixtures::material(), &changed).expect_err("content change");
        assert!(error.message().contains("generated artifacts"), "{error}");
    }

    #[test]
    fn changed_dependency_metadata_fails_the_proof() {
        let mut changed = rebased();
        changed.dependency_fingerprints[0].sha256 = fixtures::fingerprint("x", "x", 43).sha256;
        let error = prove(&fixtures::material(), &changed).expect_err("content change");
        assert!(error.message().contains("dependency metadata"), "{error}");
    }

    #[test]
    fn a_changed_contract_fingerprint_fails_the_proof() {
        let mut changed = rebased();
        changed.contract_fingerprints[0].sha256 = fixtures::fingerprint("x", "x", 44).sha256;
        let error = prove(&fixtures::material(), &changed).expect_err("content change");
        assert!(error.message().contains("contract fingerprints"), "{error}");
    }

    #[test]
    fn a_changed_dependency_diff_fails_the_proof() {
        let mut changed = rebased();
        changed.dependency_graph.push(DependencyEdge {
            from: "adr046-w2".to_owned(),
            to: "adr046-w3".to_owned(),
        });
        let error = prove(&fixtures::material(), &changed).expect_err("content change");
        assert!(error.message().contains("dependency diff"), "{error}");
    }

    #[test]
    fn a_changed_repository_set_fails_the_proof() {
        let mut changed = rebased();
        changed.repository_set.push(RepositoryRecord {
            id: "github.com/example/entrablau".to_owned(),
            object_format: GitObjectFormat::Sha1,
            base_oid: fixtures::oid(7),
            head_oid: fixtures::oid(8),
            integration_tree_oid: fixtures::oid(9),
            expected_pull_requests: vec![ExpectedPullRequest {
                number: 1,
                head_oid: fixtures::oid(8),
            }],
        });
        let error = prove(&fixtures::material(), &changed).expect_err("content change");
        assert!(error.message().contains("repository set"), "{error}");
    }

    #[test]
    fn a_different_wave_fails_the_proof() {
        let mut changed = rebased();
        changed.wave = "W1".to_owned();
        let error = prove(&fixtures::material(), &changed).expect_err("different wave");
        assert!(error.message().contains("different waves"), "{error}");
    }

    #[test]
    fn malformed_material_is_rejected_before_any_verdict() {
        let mut malformed = rebased();
        malformed.repository_set[0].base_oid = "not-an-oid".to_owned();
        assert!(prove(&fixtures::material(), &malformed).is_err());
    }
}
