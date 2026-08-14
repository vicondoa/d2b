//! Wave seal construction (spec section 12.4, work item
//! `ADR046-delivery-006`).
//!
//! `seal` requires every spec-section-12.2 validator lane to report success on
//! the exact snapshot. It writes one sealed record binding the wave's candidate
//! triple and the lanes it accepted.
//!
//! The two validator lanes - required GitHub CI and the heavy-gated local/host
//! validators - must each carry at least one imported result, and every
//! imported result must be a pass.
//! [`EvidenceResult`] has no pending state by construction: a pending lane is
//! an absent record, and an absent lane fails the seal.
//!
//! A history-only rebase moves `snapshot_sha256` while leaving the content and
//! candidate identities alone. Every prior validator result therefore becomes
//! stale and each lane must re-import against the new snapshot.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result, SnapshotView,
    command::{CliOptions, WaveCommand, WorkflowOutput},
    ensure_artifact_kind,
    evidence::{self, EvidenceLane, REQUIRED_EVIDENCE_LANES},
    model::{
        CandidateId, CandidateMaterial, ContentId, SEAL_ARTIFACT_KIND, SnapshotSha256,
        sha256_bytes, validate_identifier, validate_program_wave, validate_sha256,
    },
    storage::{CandidateDir, SEAL_FILE, StateRoot},
};

/// One lane's accepted validations, as bound into the seal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SealedLane {
    pub lane: EvidenceLane,
    pub validations: Vec<SealedValidation>,
}

/// One accepted validation, bound by the SHA-256 of the exact evidence record
/// the seal accepted.
///
/// Recording the record digest makes the seal commit to the precise validator
/// evidence it was built from, so evidence cannot be swapped under a seal after
/// the fact and a reader can tell which record each lane entry vouches for.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SealedValidation {
    pub validation: String,
    pub record_sha256: String,
}

/// The sealed record.
///
/// It carries the sealed material because
/// [`merge-eligibility`](super::eligibility) needs the sealed base and head
/// object IDs and the byte-identical content inputs to run the history proof
/// of spec section 12.6 without re-reading the snapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SealRecord {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub snapshot_sha256: SnapshotSha256,
    pub material: CandidateMaterial,
    pub evidence: Vec<SealedLane>,
}

impl SealRecord {
    /// Re-validates a seal read back from delivery state.
    pub fn validate(&self, candidate: &CandidateDir) -> Result<()> {
        ensure_artifact_kind(&self.artifact_kind, SEAL_ARTIFACT_KIND, "wave seal")?;
        if self.schema_version != DELIVERY_SCHEMA_VERSION {
            return Err(DeliveryError::new(format!(
                "unsupported wave seal schema version {}",
                self.schema_version
            )));
        }
        validate_program_wave(&self.program, &self.wave)?;
        if self.program != self.material.program || self.wave != self.material.wave {
            return Err(DeliveryError::new(
                "wave seal names a different wave than the material it sealed",
            ));
        }
        let derived = self.material.digests()?;
        if derived.content_id != self.content_id
            || derived.candidate_id != self.candidate_id
            || derived.snapshot_sha256 != self.snapshot_sha256
        {
            return Err(DeliveryError::new(
                "wave seal digests do not re-derive from the sealed material",
            ));
        }
        candidate.validate_artifact_address(&self.wave, &self.candidate_id, "wave seal")?;
        for lane in &self.evidence {
            for validation in &lane.validations {
                validate_identifier(&validation.validation, "sealed validation")?;
                validate_sha256(&validation.record_sha256, "sealed evidence record digest")?;
            }
        }
        ensure_required_lanes(&self.evidence.iter().map(|lane| lane.lane).collect())
    }
}

/// `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave seal`.
pub fn run(args: &[String]) -> Result<WorkflowOutput> {
    let mut options = CliOptions::parse(args)?;
    let snapshot_path = options.required_path("--snapshot")?;
    let (state, repository_roots) = super::prepare_state_with_roots(&mut options)?;
    options.finish()?;
    let snapshot_path = state.resolve_artifact_ref(&snapshot_path);
    let (candidate, snapshot) = super::open_candidate(&state, &snapshot_path)?;
    super::work_item_state::reject_adr046_w5_mutation(&snapshot.material, "seal")?;
    seal_checked(&state, &candidate, &snapshot, &repository_roots)
}

fn seal_checked(
    state: &StateRoot,
    candidate: &CandidateDir,
    snapshot: &SnapshotView,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<WorkflowOutput> {
    super::work_item_state::require_current_wave_merged(&snapshot.material, repository_roots)?;
    super::work_item_state::require_predecessor_state_for_exit(
        state,
        &snapshot.material,
        repository_roots,
    )?;
    seal(candidate, snapshot)
}

/// Binds passing validator lanes to one candidate.
pub fn seal(candidate: &CandidateDir, snapshot: &SnapshotView) -> Result<WorkflowOutput> {
    let evidence = sealed_lanes(candidate, snapshot)?;

    let record = SealRecord {
        artifact_kind: SEAL_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: snapshot.program().to_owned(),
        wave: snapshot.wave().to_owned(),
        candidate_id: snapshot.candidate_id.clone(),
        content_id: snapshot.content_id.clone(),
        snapshot_sha256: snapshot.snapshot_sha256.clone(),
        material: snapshot.material.clone(),
        evidence,
    };
    record.validate(candidate)?;
    candidate.write_json(SEAL_FILE, &record)?;

    WorkflowOutput::ok(WaveCommand::Seal)
        .with_digests(&snapshot.digests())
        .with_artifact(candidate, &candidate.seal_path())
}

/// Reads every imported evidence record through the one shared reader and
/// groups the passing ones by lane, binding each record's digest into the
/// seal.
///
/// The reader is [`evidence::require_passing_lanes`], the exact enforcement
/// `merge-eligibility` also runs, so `seal` consumes precisely what
/// `wave validate-import` produced against the *current* snapshot: the
/// canonical nested layout `evidence/<lane>/<validation>.json`, bound to this
/// candidate's three digests, non-empty, all passing, and covering every
/// required lane. Because a lane and validation together address one file, a
/// lane can never hold two records for one validation, so re-importing is
/// idempotent rather than a conflict.
fn sealed_lanes(candidate: &CandidateDir, snapshot: &SnapshotView) -> Result<Vec<SealedLane>> {
    let records = evidence::require_passing_lanes(
        candidate,
        &snapshot.candidate_id,
        &snapshot.content_id,
        &snapshot.snapshot_sha256,
    )?;
    let mut lanes: BTreeMap<EvidenceLane, BTreeMap<String, String>> = BTreeMap::new();
    for record in records {
        let record_sha256 = sha256_bytes(&serde_json::to_vec(&record)?);
        lanes
            .entry(record.lane)
            .or_default()
            .insert(record.validation, record_sha256);
    }
    Ok(lanes
        .into_iter()
        .map(|(lane, validations)| SealedLane {
            lane,
            validations: validations
                .into_iter()
                .map(|(validation, record_sha256)| SealedValidation {
                    validation,
                    record_sha256,
                })
                .collect(),
        })
        .collect())
}

fn ensure_required_lanes(present: &BTreeSet<EvidenceLane>) -> Result<()> {
    for lane in REQUIRED_EVIDENCE_LANES {
        if !present.contains(&lane) {
            return Err(DeliveryError::new(format!(
                "validator lane {} has no passing result for this candidate; a pending lane \
                 never permits a seal",
                lane.as_str()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::delivery::{
        evidence::EvidenceRecord,
        model::{CandidateMaterial, EVIDENCE_ARTIFACT_KIND, EvidenceResult, fixtures},
        snapshot::tests::{GitFixture, take},
        storage::tests::Scratch,
        test_support::{candidate_with_snapshot, candidate_with_snapshot_from},
    };

    pub(crate) fn evidence(
        snapshot: &SnapshotView,
        lane: EvidenceLane,
        validation: &str,
    ) -> EvidenceRecord {
        EvidenceRecord {
            artifact_kind: EVIDENCE_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            program: snapshot.program().to_owned(),
            wave: snapshot.wave().to_owned(),
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            lane,
            validation: validation.to_owned(),
            result: EvidenceResult::Passed,
            imported_at_unix: 0,
            command: None,
            output: None,
            locator: None,
        }
    }

    /// Files an evidence record through the production writer, so tests exercise
    /// the same canonical nested layout `wave validate-import` writes and
    /// `wave seal` reads.
    pub(crate) fn import(candidate: &CandidateDir, record: &EvidenceRecord) {
        evidence::import(candidate, record).expect("import evidence");
    }

    /// A candidate with both required validator lanes imported.
    pub(crate) fn sealable(scratch: &Scratch) -> (CandidateDir, SnapshotView) {
        let (_state, candidate, snapshot) = candidate_with_snapshot(scratch);
        finish_sealable(candidate, snapshot)
    }

    /// Like [`sealable`], but over a caller-supplied material, so a test can
    /// seal a wave whose expected pull-request set is not the single-slice
    /// fixture default.
    pub(crate) fn sealable_from(
        scratch: &Scratch,
        material: CandidateMaterial,
    ) -> (CandidateDir, SnapshotView) {
        let (_state, candidate, snapshot) = candidate_with_snapshot_from(scratch, material);
        finish_sealable(candidate, snapshot)
    }

    fn finish_sealable(
        candidate: CandidateDir,
        snapshot: SnapshotView,
    ) -> (CandidateDir, SnapshotView) {
        import(
            &candidate,
            &evidence(&snapshot, EvidenceLane::GithubCi, "layer1-check"),
        );
        import(
            &candidate,
            &evidence(&snapshot, EvidenceLane::LocalHost, "make-test-integration"),
        );
        (candidate, snapshot)
    }

    #[test]
    fn a_snapshot_with_passing_lanes_seals() {
        let scratch = Scratch::new("seal-ok");
        let (candidate, snapshot) = sealable(&scratch);

        let output = seal(&candidate, &snapshot).expect("seal");
        assert_eq!(output.operation.as_str(), "seal");
        assert_eq!(
            output.artifact.as_deref(),
            Some(format!("W0/{}/seal.json", snapshot.candidate_id.as_str()).as_str()),
            "the artifact must be a state-root-relative reference, not an absolute path"
        );

        let record: SealRecord = candidate.read_json(SEAL_FILE).expect("seal record");
        record.validate(&candidate).expect("sealed record is valid");
        assert_eq!(record.candidate_id, snapshot.candidate_id);
        let serialized = serde_json::to_value(&record).expect("seal JSON");
        assert!(
            serialized.get("panel").is_none(),
            "the seal must not carry panel state"
        );
        assert_eq!(record.evidence.len(), 2);
    }

    #[test]
    fn seal_command_rejects_stale_planned_work_item_state() {
        let repository = GitFixture::new("seal-planned-state-repository");
        repository.write(
            "docs/specs/ADR-046-implementation-graph.json",
            r#"{"nodes":[{"id":"ADR046-foundation-001","kind":"work-item","wave":"W0"}]}"#,
        );
        repository.write(
            "docs/specs/ADR-046-work-items.json",
            r#"{"items":[{"workItemId":"ADR046-foundation-001","implementationState":"Planned"}]}"#,
        );
        repository.commit("planned work-item state");
        let material = take(&repository).material;

        let scratch = Scratch::new("seal-planned-state");
        let (candidate, snapshot) = sealable_from(&scratch, material);
        let state = StateRoot::for_tests(&scratch.path.join("state")).expect("state root");
        let roots = BTreeMap::from([("github.com/example/d2b".to_string(), repository.repo())]);
        let error = seal_checked(&state, &candidate, &snapshot, &roots)
            .expect_err("a Planned item must block the seal command");
        assert!(error.message().contains("cannot seal W0"), "{error}");
        assert!(error.message().contains("ADR046-foundation-001"), "{error}");
        assert!(
            error.message().contains("Implementation state to Merged"),
            "{error}"
        );
    }

    /// FR-049: the seal boundary carries the predecessor-merged condition in
    /// its own right. This wave's own
    /// items are all `Merged`, so the current-wave leg passes and cannot be
    /// what refuses; only the prior-wave leg can produce the error below.
    #[test]
    fn seal_command_rejects_an_unmerged_prior_wave_item() {
        let repository = GitFixture::new("seal-prior-wave-repository");
        repository.write(
            "docs/specs/ADR-046-implementation-graph.json",
            "{\"nodes\":[\
             {\"id\":\"ADR046-foundation-001\",\"kind\":\"work-item\",\"wave\":\"W0\"},\
             {\"id\":\"ADR046-backend-001\",\"kind\":\"work-item\",\"wave\":\"W1\"}]}\n",
        );
        repository.write(
            "docs/specs/ADR-046-work-items.json",
            "{\"items\":[\
             {\"workItemId\":\"ADR046-foundation-001\",\"implementationState\":\"Planned\"},\
             {\"workItemId\":\"ADR046-backend-001\",\"implementationState\":\"Merged\"}]}\n",
        );
        repository.commit("predecessor wave still unmerged");
        let mut material = take(&repository).material;
        "W1".clone_into(&mut material.wave);

        let scratch = Scratch::new("seal-prior-wave");
        let (candidate, snapshot) = sealable_from(&scratch, material);
        let state = StateRoot::for_tests(&scratch.path.join("state")).expect("state root");
        let roots = BTreeMap::from([("github.com/example/d2b".to_string(), repository.repo())]);
        let error = seal_checked(&state, &candidate, &snapshot, &roots)
            .expect_err("an unmerged prior-wave item must block the seal command");
        assert!(
            error.message().contains("cannot seal or merge W1"),
            "{error}"
        );
        assert!(error.message().contains("ADR046-foundation-001"), "{error}");
        assert!(error.message().contains("in W0 is `Planned`"), "{error}");
    }

    /// Drives the real `wave seal` entrypoint from its argument vector, so CLI
    /// parsing, state-root preparation, and snapshot resolution are covered -
    /// not just the inner `seal_checked` helper.
    ///
    /// The state root sits inside the ignored build tree, which
    /// `StateRoot::prepare` refuses in production, so the test installs the
    /// `#[cfg(test)]`-only redirection for the duration of the run. The
    /// production refusal is untouched.
    #[test]
    fn the_seal_entrypoint_runs_end_to_end_from_its_argument_vector() {
        use crate::delivery::{snapshot, storage::test_root_override};

        let repository = GitFixture::new("seal-cli-repository");
        repository.write(
            "docs/specs/ADR-046-implementation-graph.json",
            r#"{"nodes":[{"id":"ADR046-foundation-001","kind":"work-item","wave":"W0"}]}"#,
        );
        repository.write(
            "docs/specs/ADR-046-work-items.json",
            r#"{"items":[{"workItemId":"ADR046-foundation-001","implementationState":"Planned"}]}"#,
        );
        repository.commit("planned work-item state");
        let _guard = test_root_override::install(&repository.state());

        let snapshot_ref = snapshot::run(&repository.snapshot_args())
            .expect("wave snapshot")
            .artifact
            .expect("snapshot artifact reference");
        let args = vec![
            "--snapshot".to_owned(),
            snapshot_ref,
            "--repo".to_owned(),
            format!("github.com/example/d2b={}", repository.repo().display()),
        ];
        let error = run(&args).expect_err("the entrypoint must refuse a Planned work item");
        assert!(error.message().contains("cannot seal W0"), "{error}");
        assert!(error.message().contains("ADR046-foundation-001"), "{error}");
    }

    #[test]
    fn a_missing_validator_lane_is_refused() {
        let scratch = Scratch::new("seal-missing-lane");
        let (candidate, snapshot) = sealable(&scratch);
        std::fs::remove_dir_all(candidate.evidence_dir().join("local-host")).expect("remove lane");
        let error = seal(&candidate, &snapshot).expect_err("missing lane");
        assert!(error.message().contains("local-host"), "{error}");
    }

    #[test]
    fn no_imported_evidence_at_all_is_refused() {
        let scratch = Scratch::new("seal-no-evidence");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let error = seal(&candidate, &snapshot).expect_err("no evidence");
        assert!(error.message().contains("no validator evidence"), "{error}");
    }

    #[test]
    fn a_failing_lane_result_is_refused() {
        let scratch = Scratch::new("seal-failed-lane");
        let (candidate, snapshot) = sealable(&scratch);
        let mut failed = evidence(&snapshot, EvidenceLane::LocalHost, "make-test-integration");
        failed.result = EvidenceResult::Failed;
        import(&candidate, &failed);
        let error = seal(&candidate, &snapshot).expect_err("failed lane");
        assert!(error.message().contains("did not pass"), "{error}");
    }

    #[test]
    fn evidence_bound_to_a_stale_candidate_is_refused() {
        let scratch = Scratch::new("seal-stale-evidence");
        let (candidate, snapshot) = sealable(&scratch);
        let mut stale = evidence(&snapshot, EvidenceLane::GithubCi, "layer1-check");
        stale.snapshot_sha256 = SnapshotSha256::parse("c".repeat(64)).expect("digest");
        import(&candidate, &stale);
        let error = seal(&candidate, &snapshot).expect_err("stale evidence");
        assert!(error.message().contains("stale snapshot"), "{error}");
    }

    /// A history-only rebase preserves `candidate_id` and `content_id` but
    /// moves `snapshot_sha256`, so every lane result goes stale and has to be
    /// re-imported.
    #[test]
    fn a_history_only_rebase_invalidates_the_lanes() {
        let scratch = Scratch::new("seal-rebase-evidence");
        let (candidate, snapshot) = sealable(&scratch);
        seal(&candidate, &snapshot).expect("seal before the rebase");

        let rebased = crate::delivery::test_support::rebased(&snapshot);
        let error = seal(&candidate, &rebased).expect_err("lane results are stale");
        assert!(error.message().contains("stale snapshot"), "{error}");

        import(
            &candidate,
            &evidence(&rebased, EvidenceLane::GithubCi, "layer1-check"),
        );
        import(
            &candidate,
            &evidence(&rebased, EvidenceLane::LocalHost, "make-test-integration"),
        );
        let record: SealRecord = seal(&candidate, &rebased)
            .and_then(|_| candidate.read_json(SEAL_FILE))
            .expect("re-imported lanes seal the rebased snapshot");
        assert_eq!(record.snapshot_sha256, rebased.snapshot_sha256);
        assert_eq!(record.candidate_id, snapshot.candidate_id);
        assert_eq!(record.evidence.len(), 2);
    }

    /// The nested layout addresses a record by lane and validation, so
    /// re-importing the same lane result overwrites in place rather than
    /// leaving two rival records the seal cannot choose between. A seal after
    /// a re-import still binds exactly the two required lanes.
    #[test]
    fn re_importing_a_lane_result_is_idempotent() {
        let scratch = Scratch::new("seal-idempotent-import");
        let (candidate, snapshot) = sealable(&scratch);
        import(
            &candidate,
            &evidence(&snapshot, EvidenceLane::GithubCi, "layer1-check"),
        );
        let output = seal(&candidate, &snapshot).expect("seal after a re-import");
        assert_eq!(output.operation.as_str(), "seal");
        let record: SealRecord = candidate.read_json(SEAL_FILE).expect("seal record");
        assert_eq!(record.evidence.len(), 2);
    }

    #[test]
    fn a_stray_file_in_the_evidence_directory_is_refused() {
        let scratch = Scratch::new("seal-stray-evidence");
        let (candidate, snapshot) = sealable(&scratch);
        std::fs::write(candidate.evidence_dir().join("output.log"), b"raw output")
            .expect("stray file");
        let error = seal(&candidate, &snapshot).expect_err("stray file");
        assert!(
            error.message().contains("not an evidence record"),
            "{error}"
        );
    }

    #[test]
    fn a_seal_with_forged_digests_is_refused_on_read_back() {
        let scratch = Scratch::new("seal-forged");
        let (candidate, snapshot) = sealable(&scratch);
        seal(&candidate, &snapshot).expect("seal");
        let mut record: SealRecord = candidate.read_json(SEAL_FILE).expect("seal record");
        record.content_id = ContentId::parse("d".repeat(64)).expect("digest");
        let error = record.validate(&candidate).expect_err("forged seal");
        assert!(error.message().contains("re-derive"), "{error}");
    }
}
