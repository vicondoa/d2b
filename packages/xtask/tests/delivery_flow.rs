#![forbid(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use xtask::delivery::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result, atomic_merge, check_history_merge_eligibility,
    check_merge_eligibility,
    command::{
        GitProbe, ObservedCheck, ObservedCheckState, ProcessCommandOutput, PullRequestMerger,
        PullRequestStatus, PullRequestStatusSource, RepositoryProbe, StackGraphSource, TrackedBlob,
    },
    construct_history_proof, construct_seal, create_snapshot,
    evidence::{CiAttestationClaims, CiAttestationVerifier, EvidenceRecord, VerifiedCiAttestation},
    import_ci_evidence,
    model::{
        CheckPublisher, CheckPublisherKind, DeliveryManifest, FingerprintSpec, GitObjectFormat,
        LogicalPath, PANEL_ATTESTATION_ARTIFACT_KIND, PANEL_MODEL_POLICY, PANEL_PROVIDER_POLICY,
        PANEL_ROLES, PullRequestState, RepositoryPolicy, RequiredCheck, RequiredValidation,
        SnapshotRequest, StackBranch, StackGraph, StackNodePolicy, StackPr, ValidationAuthority,
    },
    panel::{PanelAttestation, PanelReceiptVerifier, VerifiedPanelReceipt},
    read_snapshot, run_validation,
    seal::HistoryProof,
    storage::{sha256_bytes, sha256_file, verify_json_digest, write_immutable_json},
    validate_and_store_panel, verify_seal,
};

const REPOSITORY_ID: &str = "github.com/example/d2b";
static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(1);
static NEXT_CHECK_RUN: AtomicU64 = AtomicU64::new(1);

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root")
            .to_path_buf();
        let path = repository
            .parent()
            .expect("repository parent")
            .join(format!(
                ".d2b-delivery-{label}-{}-{}",
                std::process::id(),
                NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&path).expect("create external scratch");
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone)]
struct StaticGraph {
    graph: StackGraph,
}

impl StackGraphSource for StaticGraph {
    fn graph(
        &self,
        repository: &str,
        _checkout_root: &Path,
        _expected_nodes: &[StackNodePolicy],
    ) -> Result<StackGraph> {
        if repository != REPOSITORY_ID {
            return Err(DeliveryError::new("unexpected graph repository"));
        }
        Ok(self.graph.clone())
    }
}

struct StaticStatus {
    status: PullRequestStatus,
}

impl PullRequestStatusSource for StaticStatus {
    fn status(&self, repository: &str, pr: u64) -> Result<PullRequestStatus> {
        if repository != self.status.repository || pr != self.status.number {
            return Err(DeliveryError::new("unexpected PR query"));
        }
        Ok(self.status.clone())
    }
}

struct StatusMap {
    statuses: BTreeMap<u64, PullRequestStatus>,
}

impl PullRequestStatusSource for StatusMap {
    fn status(&self, repository: &str, pr: u64) -> Result<PullRequestStatus> {
        let status = self
            .statuses
            .get(&pr)
            .ok_or_else(|| DeliveryError::new("unexpected PR query"))?;
        if status.repository != repository {
            return Err(DeliveryError::new("unexpected PR repository"));
        }
        Ok(status.clone())
    }
}

struct SequenceStatus {
    statuses: RefCell<VecDeque<PullRequestStatus>>,
    fallback: PullRequestStatus,
}

impl PullRequestStatusSource for SequenceStatus {
    fn status(&self, _repository: &str, _pr: u64) -> Result<PullRequestStatus> {
        Ok(self
            .statuses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| self.fallback.clone()))
    }
}

struct RecordingMerger {
    calls: Cell<usize>,
}

struct FixturePanelVerifier;

impl PanelReceiptVerifier for FixturePanelVerifier {
    fn verify(
        &self,
        receipt_path: &Path,
        signature_path: &Path,
        trust_root_path: &Path,
    ) -> Result<VerifiedPanelReceipt> {
        let receipt_sha256 = sha256_file(receipt_path)?;
        let signature = fs::read_to_string(signature_path)?;
        if signature != format!("fixture-signature:{receipt_sha256}\n") {
            return Err(DeliveryError::new(
                "panel receipt detached-signature verification failed",
            ));
        }
        if fs::read_to_string(trust_root_path)? != "fixture-panel-trust-root\n" {
            return Err(DeliveryError::new("panel trust root is not authoritative"));
        }
        let claims = serde_json::from_slice(&fs::read(receipt_path)?)?;
        Ok(VerifiedPanelReceipt {
            claims,
            receipt_sha256,
            signature_sha256: sha256_file(signature_path)?,
            trust_root_sha256: sha256_file(trust_root_path)?,
        })
    }
}

struct MutatingPanelVerifier {
    mutated: Cell<bool>,
}

impl PanelReceiptVerifier for MutatingPanelVerifier {
    fn verify(
        &self,
        receipt_path: &Path,
        signature_path: &Path,
        trust_root_path: &Path,
    ) -> Result<VerifiedPanelReceipt> {
        let verified =
            FixturePanelVerifier.verify(receipt_path, signature_path, trust_root_path)?;
        if !self.mutated.replace(true) {
            let mut bytes = fs::read(receipt_path)?;
            bytes.push(b'\n');
            fs::write(receipt_path, bytes)?;
        }
        Ok(verified)
    }
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

struct Fixture {
    scratch: Scratch,
    repository: PathBuf,
    roots: BTreeMap<String, PathBuf>,
    state: PathBuf,
    graph: StaticGraph,
    status: PullRequestStatus,
    request: SnapshotRequest,
}

impl Fixture {
    fn new(label: &str, validation_authority: ValidationAuthority) -> Self {
        let scratch = Scratch::new(label);
        let repository = scratch.path.join("repository");
        fs::create_dir(&repository).expect("repository");
        git(&repository, &["init", "--object-format=sha1", "-b", "main"]);
        git(&repository, &["config", "user.name", "Example"]);
        git(
            &repository,
            &["config", "user.email", "example@example.invalid"],
        );
        git(
            &repository,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/example/d2b.git",
            ],
        );
        fs::write(repository.join("contract.json"), b"{\"version\":1}\n").expect("base contract");
        fs::write(repository.join("dependencies.txt"), b"none\n").expect("dependencies");
        git(&repository, &["add", "contract.json", "dependencies.txt"]);
        git(&repository, &["commit", "-m", "base"]);
        let probe = GitProbe::new(ProcessCommandOutput);
        let base = probe.resolve_commit(&repository, "main").expect("base OID");

        git(&repository, &["checkout", "-b", "feature"]);
        fs::write(repository.join("contract.json"), b"{\"version\":2}\n")
            .expect("feature contract");
        let manifest = manifest(validation_authority);
        fs::create_dir(repository.join("delivery")).expect("delivery directory");
        fs::write(
            repository.join("delivery/manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("manifest JSON"),
        )
        .expect("delivery manifest");
        git(
            &repository,
            &["add", "contract.json", "delivery/manifest.json"],
        );
        git(&repository, &["commit", "-m", "feature"]);
        let head = probe
            .resolve_commit(&repository, "feature")
            .expect("head OID");

        let graph = StaticGraph {
            graph: graph(&base, &head),
        };
        let status = status(&base, &head);
        let roots = BTreeMap::from([(REPOSITORY_ID.to_owned(), repository.clone())]);
        let state = scratch.path.join("state");
        let request = SnapshotRequest {
            authority_repository: REPOSITORY_ID.to_owned(),
            authority_ref: "feature".to_owned(),
            manifest_path: PathBuf::from("delivery/manifest.json"),
            repository_roots: roots.clone(),
            state_root: Some(state.clone()),
        };
        Self {
            scratch,
            repository,
            roots,
            state,
            graph,
            status,
            request,
        }
    }

    fn snapshot(&self) -> PathBuf {
        let probe = GitProbe::new(ProcessCommandOutput);
        create_snapshot(
            &probe,
            &self.graph,
            &StaticStatus {
                status: self.status.clone(),
            },
            &self.request,
        )
        .expect("create snapshot")
    }

    fn panel_source(&self, snapshot_path: &Path) -> (PathBuf, PathBuf) {
        panel_source_at(&self.scratch.path, snapshot_path)
    }

    fn seal(&self, snapshot: &Path) -> PathBuf {
        let probe = GitProbe::new(ProcessCommandOutput);
        run_validation(&probe, &ProcessCommandOutput, &self.roots, snapshot, "unit")
            .expect("validation evidence");
        let (panel, trust_root) = self.panel_source(snapshot);
        validate_and_store_panel(
            &probe,
            &FixturePanelVerifier,
            &self.roots,
            snapshot,
            &panel,
            &trust_root,
        )
        .expect("panel");
        construct_seal(
            &probe,
            &StaticStatus {
                status: self.status.clone(),
            },
            &RejectVerifier,
            &FixturePanelVerifier,
            &self.roots,
            snapshot,
        )
        .expect("seal")
    }

    fn refresh_head(&mut self) {
        let probe = GitProbe::new(ProcessCommandOutput);
        let head = probe
            .resolve_commit(&self.repository, "feature")
            .expect("updated head");
        let base = self.status.base_oid.clone();
        self.graph = StaticGraph {
            graph: graph(&base, &head),
        };
        self.status = status(&base, &head);
    }
}

fn manifest(authority: ValidationAuthority) -> DeliveryManifest {
    DeliveryManifest {
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: "adr0045".to_owned(),
        wave: "w1".to_owned(),
        authority_repository: REPOSITORY_ID.to_owned(),
        panel_trust_root_sha256: sha256_bytes(b"fixture-panel-trust-root\n"),
        repositories: vec![RepositoryPolicy {
            id: REPOSITORY_ID.to_owned(),
            object_format: GitObjectFormat::Sha1,
            trunk_ref: "main".to_owned(),
            integration_ref: "feature".to_owned(),
        }],
        stack_nodes: vec![StackNodePolicy {
            id: "xtask".to_owned(),
            repository: REPOSITORY_ID.to_owned(),
            branch: "feature".to_owned(),
            pr_number: 42,
            external_dependencies: vec![],
        }],
        required_validations: vec![RequiredValidation {
            id: "unit".to_owned(),
            argv: vec!["sh".to_owned(), "-c".to_owned(), "exit 0".to_owned()],
            cwd: LogicalPath {
                repository: REPOSITORY_ID.to_owned(),
                path: ".".to_owned(),
            },
            authority,
            ci_publisher: (authority == ValidationAuthority::GithubAttestation).then(publisher),
            ci_signer_workflow: (authority == ValidationAuthority::GithubAttestation)
                .then(|| "github.com/example/d2b/.github/workflows/layer1.yml".to_owned()),
            timeout_seconds: 10,
        }],
        required_checks: vec![RequiredCheck {
            node: "xtask".to_owned(),
            name: "check".to_owned(),
            publisher: publisher(),
        }],
        generated_artifacts: vec![],
        dependency_fingerprints: vec![FingerprintSpec {
            name: "dependencies".to_owned(),
            repository: REPOSITORY_ID.to_owned(),
            path: "dependencies.txt".to_owned(),
        }],
        contract_fingerprints: vec![
            FingerprintSpec {
                name: "contract".to_owned(),
                repository: REPOSITORY_ID.to_owned(),
                path: "contract.json".to_owned(),
            },
            FingerprintSpec {
                name: "delivery-authority".to_owned(),
                repository: REPOSITORY_ID.to_owned(),
                path: "delivery/manifest.json".to_owned(),
            },
        ],
    }
}

fn graph(base: &str, head: &str) -> StackGraph {
    StackGraph {
        trunk: "main".to_owned(),
        current_branch: "feature".to_owned(),
        branches: vec![StackBranch {
            name: "feature".to_owned(),
            parent: "main".to_owned(),
            base_ref: "main".to_owned(),
            observed_base: base.to_owned(),
            head: head.to_owned(),
            base: base.to_owned(),
            is_current: true,
            is_merged: false,
            is_queued: false,
            needs_rebase: false,
            pr: Some(StackPr {
                number: 42,
                url: "https://github.com/example/d2b/pull/42".to_owned(),
                state: "OPEN".to_owned(),
            }),
            merge_commit_oid: None,
            merge_commit_tree_oid: None,
        }],
    }
}

fn publisher() -> CheckPublisher {
    CheckPublisher {
        kind: CheckPublisherKind::CheckRun,
        app_slug: "github-actions".to_owned(),
        app_id: 15368,
        workflow: "Layer 1".to_owned(),
        workflow_id: 321,
    }
}

fn status(base: &str, head: &str) -> PullRequestStatus {
    status_for(42, PullRequestState::Open, "main", base, "feature", head)
}

#[test]
fn per_wave_manifest_is_selected_fingerprinted_authority() {
    let mut fixture = Fixture::new("per-wave-manifest", ValidationAuthority::LocalRunner);
    let legacy = fixture.repository.join("delivery/manifest.json");
    let selected = fixture.repository.join("delivery/manifests/w1.json");
    let mut manifest: DeliveryManifest =
        serde_json::from_slice(&fs::read(&legacy).expect("legacy manifest"))
            .expect("manifest JSON");
    manifest
        .contract_fingerprints
        .iter_mut()
        .find(|fingerprint| fingerprint.name == "delivery-authority")
        .expect("delivery authority fingerprint")
        .path = "delivery/manifests/w1.json".to_owned();
    fs::create_dir(fixture.repository.join("delivery/manifests")).expect("manifest directory");
    write_source_json(&selected, &manifest);
    fs::remove_file(&legacy).expect("remove legacy manifest");
    git(
        &fixture.repository,
        &[
            "add",
            "delivery/manifest.json",
            "delivery/manifests/w1.json",
        ],
    );
    git(
        &fixture.repository,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--amend",
            "--no-edit",
        ],
    );
    fixture.request.manifest_path = PathBuf::from("delivery/manifests/w1.json");
    fixture.refresh_head();

    let snapshot = read_snapshot(&fixture.snapshot()).expect("snapshot");
    assert_eq!(
        snapshot.authority.manifest_path,
        "delivery/manifests/w1.json"
    );
    assert_eq!(
        snapshot
            .contract_fingerprints
            .iter()
            .find(|fingerprint| fingerprint.name == "contract")
            .expect("contract fingerprint")
            .path,
        "contract.json"
    );
    assert!(
        snapshot
            .contract_fingerprints
            .iter()
            .any(|fingerprint| fingerprint.path == "delivery/manifests/w1.json")
    );
}

#[test]
fn duplicate_checked_in_authority_for_one_wave_is_rejected() {
    let mut fixture = Fixture::new("duplicate-wave-authority", ValidationAuthority::LocalRunner);
    let legacy = fixture.repository.join("delivery/manifest.json");
    let duplicate = fixture.repository.join("delivery/manifests/w1.json");
    fs::create_dir(fixture.repository.join("delivery/manifests")).expect("manifest directory");
    fs::copy(&legacy, &duplicate).expect("duplicate manifest");
    git(&fixture.repository, &["add", "delivery/manifests/w1.json"]);
    git(
        &fixture.repository,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--amend",
            "--no-edit",
        ],
    );
    fixture.refresh_head();

    let error = create_snapshot(
        &GitProbe::new(ProcessCommandOutput),
        &fixture.graph,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &fixture.request,
    )
    .expect_err("duplicate authority");
    assert!(
        error
            .to_string()
            .contains("duplicate delivery authority for wave w1")
    );
}

#[test]
fn per_wave_selection_preserves_exact_git_town_graph_validation() {
    let mut fixture = Fixture::new("per-wave-graph", ValidationAuthority::LocalRunner);
    let legacy = fixture.repository.join("delivery/manifest.json");
    let selected = fixture.repository.join("delivery/manifests/w1.json");
    let mut manifest: DeliveryManifest =
        serde_json::from_slice(&fs::read(&legacy).expect("legacy manifest"))
            .expect("manifest JSON");
    manifest
        .contract_fingerprints
        .iter_mut()
        .find(|fingerprint| fingerprint.name == "delivery-authority")
        .expect("delivery authority fingerprint")
        .path = "delivery/manifests/w1.json".to_owned();
    fs::create_dir(fixture.repository.join("delivery/manifests")).expect("manifest directory");
    write_source_json(&selected, &manifest);
    fs::remove_file(&legacy).expect("remove legacy manifest");
    git(
        &fixture.repository,
        &[
            "add",
            "delivery/manifest.json",
            "delivery/manifests/w1.json",
        ],
    );
    git(
        &fixture.repository,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--amend",
            "--no-edit",
        ],
    );
    fixture.request.manifest_path = PathBuf::from("delivery/manifests/w1.json");
    fixture.refresh_head();
    fixture.graph.graph.branches[0].parent = "wrong-parent".to_owned();

    let error = create_snapshot(
        &GitProbe::new(ProcessCommandOutput),
        &fixture.graph,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &fixture.request,
    )
    .expect_err("wrong Git Town parent");
    assert!(error.to_string().contains("parent topology"));
}

fn status_for(
    number: u64,
    state: PullRequestState,
    base_ref: &str,
    base: &str,
    head_ref: &str,
    head: &str,
) -> PullRequestStatus {
    let sequence = NEXT_CHECK_RUN.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_secs()
        .saturating_add(sequence);
    PullRequestStatus {
        repository: REPOSITORY_ID.to_owned(),
        number,
        state,
        merge_state: "CLEAN".to_owned(),
        base_ref: base_ref.to_owned(),
        base_oid: base.to_owned(),
        head_repository: REPOSITORY_ID.to_owned(),
        head_ref: head_ref.to_owned(),
        head_oid: head.to_owned(),
        merge_commit_oid: None,
        merge_commit_tree_oid: None,
        merge_base_oid: (state == PullRequestState::Merged).then(|| base.to_owned()),
        is_in_merge_queue: false,
        is_merge_queue_enabled: false,
        merge_queue_entry: None,
        checks: vec![ObservedCheck {
            name: "check".to_owned(),
            publisher: publisher(),
            check_run_id: Some(sequence.saturating_mul(2)),
            workflow_run_id: Some(sequence.saturating_mul(2).saturating_add(1)),
            status: "COMPLETED".to_owned(),
            conclusion: "SUCCESS".to_owned(),
            state: ObservedCheckState::Successful,
            commit_oid: head.to_owned(),
            started_at_unix_seconds: now,
            completed_at_unix_seconds: Some(now),
            workflow_created_at_unix_seconds: Some(now),
            workflow_updated_at_unix_seconds: Some(now),
        }],
    }
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run git");
    assert!(status.success(), "git command failed: {args:?}");
}

fn write_source_json(path: &Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("JSON")).expect("write JSON");
}

fn panel_source_at(root: &Path, snapshot_path: &Path) -> (PathBuf, PathBuf) {
    let snapshot = read_snapshot(snapshot_path).expect("snapshot");
    let snapshot_sha256 = verify_json_digest(snapshot_path).expect("snapshot digest");
    let records = root.join(format!(
        "panel-source-{}",
        NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&records).expect("panel source");
    let trust_root = root.join(format!(
        "panel-trust-root-{}.pem",
        NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&trust_root, b"fixture-panel-trust-root\n").expect("trust root");
    for (index, role) in PANEL_ROLES.iter().enumerate() {
        let record = PanelAttestation {
            artifact_kind: PANEL_ATTESTATION_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            role: *role,
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot_sha256.clone(),
            model_version: PANEL_MODEL_POLICY.to_owned(),
            provider: PANEL_PROVIDER_POLICY.to_owned(),
            run_id: format!("run-{index}"),
            receipt_locator: format!("github-copilot://runs/run-{index}/roles/{}", role.as_str()),
            output_sha256: format!("{index:064x}"),
            signoff: true,
            recommendations: vec![],
        };
        let receipt = records.join(format!("{}.json", role.as_str()));
        write_source_json(&receipt, &record);
        sign_fixture_receipt(&receipt);
    }
    (records, trust_root)
}

fn sign_fixture_receipt(path: &Path) {
    fs::write(
        path.with_extension("sig"),
        format!(
            "fixture-signature:{}\n",
            sha256_file(path).expect("receipt digest")
        ),
    )
    .expect("signature");
}

#[test]
fn complete_flow_is_portable_private_and_has_no_checkout_paths() {
    let fixture = Fixture::new("complete", ValidationAuthority::LocalRunner);
    let snapshot_path = fixture.snapshot();
    let snapshot = read_snapshot(&snapshot_path).expect("snapshot");
    let seal_path = fixture.seal(&snapshot_path);
    let evidence_json = fs::read_to_string(
        snapshot_path
            .parent()
            .expect("candidate")
            .join("validation/unit.json"),
    )
    .expect("evidence JSON");
    assert!(evidence_json.contains("\"argv\""));
    assert!(!evidence_json.contains("\"stdout\":"));
    assert!(!evidence_json.contains("\"stderr\":"));
    let evidence: EvidenceRecord = serde_json::from_str(&evidence_json).expect("evidence record");
    let capture = evidence.output_capture.expect("private output metadata");
    assert!(!capture.stdout_truncated);
    assert!(!capture.stderr_truncated);
    let payload_name = evidence
        .payload_locator
        .strip_prefix("private://validation-output/")
        .expect("private output locator");
    let payload = snapshot_path
        .parent()
        .expect("candidate")
        .join("validation-output")
        .join(payload_name);
    assert!(payload.is_file());
    assert_eq!(
        sha256_file(&payload).expect("output digest"),
        evidence.payload_sha256
    );
    assert_eq!(
        fs::metadata(&payload)
            .expect("output metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let probe = GitProbe::new(ProcessCommandOutput);
    verify_seal(
        &probe,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &seal_path,
    )
    .expect("verify seal");

    let snapshot_json = fs::read_to_string(&snapshot_path).expect("snapshot JSON");
    assert!(!snapshot_json.contains(fixture.repository.to_str().expect("UTF-8")));
    assert!(!snapshot_json.contains(fixture.state.to_str().expect("UTF-8")));
    assert!(!snapshot_json.contains(PANEL_MODEL_POLICY));
    assert_eq!(
        fs::metadata(snapshot_path.parent().expect("candidate"))
            .expect("candidate metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&snapshot_path)
            .expect("snapshot metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let portable = fixture.scratch.path.join("portable-worktree");
    git(
        &fixture.repository,
        &[
            "worktree",
            "add",
            "--detach",
            portable.to_str().expect("UTF-8"),
            &snapshot.repository_set[0].integration_oid,
        ],
    );
    let portable_roots = BTreeMap::from([(REPOSITORY_ID.to_owned(), portable)]);
    verify_seal(
        &probe,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &RejectVerifier,
        &FixturePanelVerifier,
        &portable_roots,
        &seal_path,
    )
    .expect("verify from another worktree");
}

#[test]
fn validation_checkout_is_detached_read_only_and_post_checked() {
    let mut fixture = Fixture::new("validation-checkout", ValidationAuthority::LocalRunner);
    let manifest_path = fixture.repository.join("delivery/manifest.json");
    let mut manifest: DeliveryManifest =
        serde_json::from_slice(&fs::read(&manifest_path).expect("manifest"))
            .expect("manifest JSON");
    manifest.required_validations[0].argv = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        "test -z \"$(git symbolic-ref -q HEAD)\" && chmod u+w contract.json && printf tampered > contract.json"
            .to_owned(),
    ];
    write_source_json(&manifest_path, &manifest);
    git(&fixture.repository, &["add", "delivery/manifest.json"]);
    git(
        &fixture.repository,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--amend",
            "--no-edit",
        ],
    );
    let probe = GitProbe::new(ProcessCommandOutput);
    let head = probe
        .resolve_commit(&fixture.repository, "feature")
        .expect("updated head");
    fixture.graph = StaticGraph {
        graph: graph(&fixture.status.base_oid, &head),
    };
    fixture.status = status(&fixture.status.base_oid, &head);
    let snapshot = fixture.snapshot();
    let error = run_validation(
        &probe,
        &ProcessCommandOutput,
        &fixture.roots,
        &snapshot,
        "unit",
    )
    .expect_err("validation mutated detached checkout");
    assert!(
        error
            .to_string()
            .contains("detached validation checkout identity or cleanliness changed")
    );
    assert_eq!(
        fs::read(fixture.repository.join("contract.json")).expect("original contract"),
        b"{\"version\":2}\n"
    );
}

#[test]
fn snapshot_base_relative_diff_digests_are_reverified() {
    let fixture = Fixture::new("diff-digest", ValidationAuthority::LocalRunner);
    let snapshot_path = fixture.snapshot();
    let mut forged = read_snapshot(&snapshot_path).expect("snapshot");
    forged.repository_set[0].base_to_head_diff_sha256 = "9".repeat(64);
    forged.content_id = forged.recompute_content_id().expect("content ID");
    forged.candidate_id = forged.recompute_candidate_id().expect("candidate ID");
    let forged_dir = fixture.state.join("w1").join(&forged.candidate_id);
    fs::create_dir(&forged_dir).expect("forged candidate");
    fs::set_permissions(&forged_dir, fs::Permissions::from_mode(0o700)).expect("candidate mode");
    let forged_path = forged_dir.join("snapshot.json");
    write_immutable_json(&forged_path, &forged).expect("forged snapshot");
    let error = run_validation(
        &GitProbe::new(ProcessCommandOutput),
        &ProcessCommandOutput,
        &fixture.roots,
        &forged_path,
        "unit",
    )
    .expect_err("forged base-relative diff");
    assert!(
        error
            .to_string()
            .contains("base-relative diff identity changed")
    );
}

#[test]
fn local_validation_retains_no_execution_tree_in_candidate_state() {
    let fixture = Fixture::new("compact-validation-state", ValidationAuthority::LocalRunner);
    let snapshot = fixture.snapshot();
    run_validation(
        &GitProbe::new(ProcessCommandOutput),
        &ProcessCommandOutput,
        &fixture.roots,
        &snapshot,
        "unit",
    )
    .expect("validation evidence");
    let candidate = snapshot.parent().expect("candidate directory");
    assert!(!candidate.join("execution").exists());
    assert!(candidate.join("validation/unit.json").is_file());
}

#[test]
fn local_validation_disables_persistent_compiler_cache_servers() {
    let mut fixture = Fixture::new("validation-no-sccache", ValidationAuthority::LocalRunner);
    let manifest_path = fixture.repository.join("delivery/manifest.json");
    let mut manifest: DeliveryManifest =
        serde_json::from_slice(&fs::read(&manifest_path).expect("manifest"))
            .expect("manifest JSON");
    manifest.required_validations[0].argv = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        concat!(
            "test \"$D2B_NO_SCCACHE\" = 1 && ",
            "test -z \"$RUSTC_WRAPPER\" && ",
            "test -z \"$CARGO_BUILD_RUSTC_WRAPPER\" && ",
            "test \"$SCCACHE_DIR\" = \"$D2B_VALIDATION_OUTPUT_DIR/sccache\""
        )
        .to_owned(),
    ];
    write_source_json(&manifest_path, &manifest);
    git(&fixture.repository, &["add", "delivery/manifest.json"]);
    git(
        &fixture.repository,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--amend",
            "--no-edit",
        ],
    );
    let probe = GitProbe::new(ProcessCommandOutput);
    let head = probe
        .resolve_commit(&fixture.repository, "feature")
        .expect("updated head");
    fixture.graph = StaticGraph {
        graph: graph(&fixture.status.base_oid, &head),
    };
    fixture.status = status(&fixture.status.base_oid, &head);
    let snapshot = fixture.snapshot();
    run_validation(
        &probe,
        &ProcessCommandOutput,
        &fixture.roots,
        &snapshot,
        "unit",
    )
    .expect("validation with isolated compiler cache state");
}

#[test]
fn merge_queue_without_exact_merge_group_authority_fails_closed() {
    let fixture = Fixture::new("merge-queue", ValidationAuthority::LocalRunner);
    let snapshot = fixture.snapshot();
    let probe = GitProbe::new(ProcessCommandOutput);
    run_validation(
        &probe,
        &ProcessCommandOutput,
        &fixture.roots,
        &snapshot,
        "unit",
    )
    .expect("validation");
    let (panel, trust_root) = fixture.panel_source(&snapshot);
    validate_and_store_panel(
        &probe,
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot,
        &panel,
        &trust_root,
    )
    .expect("panel");
    let mut queued = fixture.status.clone();
    queued.is_merge_queue_enabled = true;
    let error = construct_seal(
        &probe,
        &StaticStatus { status: queued },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot,
    )
    .expect_err("merge queue requires exact merge-group authority");
    assert!(error.to_string().contains("exact merge-group authority"));
}

#[test]
fn live_head_base_and_atomic_merge_races_fail_closed() {
    let fixture = Fixture::new("merge-race", ValidationAuthority::LocalRunner);
    let snapshot = fixture.snapshot();
    let seal = fixture.seal(&snapshot);
    let probe = GitProbe::new(ProcessCommandOutput);

    let mut moved_base = fixture.status.clone();
    moved_base.base_oid = "f".repeat(40);
    let error = check_merge_eligibility(
        &probe,
        &StaticStatus { status: moved_base },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &seal,
        "xtask",
    )
    .expect_err("moved base");
    assert!(error.to_string().contains("base/head") || error.to_string().contains("changed"));

    let mut moved_head = fixture.status.clone();
    moved_head.head_oid = "e".repeat(40);
    moved_head.checks[0].commit_oid = moved_head.head_oid.clone();
    let sequence = SequenceStatus {
        statuses: RefCell::new(VecDeque::from([
            fixture.status.clone(),
            fixture.status.clone(),
            moved_head,
        ])),
        fallback: fixture.status.clone(),
    };
    let merger = RecordingMerger {
        calls: Cell::new(0),
    };
    let error = atomic_merge(
        &probe,
        &sequence,
        &RejectVerifier,
        &FixturePanelVerifier,
        &merger,
        &fixture.roots,
        &seal,
        "xtask",
    )
    .expect_err("merge race");
    assert!(error.to_string().contains("base/head"));
    assert_eq!(merger.calls.get(), 0);
}

#[test]
fn panel_model_and_run_provenance_are_enforced() {
    let fixture = Fixture::new("panel-provenance", ValidationAuthority::LocalRunner);
    let snapshot = fixture.snapshot();
    let (panel, trust_root) = fixture.panel_source(&snapshot);
    let wrong_trust_root = fixture.scratch.path.join("wrong-panel-trust-root.pem");
    fs::write(&wrong_trust_root, b"attacker-controlled-key\n").expect("wrong trust root");
    let error = validate_and_store_panel(
        &GitProbe::new(ProcessCommandOutput),
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot,
        &panel,
        &wrong_trust_root,
    )
    .expect_err("untrusted panel key");
    assert!(error.to_string().contains("checked-in candidate authority"));
    let rust_path = panel.join("rust.json");
    let mut rust: PanelAttestation =
        serde_json::from_slice(&fs::read(&rust_path).expect("record")).expect("record JSON");
    rust.model_version = "wrong-model".to_owned();
    write_source_json(&rust_path, &rust);
    sign_fixture_receipt(&rust_path);
    let probe = GitProbe::new(ProcessCommandOutput);
    let error = validate_and_store_panel(
        &probe,
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot,
        &panel,
        &trust_root,
    )
    .expect_err("wrong model");
    assert!(error.to_string().contains("provider/model"));

    rust.model_version = PANEL_MODEL_POLICY.to_owned();
    rust.run_id = "run-0".to_owned();
    write_source_json(&rust_path, &rust);
    sign_fixture_receipt(&rust_path);
    let error = validate_and_store_panel(
        &probe,
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot,
        &panel,
        &trust_root,
    )
    .expect_err("duplicate run");
    assert!(error.to_string().contains("repeats a provider/run"));
}

#[test]
fn panel_receipt_inputs_cannot_change_between_verification_and_retention() {
    let fixture = Fixture::new("panel-toctou", ValidationAuthority::LocalRunner);
    let snapshot = fixture.snapshot();
    let (panel, trust_root) = fixture.panel_source(&snapshot);
    let verifier = MutatingPanelVerifier {
        mutated: Cell::new(false),
    };
    let error = validate_and_store_panel(
        &GitProbe::new(ProcessCommandOutput),
        &verifier,
        &fixture.roots,
        &snapshot,
        &panel,
        &trust_root,
    )
    .expect_err("receipt changed before retention");
    assert!(
        error
            .to_string()
            .contains("changed before immutable retention")
    );
}

#[test]
fn panel_detached_signatures_are_reverified_from_the_seal() {
    let fixture = Fixture::new("panel-reverify", ValidationAuthority::LocalRunner);
    let snapshot = fixture.snapshot();
    let seal = fixture.seal(&snapshot);
    let candidate = snapshot.parent().expect("candidate");
    let signature = candidate.join("panel/rust.sig");
    fs::write(&signature, b"forged-signature\n").expect("forge signature");
    let signature_digest = sha256_file(&signature).expect("signature digest");
    fs::write(
        candidate.join("panel/rust.sig.sha256"),
        format!("{signature_digest}\n"),
    )
    .expect("rewrite digest sidecar");
    let error = verify_seal(
        &GitProbe::new(ProcessCommandOutput),
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &seal,
    )
    .expect_err("forged panel signature");
    assert!(
        error
            .to_string()
            .contains("detached-signature verification failed")
    );
}

struct RejectVerifier;

impl CiAttestationVerifier for RejectVerifier {
    fn verify(
        &self,
        _attestation_path: &Path,
        _bundle_path: &Path,
        _policy: &xtask::delivery::evidence::CiAttestationPolicy,
    ) -> Result<VerifiedCiAttestation> {
        Err(DeliveryError::new("signature verification failed"))
    }
}

struct ClaimsVerifier {
    verified: VerifiedCiAttestation,
}

impl CiAttestationVerifier for ClaimsVerifier {
    fn verify(
        &self,
        _attestation_path: &Path,
        _bundle_path: &Path,
        _policy: &xtask::delivery::evidence::CiAttestationPolicy,
    ) -> Result<VerifiedCiAttestation> {
        Ok(self.verified.clone())
    }
}

struct CountingClaimsVerifier {
    verified: VerifiedCiAttestation,
    calls: Cell<usize>,
}

impl CiAttestationVerifier for CountingClaimsVerifier {
    fn verify(
        &self,
        attestation_path: &Path,
        bundle_path: &Path,
        _policy: &xtask::delivery::evidence::CiAttestationPolicy,
    ) -> Result<VerifiedCiAttestation> {
        if sha256_file(attestation_path)? != self.verified.artifact_sha256
            || sha256_file(bundle_path)? != self.verified.bundle_sha256
        {
            return Err(DeliveryError::new(
                "retained CI attestation artifact or bundle changed",
            ));
        }
        self.calls.set(self.calls.get() + 1);
        Ok(self.verified.clone())
    }
}

#[test]
fn seal_rejects_extra_panel_json() {
    let fixture = Fixture::new("panel-extra-json", ValidationAuthority::LocalRunner);
    let snapshot = fixture.snapshot();
    let probe = GitProbe::new(ProcessCommandOutput);
    run_validation(
        &probe,
        &ProcessCommandOutput,
        &fixture.roots,
        &snapshot,
        "unit",
    )
    .expect("validation evidence");
    let (panel, trust_root) = fixture.panel_source(&snapshot);
    validate_and_store_panel(
        &probe,
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot,
        &panel,
        &trust_root,
    )
    .expect("panel");
    let extra = snapshot
        .parent()
        .expect("candidate directory")
        .join("panel/extra.json");
    fs::write(&extra, b"{}\n").expect("extra panel JSON");
    fs::set_permissions(&extra, fs::Permissions::from_mode(0o600))
        .expect("private extra panel JSON");
    let error = construct_seal(
        &probe,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot,
    )
    .expect_err("extra panel JSON must fail closed");
    assert!(
        error.to_string().contains("panel evidence"),
        "unexpected error: {error}"
    );
}

#[test]
fn forged_and_delivery_artifact_evidence_are_rejected() {
    let fixture = Fixture::new("evidence", ValidationAuthority::GithubAttestation);
    let snapshot_path = fixture.snapshot();
    let attestation = fixture.scratch.path.join("signed-attestation.bundle");
    let bundle = fixture.scratch.path.join("signed-attestation.bundle.jsonl");
    fs::write(&bundle, b"{\"fixture\":\"signed-bundle\"}\n").expect("bundle");
    let probe = GitProbe::new(ProcessCommandOutput);
    let snapshot = read_snapshot(&snapshot_path).expect("snapshot");
    let claims = CiAttestationClaims {
        candidate_id: snapshot.candidate_id.clone(),
        content_id: snapshot.content_id.clone(),
        snapshot_sha256: verify_json_digest(&snapshot_path).expect("digest"),
        validation_id: "unit".to_owned(),
        argv: snapshot.required_validations[0].argv.clone(),
        cwd: snapshot.required_validations[0].cwd.clone(),
        repository_set: snapshot.repository_bindings(),
        exit_code: 0,
        conclusion: "success".to_owned(),
        captured_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_secs(),
        payload_locator: "github-artifact://run/output".to_owned(),
        payload_sha256: sha256_file(&snapshot_path).expect("snapshot payload digest"),
        repository: REPOSITORY_ID.to_owned(),
        run_id: "123".to_owned(),
        check_run_id: "456".to_owned(),
        app_slug: "github-actions".to_owned(),
        app_id: 15368,
        workflow: "Layer 1".to_owned(),
        workflow_id: 321,
    };
    write_source_json(&attestation, &claims);
    let error = import_ci_evidence(
        &probe,
        &RejectVerifier,
        &fixture.roots,
        &snapshot_path,
        &attestation,
        &bundle,
        None,
    )
    .expect_err("forged");
    assert!(error.to_string().contains("signature verification"));

    let mut wrong_claims = claims.clone();
    wrong_claims.app_id = 1;
    write_source_json(&attestation, &wrong_claims);
    let wrong_verifier = ClaimsVerifier {
        verified: VerifiedCiAttestation {
            claims: wrong_claims.clone(),
            artifact_sha256: "a".repeat(64),
            bundle_sha256: sha256_file(&bundle).expect("bundle digest"),
        },
    };
    let error = import_ci_evidence(
        &probe,
        &wrong_verifier,
        &fixture.roots,
        &snapshot_path,
        &attestation,
        &bundle,
        None,
    )
    .expect_err("wrong CI publisher");
    assert!(error.to_string().contains("publisher/repository"));

    write_source_json(&attestation, &claims);
    let verifier = ClaimsVerifier {
        verified: VerifiedCiAttestation {
            claims: claims.clone(),
            artifact_sha256: sha256_file(&attestation).expect("attestation digest"),
            bundle_sha256: sha256_file(&bundle).expect("bundle digest"),
        },
    };
    let error = import_ci_evidence(
        &probe,
        &verifier,
        &fixture.roots,
        &snapshot_path,
        &attestation,
        &bundle,
        Some(&snapshot_path),
    )
    .expect_err("delivery payload");
    assert!(error.to_string().contains("delivery"));

    let error = import_ci_evidence(
        &probe,
        &verifier,
        &fixture.roots,
        &snapshot_path,
        &snapshot_path,
        &bundle,
        None,
    )
    .expect_err("delivery attestation");
    assert!(error.to_string().contains("delivery"));
}

#[test]
fn ci_attestation_bundle_is_retained_and_reverified_at_seal_verify() {
    let fixture = Fixture::new("ci-reverify", ValidationAuthority::GithubAttestation);
    let snapshot_path = fixture.snapshot();
    let snapshot = read_snapshot(&snapshot_path).expect("snapshot");
    let attestation = fixture.scratch.path.join("ci-claims.json");
    let bundle = fixture.scratch.path.join("ci-attestation.bundle.jsonl");
    let claims = CiAttestationClaims {
        candidate_id: snapshot.candidate_id.clone(),
        content_id: snapshot.content_id.clone(),
        snapshot_sha256: verify_json_digest(&snapshot_path).expect("snapshot digest"),
        validation_id: "unit".to_owned(),
        argv: snapshot.required_validations[0].argv.clone(),
        cwd: snapshot.required_validations[0].cwd.clone(),
        repository_set: snapshot.repository_bindings(),
        exit_code: 0,
        conclusion: "success".to_owned(),
        captured_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_secs(),
        payload_locator: "github-artifact://runs/123/output".to_owned(),
        payload_sha256: "f".repeat(64),
        repository: REPOSITORY_ID.to_owned(),
        run_id: "123".to_owned(),
        check_run_id: "456".to_owned(),
        app_slug: "github-actions".to_owned(),
        app_id: 15368,
        workflow: "Layer 1".to_owned(),
        workflow_id: 321,
    };
    write_source_json(&attestation, &claims);
    fs::write(&bundle, b"{\"fixture\":\"offline-sigstore-bundle\"}\n").expect("bundle");
    let verifier = CountingClaimsVerifier {
        verified: VerifiedCiAttestation {
            claims,
            artifact_sha256: sha256_file(&attestation).expect("artifact digest"),
            bundle_sha256: sha256_file(&bundle).expect("bundle digest"),
        },
        calls: Cell::new(0),
    };
    let probe = GitProbe::new(ProcessCommandOutput);
    import_ci_evidence(
        &probe,
        &verifier,
        &fixture.roots,
        &snapshot_path,
        &attestation,
        &bundle,
        None,
    )
    .expect("import CI evidence");
    assert_eq!(verifier.calls.get(), 1);
    fs::remove_file(&attestation).expect("remove source artifact");
    fs::remove_file(&bundle).expect("remove source bundle");

    let (panel, trust_root) = fixture.panel_source(&snapshot_path);
    validate_and_store_panel(
        &probe,
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot_path,
        &panel,
        &trust_root,
    )
    .expect("panel");
    let seal = construct_seal(
        &probe,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &verifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &snapshot_path,
    )
    .expect("seal");
    assert_eq!(verifier.calls.get(), 2);
    verify_seal(
        &probe,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &verifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &seal,
    )
    .expect("seal verification");
    assert_eq!(verifier.calls.get(), 3);

    let retained_bundle = snapshot_path
        .parent()
        .expect("candidate")
        .join("ci-attestations/unit.bundle.jsonl");
    fs::write(&retained_bundle, b"{\"forged\":\"bundle\"}\n").expect("forge bundle");
    let forged_digest = sha256_file(&retained_bundle).expect("forged digest");
    fs::write(
        snapshot_path
            .parent()
            .expect("candidate")
            .join("ci-attestations/unit.bundle.jsonl.sha256"),
        format!("{forged_digest}\n"),
    )
    .expect("forge bundle sidecar");
    let error = verify_seal(
        &probe,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &verifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &seal,
    )
    .expect_err("forged retained CI bundle");
    assert!(
        error
            .to_string()
            .contains("retained CI attestation artifact or bundle")
    );
}

#[test]
fn candidate_and_content_ids_are_cross_repository_domain_separated() {
    let fixture = Fixture::new("identity", ValidationAuthority::LocalRunner);
    let snapshot_path = fixture.snapshot();
    let snapshot = read_snapshot(&snapshot_path).expect("snapshot");
    let mut renamed = snapshot.clone();
    let replacement = "github.com/example/other";
    renamed.authority.repository = replacement.to_owned();
    renamed.repository_set[0].id = replacement.to_owned();
    renamed.stack[0].repository = replacement.to_owned();
    renamed.required_validations[0].cwd.repository = replacement.to_owned();
    renamed.dependency_fingerprints[0].repository = replacement.to_owned();
    renamed.contract_fingerprints[0].repository = replacement.to_owned();
    renamed.candidate_id = renamed.recompute_candidate_id().expect("candidate");
    renamed.content_id = renamed.recompute_content_id().expect("content");
    assert_ne!(snapshot.candidate_id, renamed.candidate_id);
    assert_ne!(snapshot.content_id, renamed.content_id);
}

#[test]
fn ancestor_symlink_checkout_mapping_is_rejected() {
    let fixture = Fixture::new("symlink", ValidationAuthority::LocalRunner);
    let link_parent = fixture.scratch.path.join("link-parent");
    fs::create_dir(&link_parent).expect("link parent");
    let link = link_parent.join("repository-link");
    symlink(&fixture.repository, &link).expect("symlink");
    let mut request = fixture.request.clone();
    request.repository_roots = BTreeMap::from([(REPOSITORY_ID.to_owned(), link)]);
    let probe = GitProbe::new(ProcessCommandOutput);
    let error = create_snapshot(
        &probe,
        &fixture.graph,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &request,
    )
    .expect_err("symlink root");
    assert!(error.to_string().contains("symlink"));
}

#[test]
fn state_inside_git_common_directory_is_rejected() {
    let fixture = Fixture::new("git-common", ValidationAuthority::LocalRunner);
    let probe = GitProbe::new(ProcessCommandOutput);
    let mut request = fixture.request.clone();
    request.state_root = Some(
        probe
            .git_common_dir(&fixture.repository)
            .expect("git common")
            .join("delivery-state"),
    );
    let error = create_snapshot(
        &probe,
        &fixture.graph,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &request,
    )
    .expect_err("Git common state");
    assert!(error.to_string().contains("Git metadata"));
}

struct MutatingGraph {
    graph: StackGraph,
    repository: PathBuf,
    replacement_oid: String,
    calls: Cell<usize>,
}

impl StackGraphSource for MutatingGraph {
    fn graph(
        &self,
        _repository: &str,
        _checkout_root: &Path,
        _expected_nodes: &[StackNodePolicy],
    ) -> Result<StackGraph> {
        let calls = self.calls.get();
        self.calls.set(calls + 1);
        if calls == 1 {
            git(
                &self.repository,
                &["update-ref", "refs/heads/feature", &self.replacement_oid],
            );
        }
        Ok(self.graph.clone())
    }
}

struct MutatingBlobProbe {
    inner: GitProbe<ProcessCommandOutput>,
    repository: PathBuf,
    tracked_blob_calls: Cell<usize>,
}

impl RepositoryProbe for MutatingBlobProbe {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf> {
        self.inner.canonical_root(root)
    }

    fn repository_identity(&self, root: &Path) -> Result<String> {
        self.inner.repository_identity(root)
    }

    fn git_common_dir(&self, root: &Path) -> Result<PathBuf> {
        self.inner.git_common_dir(root)
    }

    fn object_format(&self, root: &Path) -> Result<GitObjectFormat> {
        self.inner.object_format(root)
    }

    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String> {
        self.inner.resolve_commit(root, revision)
    }

    fn tree_for_commit(&self, root: &Path, commit_oid: &str) -> Result<String> {
        self.inner.tree_for_commit(root, commit_oid)
    }

    fn is_dirty(&self, root: &Path) -> Result<bool> {
        self.inner.is_dirty(root)
    }

    fn is_ancestor(&self, root: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
        self.inner.is_ancestor(root, ancestor, descendant)
    }

    fn tracked_paths(&self, root: &Path, commit_oid: &str, prefix: &Path) -> Result<Vec<PathBuf>> {
        self.inner.tracked_paths(root, commit_oid, prefix)
    }

    fn tracked_blob(&self, root: &Path, commit_oid: &str, path: &Path) -> Result<TrackedBlob> {
        let calls = self.tracked_blob_calls.get();
        self.tracked_blob_calls.set(calls + 1);
        if calls == 1 {
            fs::write(self.repository.join("contract.json"), b"mutable worktree\n")
                .expect("mutate worktree");
        }
        self.inner.tracked_blob(root, commit_oid, path)
    }

    fn canonical_diff(
        &self,
        root: &Path,
        base_oid: &str,
        head_oid: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<u8>> {
        self.inner.canonical_diff(root, base_oid, head_oid, paths)
    }

    fn prospective_merge_tree(
        &self,
        root: &Path,
        base_oid: &str,
        head_oid: &str,
    ) -> Result<String> {
        self.inner.prospective_merge_tree(root, base_oid, head_oid)
    }
}

#[test]
fn snapshot_detects_ref_and_worktree_toctou_mutation() {
    let fixture = Fixture::new("toctou-ref", ValidationAuthority::LocalRunner);
    let probe = GitProbe::new(ProcessCommandOutput);
    let base = probe
        .resolve_commit(&fixture.repository, "main")
        .expect("base");
    let graph = MutatingGraph {
        graph: fixture.graph.graph.clone(),
        repository: fixture.repository.clone(),
        replacement_oid: base,
        calls: Cell::new(0),
    };
    let error = create_snapshot(
        &probe,
        &graph,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &fixture.request,
    )
    .expect_err("ref mutation");
    assert!(
        error.to_string().contains("moved")
            || error.to_string().contains("dirty")
            || error.to_string().contains("changed")
    );

    let fixture = Fixture::new("toctou-file", ValidationAuthority::LocalRunner);
    let mutating_probe = MutatingBlobProbe {
        inner: GitProbe::new(ProcessCommandOutput),
        repository: fixture.repository.clone(),
        tracked_blob_calls: Cell::new(0),
    };
    let error = create_snapshot(
        &mutating_probe,
        &fixture.graph,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &fixture.request,
    )
    .expect_err("file mutation");
    assert!(error.to_string().contains("dirty worktree"));
}

#[test]
fn git_town_merged_prefix_progresses_without_changing_content() {
    let scratch = Scratch::new("merged-prefix");
    let repository = scratch.path.join("repository");
    fs::create_dir(&repository).expect("repository");
    git(&repository, &["init", "--object-format=sha1", "-b", "main"]);
    git(&repository, &["config", "user.name", "Example"]);
    git(
        &repository,
        &["config", "user.email", "example@example.invalid"],
    );
    git(
        &repository,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/d2b.git",
        ],
    );
    fs::write(repository.join("contract.json"), b"{\"version\":1}\n").expect("contract");
    fs::write(repository.join("dependencies.txt"), b"none\n").expect("dependencies");
    git(&repository, &["add", "contract.json", "dependencies.txt"]);
    git(&repository, &["commit", "-m", "base"]);
    let probe = GitProbe::new(ProcessCommandOutput);
    let base = probe.resolve_commit(&repository, "main").expect("base");

    git(&repository, &["checkout", "-b", "first"]);
    fs::write(repository.join("first.txt"), b"first\n").expect("first");
    git(&repository, &["add", "first.txt"]);
    git(&repository, &["commit", "-m", "first"]);
    let first = probe.resolve_commit(&repository, "first").expect("first");

    git(&repository, &["checkout", "-b", "second"]);
    fs::write(repository.join("second.txt"), b"second\n").expect("second");
    fs::write(repository.join("contract.json"), b"{\"version\":2}\n").expect("contract");
    let mut delivery = manifest(ValidationAuthority::LocalRunner);
    delivery.repositories[0].integration_ref = "second".to_owned();
    delivery.stack_nodes = vec![
        StackNodePolicy {
            id: "first".to_owned(),
            repository: REPOSITORY_ID.to_owned(),
            branch: "first".to_owned(),
            pr_number: 41,
            external_dependencies: vec![],
        },
        StackNodePolicy {
            id: "second".to_owned(),
            repository: REPOSITORY_ID.to_owned(),
            branch: "second".to_owned(),
            pr_number: 42,
            external_dependencies: vec![],
        },
    ];
    delivery.required_checks = vec![
        RequiredCheck {
            node: "first".to_owned(),
            name: "check".to_owned(),
            publisher: publisher(),
        },
        RequiredCheck {
            node: "second".to_owned(),
            name: "check".to_owned(),
            publisher: publisher(),
        },
    ];
    fs::create_dir(repository.join("delivery")).expect("delivery directory");
    write_source_json(&repository.join("delivery/manifest.json"), &delivery);
    git(
        &repository,
        &[
            "add",
            "contract.json",
            "second.txt",
            "delivery/manifest.json",
        ],
    );
    git(&repository, &["commit", "-m", "second"]);
    let second = probe.resolve_commit(&repository, "second").expect("second");

    let roots = BTreeMap::from([(REPOSITORY_ID.to_owned(), repository.clone())]);
    let request = SnapshotRequest {
        authority_repository: REPOSITORY_ID.to_owned(),
        authority_ref: "second".to_owned(),
        manifest_path: PathBuf::from("delivery/manifest.json"),
        repository_roots: roots.clone(),
        state_root: Some(scratch.path.join("state")),
    };
    let old_graph = StaticGraph {
        graph: StackGraph {
            trunk: "main".to_owned(),
            current_branch: "second".to_owned(),
            branches: vec![
                StackBranch {
                    name: "first".to_owned(),
                    parent: "main".to_owned(),
                    base_ref: "main".to_owned(),
                    observed_base: base.clone(),
                    head: first.clone(),
                    base: base.clone(),
                    is_current: false,
                    is_merged: false,
                    is_queued: false,
                    needs_rebase: false,
                    pr: Some(StackPr {
                        number: 41,
                        url: String::new(),
                        state: "OPEN".to_owned(),
                    }),
                    merge_commit_oid: None,
                    merge_commit_tree_oid: None,
                },
                StackBranch {
                    name: "second".to_owned(),
                    parent: "first".to_owned(),
                    base_ref: "first".to_owned(),
                    observed_base: first.clone(),
                    head: second.clone(),
                    base: first.clone(),
                    is_current: true,
                    is_merged: false,
                    is_queued: false,
                    needs_rebase: false,
                    pr: Some(StackPr {
                        number: 42,
                        url: String::new(),
                        state: "OPEN".to_owned(),
                    }),
                    merge_commit_oid: None,
                    merge_commit_tree_oid: None,
                },
            ],
        },
    };
    let old_status = StatusMap {
        statuses: BTreeMap::from([
            (
                41,
                status_for(41, PullRequestState::Open, "main", &base, "first", &first),
            ),
            (
                42,
                status_for(
                    42,
                    PullRequestState::Open,
                    "first",
                    &first,
                    "second",
                    &second,
                ),
            ),
        ]),
    };
    let old_path =
        create_snapshot(&probe, &old_graph, &old_status, &request).expect("old snapshot");
    run_validation(&probe, &ProcessCommandOutput, &roots, &old_path, "unit")
        .expect("old validation");
    let (old_panel, old_trust_root) = panel_source_at(&scratch.path, &old_path);
    validate_and_store_panel(
        &probe,
        &FixturePanelVerifier,
        &roots,
        &old_path,
        &old_panel,
        &old_trust_root,
    )
    .expect("old panel");
    let old_seal = construct_seal(
        &probe,
        &old_status,
        &RejectVerifier,
        &FixturePanelVerifier,
        &roots,
        &old_path,
    )
    .expect("old seal");

    git(&repository, &["checkout", "main"]);
    git(&repository, &["merge", "--squash", "first"]);
    git(&repository, &["commit", "-m", "merge first"]);
    let advanced_base = probe
        .resolve_commit(&repository, "main")
        .expect("advanced base");
    git(
        &repository,
        &["rebase", "--onto", "main", "first", "second"],
    );
    let rebased_second = probe
        .resolve_commit(&repository, "second")
        .expect("rebased second");
    let new_graph = StaticGraph {
        graph: StackGraph {
            trunk: "main".to_owned(),
            current_branch: "second".to_owned(),
            branches: vec![
                StackBranch {
                    name: "first".to_owned(),
                    parent: "main".to_owned(),
                    base_ref: "main".to_owned(),
                    observed_base: base.clone(),
                    head: first.clone(),
                    base: base.clone(),
                    is_current: false,
                    is_merged: true,
                    is_queued: false,
                    needs_rebase: false,
                    pr: Some(StackPr {
                        number: 41,
                        url: String::new(),
                        state: "MERGED".to_owned(),
                    }),
                    merge_commit_oid: Some(advanced_base.clone()),
                    merge_commit_tree_oid: Some(
                        probe
                            .tree_for_commit(&repository, &advanced_base)
                            .expect("merge tree"),
                    ),
                },
                StackBranch {
                    name: "second".to_owned(),
                    parent: "first".to_owned(),
                    base_ref: "main".to_owned(),
                    observed_base: advanced_base.clone(),
                    head: rebased_second.clone(),
                    base: advanced_base.clone(),
                    is_current: true,
                    is_merged: false,
                    is_queued: false,
                    needs_rebase: false,
                    pr: Some(StackPr {
                        number: 42,
                        url: String::new(),
                        state: "OPEN".to_owned(),
                    }),
                    merge_commit_oid: None,
                    merge_commit_tree_oid: None,
                },
            ],
        },
    };
    let mut merged_first = status_for(41, PullRequestState::Merged, "main", &base, "first", &first);
    merged_first.merge_commit_oid = Some(advanced_base.clone());
    merged_first.merge_commit_tree_oid = Some(
        probe
            .tree_for_commit(&repository, &advanced_base)
            .expect("merge tree"),
    );
    let new_status = StatusMap {
        statuses: BTreeMap::from([
            (41, merged_first),
            (
                42,
                status_for(
                    42,
                    PullRequestState::Open,
                    "main",
                    &advanced_base,
                    "second",
                    &rebased_second,
                ),
            ),
        ]),
    };
    let new_path =
        create_snapshot(&probe, &new_graph, &new_status, &request).expect("new snapshot");
    let old = read_snapshot(&old_path).expect("old snapshot");
    let new = read_snapshot(&new_path).expect("new snapshot");
    assert_eq!(old.content_id, new.content_id);
    assert_ne!(old.candidate_id, new.candidate_id);
    assert_eq!(new.stack[0].snapshot_state, PullRequestState::Merged);
    assert_eq!(new.stack[0].expected_base_oid, base);
    assert_eq!(new.stack[1].expected_base_ref, "main");
    assert_eq!(new.stack[1].expected_base_oid, advanced_base);
    assert_eq!(new.stack[1].depends_on, ["first"]);
    xtask::delivery::verify_history_only_equivalence(&old, &new)
        .expect("merged prefix progression");
    let proof = construct_history_proof(
        &probe,
        &RejectVerifier,
        &FixturePanelVerifier,
        &roots,
        &old_seal,
        &new_path,
    )
    .expect("merged progression proof");
    let proof_record: HistoryProof =
        serde_json::from_slice(&fs::read(&proof).expect("proof")).expect("proof JSON");
    assert_eq!(
        proof_record.transition_kind,
        xtask::delivery::seal::HistoryTransitionKind::MergedStackProgression
    );
    check_history_merge_eligibility(
        &probe,
        &new_status,
        &RejectVerifier,
        &FixturePanelVerifier,
        &roots,
        &old_seal,
        &new_path,
        &proof,
        "second",
    )
    .expect("merged progression eligibility with fresh CI");
}

#[test]
fn history_proof_rejects_fabrication_and_requires_fresh_ci() {
    let mut fixture = Fixture::new("history", ValidationAuthority::LocalRunner);
    let old_snapshot = fixture.snapshot();
    let old_seal = fixture.seal(&old_snapshot);
    let old = read_snapshot(&old_snapshot).expect("old snapshot");

    git(
        &fixture.repository,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--amend",
            "--no-edit",
            "--date",
            "2030-01-01T00:00:00Z",
        ],
    );
    let probe = GitProbe::new(ProcessCommandOutput);
    let new_head = probe
        .resolve_commit(&fixture.repository, "feature")
        .expect("new head");
    assert_ne!(old.repository_set[0].integration_oid, new_head);
    fixture.graph = StaticGraph {
        graph: graph(&fixture.status.base_oid, &new_head),
    };
    fixture.status = status(&fixture.status.base_oid, &new_head);
    let new_snapshot = fixture.snapshot();
    let new = read_snapshot(&new_snapshot).expect("new snapshot");
    assert_eq!(old.content_id, new.content_id);
    assert_ne!(old.candidate_id, new.candidate_id);

    let proof_path = construct_history_proof(
        &probe,
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &old_seal,
        &new_snapshot,
    )
    .expect("history proof");
    let proof: HistoryProof =
        serde_json::from_slice(&fs::read(&proof_path).expect("proof")).expect("proof JSON");
    assert!(proof.fresh_ci_required);
    assert_eq!(proof.reused_panel_payloads.len(), 10);
    let old_seal_record: xtask::delivery::WaveSeal =
        serde_json::from_slice(&fs::read(&old_seal).expect("old seal")).expect("old seal JSON");
    let sealed_check = &old_seal_record.live_pull_requests[0].checks[0];

    let mut stale_run_ids = fixture.status.clone();
    stale_run_ids.checks[0].check_run_id = sealed_check.check_run_id;
    stale_run_ids.checks[0].workflow_run_id = sealed_check.workflow_run_id;
    let error = check_history_merge_eligibility(
        &probe,
        &StaticStatus {
            status: stale_run_ids,
        },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &old_seal,
        &new_snapshot,
        &proof_path,
        "xtask",
    )
    .expect_err("reused CI run IDs");
    assert!(error.to_string().contains("fresh"));

    let mut stale_timestamps = fixture.status.clone();
    stale_timestamps.checks[0].started_at_unix_seconds = sealed_check.started_at_unix_seconds;
    stale_timestamps.checks[0].completed_at_unix_seconds = sealed_check.completed_at_unix_seconds;
    stale_timestamps.checks[0].workflow_created_at_unix_seconds =
        sealed_check.workflow_created_at_unix_seconds;
    stale_timestamps.checks[0].workflow_updated_at_unix_seconds =
        sealed_check.workflow_updated_at_unix_seconds;
    let error = check_history_merge_eligibility(
        &probe,
        &StaticStatus {
            status: stale_timestamps,
        },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &old_seal,
        &new_snapshot,
        &proof_path,
        "xtask",
    )
    .expect_err("reused CI timestamps");
    assert!(error.to_string().contains("fresh"));

    let mut missing_ci = fixture.status.clone();
    missing_ci.checks.clear();
    let error = check_history_merge_eligibility(
        &probe,
        &StaticStatus { status: missing_ci },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &old_seal,
        &new_snapshot,
        &proof_path,
        "xtask",
    )
    .expect_err("missing fresh CI");
    assert!(error.to_string().contains("required check"));

    check_history_merge_eligibility(
        &probe,
        &StaticStatus {
            status: fixture.status.clone(),
        },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &old_seal,
        &new_snapshot,
        &proof_path,
        "xtask",
    )
    .expect("fresh CI on new head");

    let mut rerun = fixture.status.clone();
    rerun.checks[0].workflow_run_id = sealed_check.workflow_run_id;
    rerun.checks[0].workflow_created_at_unix_seconds =
        sealed_check.workflow_created_at_unix_seconds;
    rerun.checks[0].workflow_updated_at_unix_seconds = Some(
        rerun.checks[0]
            .completed_at_unix_seconds
            .expect("rerun completion")
            .max(
                sealed_check
                    .workflow_updated_at_unix_seconds
                    .expect("sealed workflow update")
                    + 1,
            ),
    );
    check_history_merge_eligibility(
        &probe,
        &StaticStatus { status: rerun },
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &old_seal,
        &new_snapshot,
        &proof_path,
        "xtask",
    )
    .expect("fresh check attempt in the same workflow run");

    let fabricated_id = "9".repeat(64);
    let fabricated_dir = fixture.state.join("w1").join(&fabricated_id);
    fs::create_dir_all(&fabricated_dir).expect("fabricated dir");
    let mut fabricated = new.clone();
    fabricated.candidate_id = fabricated_id;
    let fabricated_path = fabricated_dir.join("snapshot.json");
    write_immutable_json(&fabricated_path, &fabricated).expect("fabricated artifact");
    let error = construct_history_proof(
        &probe,
        &RejectVerifier,
        &FixturePanelVerifier,
        &fixture.roots,
        &old_seal,
        &fabricated_path,
    )
    .expect_err("fabricated snapshot");
    assert!(error.to_string().contains("candidate ID"));
}
