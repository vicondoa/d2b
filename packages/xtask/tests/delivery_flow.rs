#![forbid(unsafe_code)]

use std::{
    cell::Cell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use xtask::delivery::{
    DELIVERY_SCHEMA_VERSION, EvidenceImportRequest, EvidencePayloadSource, EvidenceResultClass,
    FingerprintSpec, PANEL_ROLES, PanelRecord, RepositorySpec, RequiredCheck,
    RequiredValidationSpec, RootRepositorySpec, StackManifest, StackNodeSpec,
    command::RepositoryProbe, construct_seal, create_snapshot, import_evidence, read_snapshot,
    storage::verify_json_digest, validate_and_store_panel, verify_seal,
};
use xtask::delivery::{DeliveryError, Result};

static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(1);

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
                ".d2b-xtask-delivery-{label}-{}-{}",
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

struct FakeProbe {
    commits: BTreeMap<String, String>,
    trees: BTreeMap<String, String>,
    dirty: Cell<bool>,
    common: PathBuf,
}

impl RepositoryProbe for FakeProbe {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf> {
        Ok(fs::canonicalize(root)?)
    }

    fn git_common_dir(&self, _root: &Path) -> Result<PathBuf> {
        Ok(self.common.clone())
    }

    fn resolve_commit(&self, _root: &Path, revision: &str) -> Result<String> {
        self.commits
            .get(revision)
            .cloned()
            .ok_or_else(|| DeliveryError::new(format!("missing fake commit {revision}")))
    }

    fn resolve_tree(&self, _root: &Path, revision: &str) -> Result<String> {
        self.trees
            .get(revision)
            .cloned()
            .ok_or_else(|| DeliveryError::new(format!("missing fake tree {revision}")))
    }

    fn is_dirty(&self, _root: &Path) -> Result<bool> {
        Ok(self.dirty.get())
    }
}

fn fixture(scratch: &Scratch) -> (FakeProbe, StackManifest) {
    let repository = scratch.path.join("repository");
    let common = scratch.path.join("git-common");
    fs::create_dir(&repository).expect("fake repository");
    fs::create_dir(&common).expect("fake common dir");
    fs::write(repository.join("contract.json"), b"{\"schema\":1}\n").expect("contract");
    let tree = "a".repeat(40);
    let probe = FakeProbe {
        commits: BTreeMap::from([
            ("base".to_owned(), "0".repeat(40)),
            ("head".to_owned(), "1".repeat(40)),
            ("feature".to_owned(), "1".repeat(40)),
        ]),
        trees: BTreeMap::from([("head".to_owned(), tree.clone()), ("HEAD".to_owned(), tree)]),
        dirty: Cell::new(false),
        common,
    };
    let manifest = StackManifest {
        schema_version: DELIVERY_SCHEMA_VERSION,
        wave: "w1".to_owned(),
        root_repository: RootRepositorySpec {
            name: "example/d2b".to_owned(),
            root: repository.clone(),
            base: "base".to_owned(),
            head: "head".to_owned(),
        },
        repository_set: vec![RepositorySpec {
            name: "example/d2b".to_owned(),
            root: repository,
            head: "head".to_owned(),
        }],
        stack: vec![StackNodeSpec {
            id: "root".to_owned(),
            repository: "example/d2b".to_owned(),
            branch: "feature".to_owned(),
            pr: Some(42),
            head: "head".to_owned(),
            depends_on: vec![],
        }],
        required_validations: vec![RequiredValidationSpec {
            id: "unit".to_owned(),
            command: "cargo test -p xtask".to_owned(),
        }],
        required_checks: vec![RequiredCheck {
            node: "root".to_owned(),
            name: "unit".to_owned(),
        }],
        generated_artifacts: vec![],
        dependency_fingerprints: vec![],
        contract_fingerprints: vec![FingerprintSpec {
            name: "contract".to_owned(),
            repository: "example/d2b".to_owned(),
            path: PathBuf::from("contract.json"),
        }],
    };
    (probe, manifest)
}

fn write_json(path: &Path, value: &impl serde::Serialize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create JSON parent");
    }
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize JSON");
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write JSON");
}

fn import_evidence_result(
    scratch: &Scratch,
    probe: &FakeProbe,
    snapshot_path: &Path,
    result_class: EvidenceResultClass,
) {
    let snapshot = read_snapshot(snapshot_path).expect("read snapshot");
    let request = EvidenceImportRequest {
        schema_version: DELIVERY_SCHEMA_VERSION,
        id: "unit".to_owned(),
        command: "cargo test -p xtask".to_owned(),
        result_class,
        timestamp: "2026-07-13T07:41:58Z".to_owned(),
        tree_hash: snapshot.root_repository.tree_hash,
        payload: EvidencePayloadSource {
            path: None,
            sha256: Some("b".repeat(64)),
            external_locator: Some("artifact://local/unit".to_owned()),
        },
    };
    let request_path = scratch.path.join("requests/unit.json");
    write_json(&request_path, &request);
    import_evidence(probe, snapshot_path, &request_path).expect("import evidence");
}

fn import_passing_evidence(scratch: &Scratch, probe: &FakeProbe, snapshot_path: &Path) {
    import_evidence_result(scratch, probe, snapshot_path, EvidenceResultClass::Passed);
}

fn write_panel(
    scratch: &Scratch,
    snapshot_path: &Path,
    finding_role: Option<xtask::delivery::PanelRole>,
    omit_last: bool,
) -> PathBuf {
    let snapshot = read_snapshot(snapshot_path).expect("snapshot");
    let snapshot_sha256 = verify_json_digest(snapshot_path).expect("snapshot digest");
    let records = scratch.path.join(format!(
        "panel-source-{}",
        NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&records).expect("panel source");
    let roles = if omit_last {
        &PANEL_ROLES[..PANEL_ROLES.len() - 1]
    } else {
        &PANEL_ROLES[..]
    };
    for role in roles {
        let has_finding = finding_role == Some(*role);
        let record = PanelRecord {
            schema_version: DELIVERY_SCHEMA_VERSION,
            role: *role,
            tree_hash: snapshot.root_repository.tree_hash.clone(),
            snapshot_sha256: snapshot_sha256.clone(),
            repository_set: snapshot.repository_bindings(),
            signoff: !has_finding,
            recommendations: if has_finding {
                vec!["correct the finding".to_owned()]
            } else {
                vec![]
            },
        };
        write_json(&records.join(format!("{}.json", role.as_str())), &record);
    }
    records
}

#[test]
fn complete_external_flow_builds_and_verifies_non_circular_seal() {
    let scratch = Scratch::new("complete");
    let (probe, manifest) = fixture(&scratch);
    let snapshot_path =
        create_snapshot(&probe, &manifest, Some(&scratch.path.join("state"))).expect("snapshot");
    import_passing_evidence(&scratch, &probe, &snapshot_path);
    let panel = write_panel(&scratch, &snapshot_path, None, false);
    validate_and_store_panel(&probe, &snapshot_path, &panel).expect("panel");
    let seal = construct_seal(&probe, &snapshot_path).expect("seal");
    verify_seal(&probe, &seal).expect("verify seal");

    let seal_json = fs::read_to_string(seal).expect("seal JSON");
    assert!(!seal_json.contains("seal_sha256"));
    assert!(!seal_json.contains("\"model\""));
}

#[test]
fn missing_panel_role_and_findings_prevent_sealing() {
    let missing = Scratch::new("missing-role");
    let (probe, manifest) = fixture(&missing);
    let snapshot =
        create_snapshot(&probe, &manifest, Some(&missing.path.join("state"))).expect("snapshot");
    import_passing_evidence(&missing, &probe, &snapshot);
    let records = write_panel(&missing, &snapshot, None, true);
    let error = validate_and_store_panel(&probe, &snapshot, &records).expect_err("missing role");
    assert!(error.to_string().contains("exactly 10"));

    let findings = Scratch::new("findings");
    let (probe, manifest) = fixture(&findings);
    let snapshot =
        create_snapshot(&probe, &manifest, Some(&findings.path.join("state"))).expect("snapshot");
    import_passing_evidence(&findings, &probe, &snapshot);
    let records = write_panel(
        &findings,
        &snapshot,
        Some(xtask::delivery::PanelRole::Security),
        false,
    );
    validate_and_store_panel(&probe, &snapshot, &records).expect("valid finding record");
    let error = construct_seal(&probe, &snapshot).expect_err("finding blocks seal");
    assert!(error.to_string().contains("has findings"));
}

#[test]
fn missing_failed_and_pending_evidence_prevent_sealing() {
    let missing = Scratch::new("missing-evidence");
    let (probe, manifest) = fixture(&missing);
    let snapshot =
        create_snapshot(&probe, &manifest, Some(&missing.path.join("state"))).expect("snapshot");
    let error = construct_seal(&probe, &snapshot).expect_err("missing evidence");
    assert!(error.to_string().contains("validation evidence"));

    for result_class in [EvidenceResultClass::Failed, EvidenceResultClass::Pending] {
        let scratch = Scratch::new("non-passing-evidence");
        let (probe, manifest) = fixture(&scratch);
        let snapshot = create_snapshot(&probe, &manifest, Some(&scratch.path.join("state")))
            .expect("snapshot");
        import_evidence_result(&scratch, &probe, &snapshot, result_class);
        let error = construct_seal(&probe, &snapshot).expect_err("non-passing evidence");
        assert!(error.to_string().contains("is not passed"));
    }
}

#[test]
fn evidence_command_hash_and_repository_state_fail_closed() {
    let scratch = Scratch::new("evidence-mismatch");
    let (probe, manifest) = fixture(&scratch);
    let snapshot_path =
        create_snapshot(&probe, &manifest, Some(&scratch.path.join("state"))).expect("snapshot");
    let snapshot = read_snapshot(&snapshot_path).expect("snapshot");
    let request = EvidenceImportRequest {
        schema_version: DELIVERY_SCHEMA_VERSION,
        id: "unit".to_owned(),
        command: "cargo test --workspace".to_owned(),
        result_class: EvidenceResultClass::Passed,
        timestamp: "2026-07-13T07:41:58Z".to_owned(),
        tree_hash: snapshot.root_repository.tree_hash,
        payload: EvidencePayloadSource {
            path: None,
            sha256: Some("b".repeat(64)),
            external_locator: None,
        },
    };
    let request_path = scratch.path.join("requests/mismatch.json");
    write_json(&request_path, &request);
    let error =
        import_evidence(&probe, &snapshot_path, &request_path).expect_err("command mismatch");
    assert!(error.to_string().contains("command digest mismatch"));

    probe.dirty.set(true);
    let error =
        verify_seal(&probe, &snapshot_path.with_file_name("seal.json")).expect_err("dirty state");
    assert!(error.to_string().contains("dirty worktree"));
}

#[test]
fn state_directory_inside_reviewed_repository_is_rejected() {
    let scratch = Scratch::new("path-rejection");
    let (probe, manifest) = fixture(&scratch);
    let repository = &manifest.root_repository.root;
    let error = create_snapshot(&probe, &manifest, Some(&repository.join("delivery-state")))
        .expect_err("repository state path");
    assert!(error.to_string().contains("must not be stored"));
}
