//! Wave seal construction (spec section 12.4, work item
//! `ADR046-delivery-006`).
//!
//! `seal` requires all ten panel records present, unanimous, and bound to the
//! same `candidate_id`, `content_id`, and `snapshot_sha256`, plus every
//! spec-section-12.2 validator lane reporting success on that exact snapshot.
//! It writes one sealed record binding the wave's candidate triple, the
//! attested panel records, and the lanes it accepted.
//!
//! The panel lane's success is the ten unanimous records themselves, which
//! this stage verifies directly, so a lane record for the panel is accepted
//! but never a substitute for the records. The two validator lanes — required
//! GitHub CI and the heavy-gated local/host validators — must each carry at
//! least one imported result, and every imported result must be a pass.
//! [`EvidenceResult`] has no pending state by construction: a pending lane is
//! an absent record, and an absent lane fails the seal.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{WaveCommand, WorkflowOutput},
    model::{
        CandidateId, CandidateMaterial, ContentId, EVIDENCE_ARTIFACT_KIND, EvidenceResult,
        SEAL_ARTIFACT_KIND, SnapshotSha256, sha256_bytes, validate_bounded_string,
        validate_identifier, validate_sha256,
    },
    panel::{
        self, PanelAttestation, SnapshotView, ensure_artifact_kind, parse_snapshot_invocation,
    },
    storage::{CandidateDir, EVIDENCE_DIR, PANEL_REQUEST_FILE, SEAL_FILE},
};

/// The three concurrent lanes of spec section 12.2.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceLane {
    /// The required GitHub Actions Layer-1 rollup.
    GithubCi,
    /// The heavy-gated local and host validators run by the integrator.
    LocalHost,
    /// The ten-role panel. Its authoritative evidence is the record set.
    Panel,
}

impl EvidenceLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GithubCi => "github-ci",
            Self::LocalHost => "local-host",
            Self::Panel => "panel",
        }
    }
}

/// Lanes that must carry at least one passing imported result before a seal.
pub const REQUIRED_EVIDENCE_LANES: [EvidenceLane; 2] =
    [EvidenceLane::GithubCi, EvidenceLane::LocalHost];

/// Reader view of one validator-evidence record.
///
/// The writer is `wave validate-import`, work item `ADR046-delivery-003`, so
/// this view reads the fields the seal is specified to depend on and tolerates
/// fields it does not know. It carries a digest of the validator's output, not
/// the output: raw command output never leaves the importing lane.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceRecord {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub lane: EvidenceLane,
    pub validation: String,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub snapshot_sha256: SnapshotSha256,
    pub result: EvidenceResult,
}

impl EvidenceRecord {
    fn validate(&self, snapshot: &SnapshotView) -> Result<()> {
        ensure_artifact_kind(
            &self.artifact_kind,
            EVIDENCE_ARTIFACT_KIND,
            "validator evidence",
        )?;
        if self.schema_version != DELIVERY_SCHEMA_VERSION {
            return Err(DeliveryError::new(format!(
                "unsupported validator evidence schema version {}",
                self.schema_version
            )));
        }
        validate_bounded_string(&self.validation, "validation name")?;
        if self.candidate_id != snapshot.candidate_id
            || self.content_id != snapshot.content_id
            || self.snapshot_sha256 != snapshot.snapshot_sha256
        {
            return Err(DeliveryError::new(format!(
                "validator evidence for {:?} is bound to a stale candidate; every lane reruns \
                 against the current snapshot",
                self.validation
            )));
        }
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

/// One lane's accepted validations, as bound into the seal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SealedLane {
    pub lane: EvidenceLane,
    pub validations: Vec<String>,
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
    pub panel: PanelAttestation,
    pub panel_request_sha256: String,
    pub evidence: Vec<SealedLane>,
}

impl SealRecord {
    /// Re-validates a seal read back from delivery state.
    pub fn validate(&self) -> Result<()> {
        ensure_artifact_kind(&self.artifact_kind, SEAL_ARTIFACT_KIND, "wave seal")?;
        if self.schema_version != DELIVERY_SCHEMA_VERSION {
            return Err(DeliveryError::new(format!(
                "unsupported wave seal schema version {}",
                self.schema_version
            )));
        }
        validate_identifier(&self.program, "program")?;
        validate_identifier(&self.wave, "wave")?;
        validate_sha256(&self.panel_request_sha256, "panel request digest")?;
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
        self.panel.validate()?;
        ensure_required_lanes(&self.evidence.iter().map(|lane| lane.lane).collect())
    }
}

/// `cargo xtask delivery wave seal`.
pub fn run(args: &[String]) -> Result<WorkflowOutput> {
    let (state, snapshot_path) = parse_snapshot_invocation(args)?;
    let (candidate, snapshot) = panel::open_candidate(&state, &snapshot_path)?;
    seal(&candidate, &snapshot)
}

/// Binds unanimous panel records and passing validator lanes to one candidate.
pub fn seal(candidate: &CandidateDir, snapshot: &SnapshotView) -> Result<WorkflowOutput> {
    let request = panel::stored_request(candidate, snapshot)?;
    let attestation = panel::attested_records(candidate, &request)?;
    let evidence = sealed_lanes(candidate, snapshot)?;
    let panel_request_sha256 = sha256_bytes(&candidate.read_bytes(PANEL_REQUEST_FILE)?);

    let record = SealRecord {
        artifact_kind: SEAL_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: snapshot.program().to_owned(),
        wave: snapshot.wave().to_owned(),
        candidate_id: snapshot.candidate_id.clone(),
        content_id: snapshot.content_id.clone(),
        snapshot_sha256: snapshot.snapshot_sha256.clone(),
        material: snapshot.material.clone(),
        panel: attestation,
        panel_request_sha256,
        evidence,
    };
    record.validate()?;
    candidate.write_json(SEAL_FILE, &record)?;

    WorkflowOutput::ok(WaveCommand::Seal)
        .with_digests(&snapshot.digests())
        .with_artifact(&candidate.seal_path())
}

/// Reads every imported evidence record and groups the passing ones by lane.
fn sealed_lanes(candidate: &CandidateDir, snapshot: &SnapshotView) -> Result<Vec<SealedLane>> {
    let names = candidate.list(EVIDENCE_DIR).map_err(|error| {
        DeliveryError::new(format!(
            "no validator evidence for this candidate; import every lane's result first \
             ({error})"
        ))
    })?;
    let mut lanes: BTreeMap<EvidenceLane, BTreeSet<String>> = BTreeMap::new();
    for name in names {
        let name = name
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| DeliveryError::new("validator evidence file name is not UTF-8"))?;
        if !name.ends_with(".json") {
            return Err(DeliveryError::new(format!(
                "validator evidence directory holds {name:?}, which is not an evidence record"
            )));
        }
        let record: EvidenceRecord = serde_json::from_slice(
            &candidate.read_bytes(std::path::Path::new(EVIDENCE_DIR).join(&name))?,
        )
        .map_err(|error| {
            DeliveryError::new(format!(
                "validator evidence {name:?} is not a valid record: {error}"
            ))
        })?;
        record.validate(snapshot)?;
        if !lanes
            .entry(record.lane)
            .or_default()
            .insert(record.validation.clone())
        {
            return Err(DeliveryError::new(format!(
                "lane {} imported two results for validation {:?}; the seal cannot tell which \
                 one is current",
                record.lane.as_str(),
                record.validation
            )));
        }
    }
    ensure_required_lanes(&lanes.keys().copied().collect())?;
    Ok(lanes
        .into_iter()
        .map(|(lane, validations)| SealedLane {
            lane,
            validations: validations.into_iter().collect(),
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
        model::PANEL_ROLES,
        panel::tests::{candidate_with_snapshot, record_files, write_record_dir},
        storage::tests::Scratch,
    };
    use std::path::Path;

    pub(crate) fn evidence(
        snapshot: &SnapshotView,
        lane: EvidenceLane,
        validation: &str,
    ) -> EvidenceRecord {
        EvidenceRecord {
            artifact_kind: EVIDENCE_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            lane,
            validation: validation.to_owned(),
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            result: EvidenceResult::Passed,
        }
    }

    pub(crate) fn import(candidate: &CandidateDir, name: &str, record: &EvidenceRecord) {
        candidate
            .write_json(Path::new(EVIDENCE_DIR).join(name), record)
            .expect("import evidence");
    }

    /// A candidate with a request, ten unanimous records, and both required
    /// lanes imported.
    pub(crate) fn sealable(scratch: &Scratch) -> (CandidateDir, SnapshotView) {
        let (_state, candidate, snapshot) = candidate_with_snapshot(scratch);
        panel::request(&candidate, &snapshot).expect("panel request");
        let files = record_files(&snapshot);
        let dir = write_record_dir(scratch, &files);
        panel::attest(&candidate, &snapshot, &dir).expect("attest");
        import(
            &candidate,
            "github-ci.json",
            &evidence(&snapshot, EvidenceLane::GithubCi, "layer1-check"),
        );
        import(
            &candidate,
            "local-host.json",
            &evidence(&snapshot, EvidenceLane::LocalHost, "make-test-integration"),
        );
        (candidate, snapshot)
    }

    #[test]
    fn a_unanimous_panel_and_passing_lanes_seal() {
        let scratch = Scratch::new("seal-ok");
        let (candidate, snapshot) = sealable(&scratch);

        let output = seal(&candidate, &snapshot).expect("seal");
        assert_eq!(output.operation, "seal");
        assert_eq!(output.artifact.as_deref(), candidate.seal_path().to_str());

        let record: SealRecord = candidate.read_json(SEAL_FILE).expect("seal record");
        record.validate().expect("sealed record is valid");
        assert_eq!(record.candidate_id, snapshot.candidate_id);
        assert_eq!(record.panel.records.len(), PANEL_ROLES.len());
        assert!(record.panel.unanimous);
        assert_eq!(record.evidence.len(), 2);
    }

    #[test]
    fn a_seal_without_panel_records_is_refused() {
        let scratch = Scratch::new("seal-no-panel");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        panel::request(&candidate, &snapshot).expect("panel request");
        import(
            &candidate,
            "github-ci.json",
            &evidence(&snapshot, EvidenceLane::GithubCi, "layer1-check"),
        );
        let error = seal(&candidate, &snapshot).expect_err("no records");
        assert!(error.message().contains("panel-attest"), "{error}");
    }

    #[test]
    fn a_seal_without_a_panel_request_is_refused() {
        let scratch = Scratch::new("seal-no-request");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let error = seal(&candidate, &snapshot).expect_err("no request");
        assert!(error.message().contains("panel-request"), "{error}");
    }

    #[test]
    fn a_missing_panel_record_is_refused() {
        let scratch = Scratch::new("seal-missing-record");
        let (candidate, snapshot) = sealable(&scratch);
        std::fs::remove_file(candidate.panel_dir().join("kernel.json")).expect("remove record");
        let error = seal(&candidate, &snapshot).expect_err("missing record");
        assert!(error.message().contains("exactly 10 records"), "{error}");
    }

    #[test]
    fn a_panel_record_bound_to_another_candidate_is_refused() {
        let scratch = Scratch::new("seal-stale-record");
        let (candidate, snapshot) = sealable(&scratch);
        let mut stale = crate::delivery::panel::tests::record(
            crate::delivery::model::PanelRole::Docs,
            &snapshot,
        );
        stale.candidate_id = CandidateId::parse("b".repeat(64)).expect("digest");
        candidate
            .write_json(Path::new("panel").join("docs.json"), &stale)
            .expect("overwrite record");
        let error = seal(&candidate, &snapshot).expect_err("stale record");
        assert!(error.message().contains("different candidate"), "{error}");
    }

    #[test]
    fn a_missing_validator_lane_is_refused() {
        let scratch = Scratch::new("seal-missing-lane");
        let (candidate, snapshot) = sealable(&scratch);
        std::fs::remove_file(candidate.evidence_dir().join("local-host.json")).expect("remove");
        let error = seal(&candidate, &snapshot).expect_err("missing lane");
        assert!(error.message().contains("local-host"), "{error}");
    }

    #[test]
    fn no_imported_evidence_at_all_is_refused() {
        let scratch = Scratch::new("seal-no-evidence");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        panel::request(&candidate, &snapshot).expect("panel request");
        let dir = write_record_dir(&scratch, &record_files(&snapshot));
        panel::attest(&candidate, &snapshot, &dir).expect("attest");
        let error = seal(&candidate, &snapshot).expect_err("no evidence");
        assert!(error.message().contains("no validator evidence"), "{error}");
    }

    #[test]
    fn a_failing_lane_result_is_refused() {
        let scratch = Scratch::new("seal-failed-lane");
        let (candidate, snapshot) = sealable(&scratch);
        let mut failed = evidence(&snapshot, EvidenceLane::LocalHost, "make-test-integration");
        failed.result = EvidenceResult::Failed;
        import(&candidate, "local-host.json", &failed);
        let error = seal(&candidate, &snapshot).expect_err("failed lane");
        assert!(error.message().contains("did not pass"), "{error}");
    }

    #[test]
    fn evidence_bound_to_a_stale_candidate_is_refused() {
        let scratch = Scratch::new("seal-stale-evidence");
        let (candidate, snapshot) = sealable(&scratch);
        let mut stale = evidence(&snapshot, EvidenceLane::GithubCi, "layer1-check");
        stale.snapshot_sha256 = SnapshotSha256::parse("c".repeat(64)).expect("digest");
        import(&candidate, "github-ci.json", &stale);
        let error = seal(&candidate, &snapshot).expect_err("stale evidence");
        assert!(error.message().contains("stale candidate"), "{error}");
    }

    #[test]
    fn a_duplicate_validation_within_one_lane_is_refused() {
        let scratch = Scratch::new("seal-duplicate-validation");
        let (candidate, snapshot) = sealable(&scratch);
        import(
            &candidate,
            "github-ci-again.json",
            &evidence(&snapshot, EvidenceLane::GithubCi, "layer1-check"),
        );
        let error = seal(&candidate, &snapshot).expect_err("duplicate validation");
        assert!(error.message().contains("two results"), "{error}");
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
        let error = record.validate().expect_err("forged seal");
        assert!(error.message().contains("re-derive"), "{error}");
    }

    #[test]
    fn the_panel_lane_may_be_imported_but_never_replaces_the_records() {
        let scratch = Scratch::new("seal-panel-lane");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        panel::request(&candidate, &snapshot).expect("panel request");
        for (name, lane, validation) in [
            ("github-ci.json", EvidenceLane::GithubCi, "layer1-check"),
            (
                "local-host.json",
                EvidenceLane::LocalHost,
                "make-test-integration",
            ),
            ("panel.json", EvidenceLane::Panel, "ten-role-panel"),
        ] {
            import(&candidate, name, &evidence(&snapshot, lane, validation));
        }
        let error = seal(&candidate, &snapshot).expect_err("no records");
        assert!(error.message().contains("panel-attest"), "{error}");
    }
}
