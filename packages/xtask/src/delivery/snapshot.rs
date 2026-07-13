use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::RepositoryProbe,
    model::{
        Fingerprint, FingerprintSpec, RepositoryRecord, RequiredValidation, RootRepository,
        StackManifest, StackNode, WaveSnapshot, validate_hash, validate_repo_relative_path,
    },
    storage::{
        StateLayout, ensure_external_path, sha256_bytes, sha256_file, verify_json_digest,
        write_immutable_json,
    },
};

#[derive(Clone, Debug)]
pub(crate) struct SnapshotContext {
    pub snapshot: WaveSnapshot,
    pub digest: String,
    pub layout: StateLayout,
    pub repository_roots: Vec<PathBuf>,
    pub git_common_dirs: Vec<PathBuf>,
}

pub fn create_snapshot<P: RepositoryProbe>(
    probe: &P,
    manifest: &StackManifest,
    state_root: Option<&Path>,
) -> Result<PathBuf> {
    manifest.validate()?;
    let repositories = resolve_repositories(probe, manifest)?;
    let repository_roots = repositories
        .values()
        .map(|repository| PathBuf::from(&repository.root))
        .collect::<Vec<_>>();
    let root_repository = repositories
        .get(&manifest.root_repository.name)
        .ok_or_else(|| DeliveryError::new("resolved repository set omitted root repository"))?;
    let declared_root = probe.canonical_root(&manifest.root_repository.root)?;
    if Path::new(&root_repository.root) != declared_root {
        return Err(DeliveryError::new(
            "root_repository root differs from its repository_set root",
        ));
    }
    let root_head = probe.resolve_commit(&declared_root, &manifest.root_repository.head)?;
    let root_tree = probe.resolve_tree(&declared_root, &manifest.root_repository.head)?;
    if root_head != root_repository.head_commit || root_tree != root_repository.tree_hash {
        return Err(DeliveryError::new(
            "root_repository head differs from its repository_set head",
        ));
    }
    let root = RootRepository {
        name: manifest.root_repository.name.clone(),
        root: path_string(&declared_root)?,
        base_commit: probe.resolve_commit(&declared_root, &manifest.root_repository.base)?,
        head_commit: root_head,
        tree_hash: root_tree,
    };

    let stack = manifest
        .stack
        .iter()
        .map(|node| {
            let repository = repositories.get(&node.repository).ok_or_else(|| {
                DeliveryError::new(format!("unknown repository {}", node.repository))
            })?;
            let root = Path::new(&repository.root);
            let head_commit = probe.resolve_commit(root, &node.head)?;
            let branch_commit = probe.resolve_commit(root, &node.branch)?;
            if branch_commit != head_commit {
                return Err(DeliveryError::new(format!(
                    "stack node {} branch does not point at its recorded head",
                    node.id
                )));
            }
            Ok(StackNode {
                id: node.id.clone(),
                repository: node.repository.clone(),
                branch: node.branch.clone(),
                pr: node.pr,
                head_commit,
                depends_on: node.depends_on.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut required_validations = manifest
        .required_validations
        .iter()
        .map(|validation| RequiredValidation {
            id: validation.id.clone(),
            command_sha256: sha256_bytes(validation.command.as_bytes()),
        })
        .collect::<Vec<_>>();
    required_validations.sort();
    let mut required_checks = manifest.required_checks.clone();
    required_checks.sort();

    let mut repository_set = repositories.into_values().collect::<Vec<_>>();
    repository_set.sort_by(|left, right| left.name.cmp(&right.name));
    let repository_map = repository_set
        .iter()
        .map(|repository| (repository.name.as_str(), Path::new(&repository.root)))
        .collect::<BTreeMap<_, _>>();
    let generated_artifacts = fingerprint_specs(
        "generated artifact",
        &manifest.generated_artifacts,
        &repository_map,
    )?;
    let dependency_fingerprints = fingerprint_specs(
        "dependency",
        &manifest.dependency_fingerprints,
        &repository_map,
    )?;
    let contract_fingerprints =
        fingerprint_specs("contract", &manifest.contract_fingerprints, &repository_map)?;
    let snapshot = WaveSnapshot {
        schema_version: DELIVERY_SCHEMA_VERSION,
        wave: manifest.wave.clone(),
        root_repository: root,
        repository_set,
        stack,
        required_validations,
        required_checks,
        generated_artifacts,
        dependency_fingerprints,
        contract_fingerprints,
    };
    snapshot.validate()?;

    let layout = StateLayout::create(
        probe,
        &declared_root,
        &repository_roots,
        state_root,
        &snapshot.wave,
        &snapshot.root_repository.tree_hash,
    )?;
    let path = layout.snapshot();
    write_immutable_json(&path, &snapshot)?;
    Ok(path)
}

pub fn read_snapshot(path: &Path) -> Result<WaveSnapshot> {
    let snapshot: WaveSnapshot = super::storage::read_json(path)?;
    snapshot.validate()?;
    Ok(snapshot)
}

pub(crate) fn load_snapshot_context<P: RepositoryProbe>(
    probe: &P,
    snapshot_path: &Path,
    verify_current: bool,
) -> Result<SnapshotContext> {
    let snapshot = read_snapshot(snapshot_path)?;
    let layout = StateLayout::from_snapshot_path(
        snapshot_path,
        &snapshot.wave,
        &snapshot.root_repository.tree_hash,
    )?;
    let repository_roots = snapshot
        .repository_set
        .iter()
        .map(|repository| {
            let recorded = PathBuf::from(&repository.root);
            let canonical = probe.canonical_root(&recorded)?;
            if canonical != recorded {
                return Err(DeliveryError::new(format!(
                    "snapshot repository root is not canonical: {}",
                    repository.root
                )));
            }
            Ok(canonical)
        })
        .collect::<Result<Vec<_>>>()?;
    let git_common_dirs = repository_roots
        .iter()
        .map(|root| probe.git_common_dir(root))
        .collect::<Result<Vec<_>>>()?;
    ensure_external_path(snapshot_path, &repository_roots, &git_common_dirs)?;
    let digest = verify_json_digest(snapshot_path)?;
    if verify_current {
        verify_current_content(probe, &snapshot)?;
    }
    Ok(SnapshotContext {
        snapshot,
        digest,
        layout,
        repository_roots,
        git_common_dirs,
    })
}

pub(crate) fn verify_current_content<P: RepositoryProbe>(
    probe: &P,
    snapshot: &WaveSnapshot,
) -> Result<()> {
    for repository in &snapshot.repository_set {
        let root = Path::new(&repository.root);
        if probe.is_dirty(root)? {
            return Err(DeliveryError::new(format!(
                "repository {} has a dirty worktree",
                repository.name
            )));
        }
        let current_tree = probe.resolve_tree(root, "HEAD")?;
        if current_tree != repository.tree_hash {
            return Err(DeliveryError::new(format!(
                "repository {} tree changed from snapshot",
                repository.name
            )));
        }
    }
    verify_fingerprints(
        "generated artifact",
        &snapshot.generated_artifacts,
        snapshot,
    )?;
    verify_fingerprints("dependency", &snapshot.dependency_fingerprints, snapshot)?;
    verify_fingerprints("contract", &snapshot.contract_fingerprints, snapshot)?;
    Ok(())
}

fn resolve_repositories<P: RepositoryProbe>(
    probe: &P,
    manifest: &StackManifest,
) -> Result<BTreeMap<String, RepositoryRecord>> {
    let mut repositories = BTreeMap::new();
    for spec in &manifest.repository_set {
        let root = probe.canonical_root(&spec.root)?;
        let head_commit = probe.resolve_commit(&root, &spec.head)?;
        let tree_hash = probe.resolve_tree(&root, &spec.head)?;
        validate_hash(&head_commit, "repository head")?;
        validate_hash(&tree_hash, "repository tree")?;
        let record = RepositoryRecord {
            name: spec.name.clone(),
            root: path_string(&root)?,
            head_commit,
            tree_hash,
        };
        if repositories.insert(spec.name.clone(), record).is_some() {
            return Err(DeliveryError::new(format!(
                "duplicate repository {}",
                spec.name
            )));
        }
    }
    Ok(repositories)
}

fn fingerprint_specs(
    label: &str,
    specs: &[FingerprintSpec],
    repositories: &BTreeMap<&str, &Path>,
) -> Result<Vec<Fingerprint>> {
    let mut fingerprints = specs
        .iter()
        .map(|spec| {
            validate_repo_relative_path(&spec.path)?;
            let root = repositories.get(spec.repository.as_str()).ok_or_else(|| {
                DeliveryError::new(format!(
                    "{label} {} references unknown repository {}",
                    spec.name, spec.repository
                ))
            })?;
            let source = root.join(&spec.path);
            let metadata = fs::symlink_metadata(&source).map_err(|error| {
                DeliveryError::new(format!(
                    "cannot inspect {label} {}: {error}",
                    source.display()
                ))
            })?;
            if !metadata.file_type().is_file() {
                return Err(DeliveryError::new(format!(
                    "{label} source must be a regular file: {}",
                    source.display()
                )));
            }
            Ok(Fingerprint {
                name: spec.name.clone(),
                repository: spec.repository.clone(),
                path: path_string(&spec.path)?,
                sha256: sha256_file(&source)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    fingerprints.sort();
    Ok(fingerprints)
}

fn verify_fingerprints(
    label: &str,
    fingerprints: &[Fingerprint],
    snapshot: &WaveSnapshot,
) -> Result<()> {
    let roots = snapshot
        .repository_set
        .iter()
        .map(|repository| (repository.name.as_str(), repository.root.as_str()))
        .collect::<BTreeMap<_, _>>();
    for fingerprint in fingerprints {
        let root = roots.get(fingerprint.repository.as_str()).ok_or_else(|| {
            DeliveryError::new(format!(
                "{label} {} references an absent repository",
                fingerprint.name
            ))
        })?;
        let source = Path::new(root).join(&fingerprint.path);
        let metadata = fs::symlink_metadata(&source).map_err(|error| {
            DeliveryError::new(format!(
                "cannot inspect {label} {}: {error}",
                source.display()
            ))
        })?;
        if !metadata.file_type().is_file() {
            return Err(DeliveryError::new(format!(
                "{label} source is no longer a regular file: {}",
                source.display()
            )));
        }
        if sha256_file(&source)? != fingerprint.sha256 {
            return Err(DeliveryError::new(format!(
                "{label} fingerprint changed: {}",
                fingerprint.name
            )));
        }
    }
    Ok(())
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DeliveryError::new("delivery path is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        collections::BTreeMap,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;
    use crate::delivery::model::{
        RepositorySpec, RequiredCheck, RequiredValidationSpec, RootRepositorySpec, StackNodeSpec,
    };

    static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(1);

    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("repository root")
                .to_path_buf();
            let path = repository.parent().expect("parent").join(format!(
                ".d2b-xtask-snapshot-test-{}-{}",
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
                .ok_or_else(|| DeliveryError::new("missing fake commit"))
        }

        fn resolve_tree(&self, _root: &Path, revision: &str) -> Result<String> {
            self.trees
                .get(revision)
                .cloned()
                .ok_or_else(|| DeliveryError::new("missing fake tree"))
        }

        fn is_dirty(&self, _root: &Path) -> Result<bool> {
            Ok(self.dirty.get())
        }
    }

    fn fixture(scratch: &Scratch) -> (FakeProbe, StackManifest) {
        let repository = scratch.path.join("repository");
        let common = scratch.path.join("git-common");
        fs::create_dir(&repository).expect("repository");
        fs::create_dir(&common).expect("common");
        fs::write(repository.join("generated.json"), b"generated\n").expect("artifact");
        let head = "1".repeat(40);
        let tree = "2".repeat(40);
        let probe = FakeProbe {
            commits: BTreeMap::from([
                ("base".to_owned(), "0".repeat(40)),
                ("head".to_owned(), head.clone()),
                ("feature".to_owned(), head),
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
            generated_artifacts: vec![FingerprintSpec {
                name: "generated".to_owned(),
                repository: "example/d2b".to_owned(),
                path: PathBuf::from("generated.json"),
            }],
            dependency_fingerprints: vec![],
            contract_fingerprints: vec![],
        };
        (probe, manifest)
    }

    #[test]
    fn creates_tree_addressed_snapshot_without_raw_command() {
        let scratch = Scratch::new();
        let (probe, manifest) = fixture(&scratch);
        let state = scratch.path.join("state");
        let path = create_snapshot(&probe, &manifest, Some(&state)).expect("create snapshot");
        assert!(path.to_string_lossy().contains(&"2".repeat(40)));
        let bytes = fs::read_to_string(&path).expect("snapshot");
        assert!(!bytes.contains("cargo test -p xtask"));
        assert!(bytes.contains("command_sha256"));
        load_snapshot_context(&probe, &path, true).expect("verify snapshot");
    }

    #[test]
    fn current_snapshot_rejects_dirty_repository() {
        let scratch = Scratch::new();
        let (probe, manifest) = fixture(&scratch);
        let path = create_snapshot(&probe, &manifest, Some(&scratch.path.join("state")))
            .expect("snapshot");
        probe.dirty.set(true);
        let error = load_snapshot_context(&probe, &path, true).expect_err("dirty");
        assert!(error.to_string().contains("dirty worktree"));
    }

    #[test]
    fn snapshot_digest_mismatch_is_rejected() {
        let scratch = Scratch::new();
        let (probe, manifest) = fixture(&scratch);
        let path = create_snapshot(&probe, &manifest, Some(&scratch.path.join("state")))
            .expect("snapshot");
        fs::write(
            super::super::storage::digest_path(&path).expect("sidecar"),
            "0".repeat(64),
        )
        .expect("corrupt sidecar");
        let error = load_snapshot_context(&probe, &path, false).expect_err("digest mismatch");
        assert!(error.to_string().contains("digest mismatch"));
    }
}
