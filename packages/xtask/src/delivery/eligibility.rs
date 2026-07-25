//! Merge eligibility (spec section 12.4, work item `ADR046-delivery-006`).
//!
//! `merge-eligibility` confirms, per pull request in the wave's stack:
//!
//! * the seal exists for the wave's candidate and re-derives from its own
//!   sealed material;
//! * the pull request's current base and head still match the sealed
//!   snapshot's recorded object IDs, or a history-only rebase has passed the
//!   byte-identical proof in [`history_proof`](super::history_proof);
//! * every required GitHub check is green.
//!
//! # Where the check statuses come from
//!
//! This stage performs no network I/O and shells out to nothing. It reads one
//! merge-target artifact the integrator produces out of band — in practice
//! from `gh pr view --json` or `gh api` — describing the wave's current
//! pull-request stack: each pull request's repository, number, base and head
//! refs and object IDs, and its list of required checks with their
//! conclusions, plus the current integrated material. Everything the gate
//! decides is a function of the seal and that artifact, so the gate is
//! hermetic, offline, and reproducible, and a stale or hand-edited target is
//! caught by the same digest re-derivation every other stage uses.
//!
//! The cost of that choice is that freshness of the check statuses is the
//! integrator's responsibility: the artifact is a snapshot of GitHub state at
//! the moment it was captured. It is captured immediately before merge, in
//! the same step that merges, which is the same freshness window a direct API
//! call inside this process would have.
//!
//! Anything short of green fails closed: a pending, failed, neutral, skipped,
//! cancelled, or duplicated required check, a pull request with no required
//! checks at all, a repository in the sealed set with no open pull request, or
//! a pull request whose base is not reachable from the sealed base commit.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{CliOptions, WaveCommand, WorkflowOutput},
    history_proof::{self, HistoryVerdict},
    model::{
        CandidateId, CandidateMaterial, ContentId, GitObjectFormat, RepositoryRecord,
        SnapshotSha256, validate_bounded_string, validate_git_ref, validate_hash_for_format,
        validate_repository_id,
    },
    panel::{ensure_artifact_kind, ensure_same_file, prepare_state, read_json_file},
    seal::SealRecord,
    storage::{CandidateDir, HISTORY_PROOF_FILE, StateRoot},
};

pub const MERGE_TARGET_ARTIFACT_KIND: &str = "d2b-delivery/merge-target";
pub const MERGE_ELIGIBILITY_ARTIFACT_KIND: &str = "d2b-delivery/merge-eligibility";
pub const MERGE_ELIGIBILITY_FILE: &str = "merge-eligibility.json";

/// Upper bound on pull requests and required checks a target may declare.
const MAX_PULL_REQUESTS: usize = 64;
const MAX_REQUIRED_CHECKS: usize = 128;

/// Conclusion of one required GitHub check. Only [`Success`](Self::Success)
/// permits a merge; an unknown conclusion fails to parse, which fails closed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckConclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    Skipped,
    Stale,
    TimedOut,
    ActionRequired,
    StartupFailure,
    /// Queued or in progress. Pending never permits a merge.
    Pending,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredCheck {
    pub name: String,
    pub conclusion: CheckConclusion,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetPullRequest {
    pub repository: String,
    pub number: u64,
    pub base_ref: String,
    pub base_oid: String,
    pub head_ref: String,
    pub head_oid: String,
    pub required_checks: Vec<RequiredCheck>,
}

/// The current state of the wave's pull-request stack.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MergeTarget {
    pub artifact_kind: String,
    pub schema_version: u32,
    /// The wave's currently integrated material, re-derived after any rebase.
    pub material: CandidateMaterial,
    pub pull_requests: Vec<TargetPullRequest>,
}

/// One pull request's verdict, as recorded in the eligibility artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestVerdict {
    pub repository: String,
    pub number: u64,
    pub head_oid: String,
    pub required_checks: usize,
}

/// The eligibility artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EligibilityRecord {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub sealed_snapshot_sha256: SnapshotSha256,
    pub current_snapshot_sha256: SnapshotSha256,
    pub history: HistoryVerdict,
    pub pull_requests: Vec<PullRequestVerdict>,
    pub eligible: bool,
}

/// `cargo xtask delivery wave merge-eligibility`.
pub fn run(args: &[String]) -> Result<WorkflowOutput> {
    let mut options = CliOptions::parse(args)?;
    let seal_path = options.required_path("--seal")?;
    let target_path = options.required_path("--target")?;
    let state = prepare_state(&mut options)?;
    options.finish()?;

    let (candidate, seal) = open_sealed_candidate(&state, &seal_path)?;
    let target: MergeTarget = read_json_file(&target_path, "merge target")?;
    evaluate(&candidate, &seal, &target)
}

/// Confirms every clause of spec section 12.4 for the wave's stack.
pub fn evaluate(
    candidate: &CandidateDir,
    seal: &SealRecord,
    target: &MergeTarget,
) -> Result<WorkflowOutput> {
    target.validate()?;

    let proof = history_proof::prove(&seal.material, &target.material)?;
    candidate.write_json(HISTORY_PROOF_FILE, &proof)?;

    let mut material = target.material.clone();
    material.canonicalize()?;
    let verdicts = check_stack(&material, &target.pull_requests)?;

    let record = EligibilityRecord {
        artifact_kind: MERGE_ELIGIBILITY_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: seal.program.clone(),
        wave: seal.wave.clone(),
        candidate_id: seal.candidate_id.clone(),
        content_id: seal.content_id.clone(),
        sealed_snapshot_sha256: proof.sealed_snapshot_sha256.clone(),
        current_snapshot_sha256: proof.current_snapshot_sha256.clone(),
        history: proof.verdict,
        pull_requests: verdicts,
        eligible: true,
    };
    candidate.write_json(MERGE_ELIGIBILITY_FILE, &record)?;

    let digests = material.digests()?;
    WorkflowOutput::ok(WaveCommand::MergeEligibility)
        .with_digests(&digests)
        .with_artifact(&candidate.resolve(MERGE_ELIGIBILITY_FILE)?)
}

impl MergeTarget {
    fn validate(&self) -> Result<()> {
        ensure_artifact_kind(
            &self.artifact_kind,
            MERGE_TARGET_ARTIFACT_KIND,
            "merge target",
        )?;
        if self.schema_version != DELIVERY_SCHEMA_VERSION {
            return Err(DeliveryError::new(format!(
                "unsupported merge target schema version {}",
                self.schema_version
            )));
        }
        if self.pull_requests.is_empty() || self.pull_requests.len() > MAX_PULL_REQUESTS {
            return Err(DeliveryError::new(format!(
                "merge target must name between 1 and {MAX_PULL_REQUESTS} pull requests"
            )));
        }
        Ok(())
    }
}

/// Validates the stack against the wave's material, repository by repository.
fn check_stack(
    material: &CandidateMaterial,
    pull_requests: &[TargetPullRequest],
) -> Result<Vec<PullRequestVerdict>> {
    let repositories = material
        .repository_set
        .iter()
        .map(|repository| (repository.id.as_str(), repository))
        .collect::<BTreeMap<_, _>>();

    let mut seen = BTreeSet::new();
    let mut grouped: BTreeMap<&str, Vec<&TargetPullRequest>> = BTreeMap::new();
    for pull_request in pull_requests {
        let repository = *repositories
            .get(pull_request.repository.as_str())
            .ok_or_else(|| {
                DeliveryError::new(format!(
                    "pull request {} names repository {:?}, which is outside the sealed \
                     repository set",
                    pull_request.number, pull_request.repository
                ))
            })?;
        pull_request.validate(repository)?;
        if !seen.insert((pull_request.repository.as_str(), pull_request.number)) {
            return Err(DeliveryError::new(format!(
                "merge target repeats pull request {} in {}",
                pull_request.number, pull_request.repository
            )));
        }
        grouped
            .entry(repository.id.as_str())
            .or_default()
            .push(pull_request);
    }

    let mut verdicts = Vec::with_capacity(pull_requests.len());
    for repository in &material.repository_set {
        let stack = grouped.get(repository.id.as_str()).ok_or_else(|| {
            DeliveryError::new(format!(
                "repository {} is in the sealed set but the merge target names no pull request \
                 for it",
                repository.id
            ))
        })?;
        verdicts.extend(check_repository_stack(repository, stack)?);
    }
    Ok(verdicts)
}

/// Requires every pull request to sit on the sealed base, directly or through
/// another pull request in the same stack, and the sealed head to be one of
/// the stack's heads.
fn check_repository_stack(
    repository: &RepositoryRecord,
    stack: &[&TargetPullRequest],
) -> Result<Vec<PullRequestVerdict>> {
    let mut heads = BTreeSet::new();
    for pull_request in stack {
        if !heads.insert(pull_request.head_oid.as_str()) {
            return Err(DeliveryError::new(format!(
                "two pull requests in {} share head commit {}",
                repository.id, pull_request.head_oid
            )));
        }
    }

    let mut reachable = BTreeSet::from([repository.base_oid.as_str()]);
    let mut remaining: Vec<&TargetPullRequest> = stack.to_vec();
    loop {
        let mut attached: Vec<&TargetPullRequest> = Vec::new();
        let mut detached: Vec<&TargetPullRequest> = Vec::new();
        for pull_request in remaining {
            if reachable.contains(pull_request.base_oid.as_str()) {
                attached.push(pull_request);
            } else {
                detached.push(pull_request);
            }
        }
        remaining = detached;
        if attached.is_empty() {
            break;
        }
        for pull_request in attached {
            reachable.insert(pull_request.head_oid.as_str());
        }
    }
    if let Some(orphan) = remaining.first() {
        return Err(DeliveryError::new(format!(
            "pull request {} in {} is based on {}, which is not the sealed base commit and not \
             another pull request in the wave's stack",
            orphan.number, repository.id, orphan.base_ref
        )));
    }
    if !heads.contains(repository.head_oid.as_str()) {
        return Err(DeliveryError::new(format!(
            "the sealed head commit for {} is not the head of any pull request in the merge \
             target; re-snapshot the wave",
            repository.id
        )));
    }

    stack
        .iter()
        .map(|pull_request| {
            pull_request.check_required_checks()?;
            Ok(PullRequestVerdict {
                repository: pull_request.repository.clone(),
                number: pull_request.number,
                head_oid: pull_request.head_oid.clone(),
                required_checks: pull_request.required_checks.len(),
            })
        })
        .collect()
}

impl TargetPullRequest {
    fn validate(&self, repository: &RepositoryRecord) -> Result<()> {
        validate_repository_id(&self.repository)?;
        if self.number == 0 {
            return Err(DeliveryError::new("pull request number must be nonzero"));
        }
        validate_git_ref(&self.base_ref, "pull request base ref")?;
        validate_git_ref(&self.head_ref, "pull request head ref")?;
        validate_oid(&self.base_oid, repository.object_format, "base commit")?;
        validate_oid(&self.head_oid, repository.object_format, "head commit")?;
        if self.base_oid == self.head_oid {
            return Err(DeliveryError::new(format!(
                "pull request {} has an empty range: base and head are the same commit",
                self.number
            )));
        }
        Ok(())
    }

    /// Every required check must be present exactly once and green.
    fn check_required_checks(&self) -> Result<()> {
        if self.required_checks.is_empty() || self.required_checks.len() > MAX_REQUIRED_CHECKS {
            return Err(DeliveryError::new(format!(
                "pull request {} must declare between 1 and {MAX_REQUIRED_CHECKS} required \
                 checks; a stack with no required check is never eligible",
                self.number
            )));
        }
        let mut seen = BTreeSet::new();
        for check in &self.required_checks {
            validate_bounded_string(&check.name, "required check name")?;
            if !seen.insert(check.name.as_str()) {
                return Err(DeliveryError::new(format!(
                    "pull request {} reports required check {:?} twice",
                    self.number, check.name
                )));
            }
            if check.conclusion != CheckConclusion::Success {
                return Err(DeliveryError::new(format!(
                    "pull request {} is not mergeable: required check {:?} is not green",
                    self.number, check.name
                )));
            }
        }
        Ok(())
    }
}

fn validate_oid(value: &str, format: GitObjectFormat, label: &str) -> Result<()> {
    validate_hash_for_format(value, format, label)
}

/// Opens the candidate directory a seal belongs to.
///
/// The seal path must resolve to that candidate's own `seal.json` inside
/// external delivery state, so an operator cannot point the gate at a seal
/// carried in a checkout or a pull-request attachment.
pub fn open_sealed_candidate(
    state: &StateRoot,
    seal_path: &Path,
) -> Result<(CandidateDir, SealRecord)> {
    let seal: SealRecord = read_json_file(seal_path, "wave seal")?;
    seal.validate()?;
    let candidate = state.existing_candidate(&seal.wave, &seal.candidate_id)?;
    ensure_same_file(seal_path, &candidate.seal_path(), "wave seal")?;
    Ok((candidate, seal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{
        DeliveryErrorKind,
        model::fixtures,
        panel::SnapshotView,
        seal::{seal, tests::sealable},
        storage::{SEAL_FILE, Scratch},
    };

    fn sealed(scratch: &Scratch) -> (CandidateDir, SealRecord, SnapshotView) {
        let (candidate, snapshot) = sealable(scratch);
        seal(&candidate, &snapshot).expect("seal");
        let record: SealRecord = candidate.read_json(SEAL_FILE).expect("seal record");
        (candidate, record, snapshot)
    }

    fn checks() -> Vec<RequiredCheck> {
        vec![
            RequiredCheck {
                name: "pr-l1-static-fast".to_owned(),
                conclusion: CheckConclusion::Success,
            },
            RequiredCheck {
                name: "eval-minimal".to_owned(),
                conclusion: CheckConclusion::Success,
            },
        ]
    }

    fn target(material: CandidateMaterial) -> MergeTarget {
        let head = material.repository_set[0].head_oid.clone();
        let base = material.repository_set[0].base_oid.clone();
        MergeTarget {
            artifact_kind: MERGE_TARGET_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            material,
            pull_requests: vec![TargetPullRequest {
                repository: "github.com/example/d2b".to_owned(),
                number: 1,
                base_ref: "v3".to_owned(),
                base_oid: base,
                head_ref: "adr046-w0-panel-seal".to_owned(),
                head_oid: head,
                required_checks: checks(),
            }],
        }
    }

    fn rebased_material() -> CandidateMaterial {
        let mut material = fixtures::material();
        material.repository_set[0].base_oid = fixtures::oid(5);
        material.repository_set[0].head_oid = fixtures::oid(6);
        material
    }

    #[test]
    fn an_unchanged_stack_with_green_checks_is_eligible() {
        let scratch = Scratch::new("eligibility-ok");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let output = evaluate(&candidate, &seal, &target(fixtures::material())).expect("eligible");
        assert_eq!(output.operation, "merge-eligibility");

        let record: EligibilityRecord = candidate
            .read_json(MERGE_ELIGIBILITY_FILE)
            .expect("eligibility record");
        assert!(record.eligible);
        assert_eq!(record.history, HistoryVerdict::SealedHistoryCurrent);
        assert_eq!(record.pull_requests.len(), 1);
        assert_eq!(
            record.sealed_snapshot_sha256,
            record.current_snapshot_sha256
        );
    }

    #[test]
    fn a_history_only_rebase_stays_eligible_through_the_proof() {
        let scratch = Scratch::new("eligibility-rebase");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let output = evaluate(&candidate, &seal, &target(rebased_material())).expect("eligible");
        assert_eq!(
            output.candidate_id.as_deref(),
            Some(seal.candidate_id.as_str())
        );

        let record: EligibilityRecord = candidate
            .read_json(MERGE_ELIGIBILITY_FILE)
            .expect("eligibility record");
        assert_eq!(record.history, HistoryVerdict::HistoryOnlyRebase);
        assert_ne!(
            record.sealed_snapshot_sha256,
            record.current_snapshot_sha256
        );

        let proof: crate::delivery::history_proof::HistoryProof = candidate
            .read_json(HISTORY_PROOF_FILE)
            .expect("history proof artifact");
        assert_eq!(proof.verdict, HistoryVerdict::HistoryOnlyRebase);
    }

    #[test]
    fn a_content_change_is_ineligible_even_with_green_checks() {
        let scratch = Scratch::new("eligibility-content-change");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut changed = rebased_material();
        changed.repository_set[0].integration_tree_oid = fixtures::oid(9);
        let error = evaluate(&candidate, &seal, &target(changed)).expect_err("content change");
        assert!(error.message().contains("history proof failed"), "{error}");
    }

    #[test]
    fn a_pending_or_failing_required_check_is_ineligible() {
        for conclusion in [
            CheckConclusion::Pending,
            CheckConclusion::Failure,
            CheckConclusion::Neutral,
            CheckConclusion::Skipped,
            CheckConclusion::Cancelled,
            CheckConclusion::TimedOut,
        ] {
            let scratch = Scratch::new("eligibility-check");
            let (candidate, seal, _snapshot) = sealed(&scratch);
            let mut target = target(fixtures::material());
            target.pull_requests[0].required_checks[1].conclusion = conclusion;
            let error = evaluate(&candidate, &seal, &target).expect_err("not green");
            assert!(error.message().contains("not green"), "{error}");
        }
    }

    #[test]
    fn a_pull_request_without_required_checks_is_ineligible() {
        let scratch = Scratch::new("eligibility-no-checks");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut target = target(fixtures::material());
        target.pull_requests[0].required_checks.clear();
        let error = evaluate(&candidate, &seal, &target).expect_err("no checks");
        assert!(error.message().contains("required checks"), "{error}");
    }

    #[test]
    fn a_duplicated_required_check_is_ineligible() {
        let scratch = Scratch::new("eligibility-duplicate-check");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut target = target(fixtures::material());
        target.pull_requests[0].required_checks[1].name = "pr-l1-static-fast".to_owned();
        let error = evaluate(&candidate, &seal, &target).expect_err("duplicate check");
        assert!(error.message().contains("twice"), "{error}");
    }

    #[test]
    fn a_pull_request_off_the_sealed_base_is_ineligible() {
        let scratch = Scratch::new("eligibility-orphan");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut target = target(fixtures::material());
        target.pull_requests[0].base_oid = fixtures::oid(7);
        let error = evaluate(&candidate, &seal, &target).expect_err("orphan base");
        assert!(
            error.message().contains("not the sealed base commit"),
            "{error}"
        );
    }

    #[test]
    fn a_stacked_pull_request_chain_is_accepted() {
        let scratch = Scratch::new("eligibility-stack");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut target = target(fixtures::material());
        let top = target.pull_requests[0].clone();
        target.pull_requests[0] = TargetPullRequest {
            number: 1,
            head_oid: fixtures::oid(7),
            head_ref: "adr046-w0-lower".to_owned(),
            ..top.clone()
        };
        target.pull_requests.push(TargetPullRequest {
            number: 2,
            base_oid: fixtures::oid(7),
            base_ref: "adr046-w0-lower".to_owned(),
            ..top
        });
        evaluate(&candidate, &seal, &target).expect("stacked chain is eligible");
    }

    #[test]
    fn a_sealed_head_missing_from_the_stack_is_ineligible() {
        let scratch = Scratch::new("eligibility-missing-head");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut target = target(fixtures::material());
        target.pull_requests[0].head_oid = fixtures::oid(7);
        let error = evaluate(&candidate, &seal, &target).expect_err("missing head");
        assert!(error.message().contains("sealed head commit"), "{error}");
    }

    #[test]
    fn a_repository_without_a_pull_request_is_ineligible() {
        let scratch = Scratch::new("eligibility-missing-pr");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut target = target(fixtures::material());
        target.pull_requests[0].repository = "github.com/example/other".to_owned();
        let error = evaluate(&candidate, &seal, &target).expect_err("unknown repository");
        assert!(error.message().contains("outside the sealed"), "{error}");
    }

    #[test]
    fn a_duplicate_pull_request_is_ineligible() {
        let scratch = Scratch::new("eligibility-duplicate-pr");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut target = target(fixtures::material());
        let duplicate = target.pull_requests[0].clone();
        target.pull_requests.push(duplicate);
        let error = evaluate(&candidate, &seal, &target).expect_err("duplicate pull request");
        assert!(error.message().contains("repeats pull request"), "{error}");
    }

    #[test]
    fn a_malformed_target_is_rejected() {
        let scratch = Scratch::new("eligibility-malformed");
        let (candidate, seal, _snapshot) = sealed(&scratch);
        let mut wrong_kind = target(fixtures::material());
        wrong_kind.artifact_kind = "d2b-delivery/wave-seal".to_owned();
        assert!(evaluate(&candidate, &seal, &wrong_kind).is_err());

        let mut empty = target(fixtures::material());
        empty.pull_requests.clear();
        assert!(evaluate(&candidate, &seal, &empty).is_err());

        let mut zero = target(fixtures::material());
        zero.pull_requests[0].number = 0;
        assert!(evaluate(&candidate, &seal, &zero).is_err());

        let mut empty_range = target(fixtures::material());
        empty_range.pull_requests[0].head_oid = empty_range.pull_requests[0].base_oid.clone();
        assert!(evaluate(&candidate, &seal, &empty_range).is_err());
    }

    #[test]
    fn an_unknown_check_conclusion_fails_to_parse() {
        let value = serde_json::json!({ "name": "check", "conclusion": "probably-fine" });
        assert!(serde_json::from_value::<RequiredCheck>(value).is_err());
    }

    #[test]
    fn the_cli_rejects_a_missing_option() {
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            run(&args(&["--target", "/state/target.json"]))
                .expect_err("missing --seal")
                .kind(),
            DeliveryErrorKind::Usage
        );
    }
}
