//! Immutable candidate snapshot creation (spec section 12.1, work item
//! `ADR046-delivery-002`).
//!
//! `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave snapshot` binds one wave's stack into a single
//! immutable candidate:
//!
//! * the exact base commit and head commit of every repository in the wave's
//!   stack, plus the tree that stack integrates to;
//! * the wave's dependency graph edges and repository set;
//! * digests of the generated artifacts, dependency metadata, and contract
//!   content the wave's identity depends on.
//!
//! Every value is read out of Git rather than out of the working tree, and a
//! repository with uncommitted tracked changes is refused: a candidate binds
//! committed content, because that is what the wave's pull requests carry.
//!
//! The three digests come from
//! [`CandidateMaterial::digests`](super::model::CandidateMaterial::digests) and
//! are never recomputed here. The consequence worth restating, because it is
//! deliberate: `content_id` and `candidate_id` exclude commit history, so a
//! history-only rebase reproduces the same candidate address and re-snapshots
//! in place, rewriting `snapshot.json` with a new `snapshot_sha256`. A snapshot
//! whose integrated content differs can never share that address, and this
//! module refuses to overwrite one that does.
//!
//! The snapshot is written only through
//! [`CandidateDir`](super::storage::CandidateDir), anchored under a state root
//! proven to sit outside every repository checkout and every Git working tree.

use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{CliOptions, WaveCommand, WorkflowOutput},
    model::{
        CandidateDigests, CandidateId, CandidateMaterial, ContentId, DependencyEdge, Fingerprint,
        GitObjectFormat, RepositoryRecord, SNAPSHOT_ARTIFACT_KIND, SnapshotSha256, ensure_schema,
        sha256_bytes, validate_git_ref, validate_identifier, validate_program_wave,
        validate_repo_relative_path, validate_repository_id,
    },
    storage::{CandidateDir, MAX_ARTIFACT_BYTES, MAX_JSON_BYTES, SNAPSHOT_FILE, StateRoot},
};

/// Ref used for a repository whose head was not named explicitly.
const DEFAULT_HEAD_REF: &str = "HEAD";

/// The immutable artifact `wave snapshot` writes.
///
/// It carries the wave's material and the three identifiers derived from it;
/// [`WaveSnapshot::verify`] rederives them, so a snapshot that has been edited
/// after the fact fails closed everywhere it is consumed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WaveSnapshot {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub content_id: ContentId,
    pub candidate_id: CandidateId,
    pub snapshot_sha256: SnapshotSha256,
    pub material: CandidateMaterial,
}

impl WaveSnapshot {
    /// Canonicalizes the material and binds it to its three digests.
    pub fn seal(mut material: CandidateMaterial) -> Result<Self> {
        material.canonicalize()?;
        let digests = material.digests()?;
        Ok(Self {
            artifact_kind: SNAPSHOT_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            content_id: digests.content_id,
            candidate_id: digests.candidate_id,
            snapshot_sha256: digests.snapshot_sha256,
            material,
        })
    }

    pub fn digests(&self) -> CandidateDigests {
        CandidateDigests {
            content_id: self.content_id.clone(),
            candidate_id: self.candidate_id.clone(),
            snapshot_sha256: self.snapshot_sha256.clone(),
        }
    }

    pub fn program(&self) -> &str {
        &self.material.program
    }

    pub fn wave(&self) -> &str {
        &self.material.wave
    }

    /// Rederives every digest from the recorded material and rejects a
    /// snapshot whose identifiers do not describe its own content.
    pub fn verify(&self) -> Result<()> {
        if self.artifact_kind != SNAPSHOT_ARTIFACT_KIND {
            return Err(DeliveryError::new(format!(
                "expected artifact kind {SNAPSHOT_ARTIFACT_KIND}, found {:?}",
                self.artifact_kind
            )));
        }
        ensure_schema(self.schema_version, "wave snapshot")?;
        if self.material.digests()? != self.digests() {
            return Err(DeliveryError::new(
                "wave snapshot identifiers do not match its recorded material",
            ));
        }
        Ok(())
    }
}

/// Routes `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave snapshot`.
pub fn run(args: &[String]) -> Result<WorkflowOutput> {
    let request = SnapshotRequest::parse(args)?;
    let root = StateRoot::prepare(&request.checkout_roots()?, request.state_dir.as_deref())?;
    run_with_root(&request, &root)
}

fn run_with_root(request: &SnapshotRequest, root: &StateRoot) -> Result<WorkflowOutput> {
    let snapshot = WaveSnapshot::seal(discover(request)?)?;
    let candidate = root.candidate(snapshot.wave(), &snapshot.candidate_id)?;
    write(&candidate, &snapshot)?;
    WorkflowOutput::ok(WaveCommand::Snapshot)
        .with_digests(&snapshot.digests())
        .with_artifact(&candidate, &candidate.snapshot_path())
}

/// Writes `snapshot.json` into the candidate directory.
///
/// A candidate address holds exactly one integrated content. Rewriting it with
/// the same `content_id` is how a history-only rebase re-binds its new base and
/// head; rewriting it with different content is refused.
pub fn write(candidate: &CandidateDir, snapshot: &WaveSnapshot) -> Result<()> {
    if candidate.candidate_id() != &snapshot.candidate_id {
        return Err(DeliveryError::new(
            "candidate directory does not address this snapshot's candidate ID",
        ));
    }
    if let Some(existing) = read(candidate)?
        && existing.content_id != snapshot.content_id
    {
        return Err(DeliveryError::new(format!(
            "candidate {} already holds a snapshot of different integrated content; \
             a content change must be snapshotted under its own candidate ID",
            snapshot.candidate_id
        )));
    }
    candidate.write_json(SNAPSHOT_FILE, snapshot)?;
    Ok(())
}

/// Reads and verifies the snapshot held by a candidate directory, if any.
pub fn read(candidate: &CandidateDir) -> Result<Option<WaveSnapshot>> {
    if !candidate.snapshot_path().is_file() {
        return Ok(None);
    }
    let snapshot: WaveSnapshot = candidate.read_json(SNAPSHOT_FILE)?;
    snapshot.verify()?;
    Ok(Some(snapshot))
}

/// Reads and verifies a snapshot from an explicit path.
///
/// Every later stage takes `--snapshot <path>`, so this is the one place a
/// snapshot enters the workflow from outside a candidate directory.
pub fn read_file(path: &Path) -> Result<WaveSnapshot> {
    let bytes = read_bounded(path, MAX_JSON_BYTES)?;
    let snapshot: WaveSnapshot = serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid wave snapshot: {error}")))?;
    snapshot.verify()?;
    Ok(snapshot)
}

/// Rederives a sealed candidate's content digests from the repositories as
/// they stand now.
///
/// The integration tree and the fingerprinted objects are reread so a content
/// change rederives to a different `content_id`/`candidate_id`, which is the
/// section 12.6 invalidation rule. The head commit is reread too, so a
/// history-only rebase - which preserves the tree but rewrites the commit -
/// reproduces the same candidate address while moving `snapshot_sha256`. That
/// asymmetry is what lets panel evidence (bound to `content_id`/`candidate_id`)
/// survive a rebase while validator-lane evidence (additionally bound to
/// `snapshot_sha256`) does not. The base commit is left as recorded because no
/// base ref is carried into rederivation; a moved head alone is sufficient to
/// invalidate the recorded snapshot digest.
pub fn rederive(
    snapshot: &WaveSnapshot,
    checkouts: &BTreeMap<String, PathBuf>,
) -> Result<CandidateDigests> {
    let mut material = snapshot.material.clone();
    let mut heads = BTreeMap::new();
    for repository in &mut material.repository_set {
        let root = checkouts.get(&repository.id).ok_or_else(|| {
            DeliveryError::usage(format!(
                "missing --repo mapping for repository {}",
                repository.id
            ))
        })?;
        let root = toplevel(root)?;
        ensure_committed(&root, &repository.id)?;
        let head = resolve(&root, DEFAULT_HEAD_REF, "^{commit}")?;
        repository.integration_tree_oid = resolve(&root, &head, "^{tree}")?;
        repository.head_oid = head.clone();
        heads.insert(repository.id.clone(), (root, head));
    }
    for list in [
        &mut material.generated_artifacts,
        &mut material.dependency_fingerprints,
        &mut material.contract_fingerprints,
    ] {
        for fingerprint in list.iter_mut() {
            let (root, head) = heads.get(&fingerprint.repository).ok_or_else(|| {
                DeliveryError::new(format!(
                    "fingerprint {} names repository {} outside the snapshot's repository set",
                    fingerprint.name, fingerprint.repository
                ))
            })?;
            fingerprint.sha256 = object_digest(root, head, &fingerprint.path)?;
        }
    }
    material.digests()
}

/// One repository the wave spans, as named on the command line.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RepositoryInput {
    id: String,
    checkout: PathBuf,
    base_ref: String,
    head_ref: String,
}

/// One object whose content participates in the wave's identity.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FingerprintInput {
    name: String,
    repository: String,
    path: String,
}

/// Parsed `wave snapshot` invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotRequest {
    program: String,
    wave: String,
    repositories: Vec<RepositoryInput>,
    dependency_graph: Vec<DependencyEdge>,
    generated_artifacts: Vec<FingerprintInput>,
    dependency_fingerprints: Vec<FingerprintInput>,
    contract_fingerprints: Vec<FingerprintInput>,
    state_dir: Option<PathBuf>,
}

impl SnapshotRequest {
    fn parse(args: &[String]) -> Result<Self> {
        let mut options = CliOptions::parse(args)?;
        let program = options.required_string("--program")?;
        let wave = options.required_string("--wave")?;
        let checkouts = options.repository_roots()?;
        let mut bases = pairs(options.repeated_strings("--base"), "--base")?;
        let mut heads = pairs(options.repeated_strings("--head"), "--head")?;
        let dependency_graph = edges(options.repeated_strings("--edge"))?;
        let generated_artifacts = fingerprints(options.repeated_strings("--generated"))?;
        let dependency_fingerprints = fingerprints(options.repeated_strings("--dependency"))?;
        let contract_fingerprints = fingerprints(options.repeated_strings("--contract"))?;
        let state_dir = options.optional_path("--state-dir")?;
        options.finish()?;

        validate_program_wave(&program, &wave)?;

        let mut repositories = Vec::with_capacity(checkouts.len());
        for (id, checkout) in checkouts {
            let base_ref = bases.remove(&id).ok_or_else(|| {
                DeliveryError::usage(format!("repository {id} needs a --base {id}=REV mapping"))
            })?;
            let head_ref = heads
                .remove(&id)
                .unwrap_or_else(|| DEFAULT_HEAD_REF.to_owned());
            validate_git_ref(&base_ref, "base revision")?;
            validate_git_ref(&head_ref, "head revision")?;
            repositories.push(RepositoryInput {
                id,
                checkout,
                base_ref,
                head_ref,
            });
        }
        for (option, leftover) in [("--base", bases), ("--head", heads)] {
            if let Some(id) = leftover.into_keys().next() {
                return Err(DeliveryError::usage(format!(
                    "{option} names repository {id}, which has no --repo mapping"
                )));
            }
        }
        let request = Self {
            program,
            wave,
            repositories,
            dependency_graph,
            generated_artifacts,
            dependency_fingerprints,
            contract_fingerprints,
            state_dir,
        };
        request.ensure_fingerprint_repositories()?;
        Ok(request)
    }

    fn ensure_fingerprint_repositories(&self) -> Result<()> {
        for fingerprint in self
            .generated_artifacts
            .iter()
            .chain(&self.dependency_fingerprints)
            .chain(&self.contract_fingerprints)
        {
            if !self
                .repositories
                .iter()
                .any(|repository| repository.id == fingerprint.repository)
            {
                return Err(DeliveryError::usage(format!(
                    "fingerprint {} names repository {}, which has no --repo mapping",
                    fingerprint.name, fingerprint.repository
                )));
            }
        }
        Ok(())
    }

    /// Git working-tree roots of every repository the wave spans, used to keep
    /// delivery state outside all of them.
    fn checkout_roots(&self) -> Result<Vec<PathBuf>> {
        self.repositories
            .iter()
            .map(|repository| toplevel(&repository.checkout))
            .collect()
    }
}

/// Reads every snapshot input out of Git.
fn discover(request: &SnapshotRequest) -> Result<CandidateMaterial> {
    let mut repository_set = Vec::with_capacity(request.repositories.len());
    let mut heads = BTreeMap::new();
    for input in &request.repositories {
        let root = toplevel(&input.checkout)?;
        ensure_committed(&root, &input.id)?;
        let object_format = object_format(&root)?;
        let base_oid = resolve(&root, &input.base_ref, "^{commit}")?;
        let head_oid = resolve(&root, &input.head_ref, "^{commit}")?;
        let integration_tree_oid = resolve(&root, &head_oid, "^{tree}")?;
        heads.insert(input.id.clone(), (root, head_oid.clone()));
        repository_set.push(RepositoryRecord {
            id: input.id.clone(),
            object_format,
            base_oid,
            head_oid,
            integration_tree_oid,
        });
    }
    let resolve_all = |inputs: &[FingerprintInput]| -> Result<Vec<Fingerprint>> {
        inputs
            .iter()
            .map(|input| {
                let (root, head) = heads
                    .get(&input.repository)
                    .expect("fingerprint repositories are checked at parse time");
                Ok(Fingerprint {
                    name: input.name.clone(),
                    repository: input.repository.clone(),
                    path: input.path.clone(),
                    sha256: object_digest(root, head, &input.path)?,
                })
            })
            .collect()
    };
    Ok(CandidateMaterial {
        program: request.program.clone(),
        wave: request.wave.clone(),
        repository_set,
        dependency_graph: request.dependency_graph.clone(),
        generated_artifacts: resolve_all(&request.generated_artifacts)?,
        dependency_fingerprints: resolve_all(&request.dependency_fingerprints)?,
        contract_fingerprints: resolve_all(&request.contract_fingerprints)?,
    })
}

fn pairs(values: Vec<String>, option: &str) -> Result<BTreeMap<String, String>> {
    let mut mapped = BTreeMap::new();
    for value in values {
        let (id, rest) = value.split_once('=').ok_or_else(|| {
            DeliveryError::usage(format!("{option} must use LOGICAL_ID=REVISION"))
        })?;
        validate_repository_id(id)?;
        if rest.is_empty() || mapped.insert(id.to_owned(), rest.to_owned()).is_some() {
            return Err(DeliveryError::usage(format!(
                "{option} has an empty value or repeats repository {id}"
            )));
        }
    }
    Ok(mapped)
}

fn edges(values: Vec<String>) -> Result<Vec<DependencyEdge>> {
    values
        .into_iter()
        .map(|value| {
            let (from, to) = value
                .split_once('=')
                .ok_or_else(|| DeliveryError::usage("--edge must use FROM=TO"))?;
            let edge = DependencyEdge {
                from: from.to_owned(),
                to: to.to_owned(),
            };
            edge.validate()?;
            Ok(edge)
        })
        .collect()
}

fn fingerprints(values: Vec<String>) -> Result<Vec<FingerprintInput>> {
    values
        .into_iter()
        .map(|value| {
            let (name, target) = value.split_once('=').ok_or_else(|| {
                DeliveryError::usage("a fingerprint must use NAME=LOGICAL_ID:PATH")
            })?;
            let (repository, path) = target.rsplit_once(':').ok_or_else(|| {
                DeliveryError::usage("a fingerprint must use NAME=LOGICAL_ID:PATH")
            })?;
            validate_identifier(name, "fingerprint name")?;
            validate_repository_id(repository)?;
            validate_repo_relative_path(Path::new(path))?;
            Ok(FingerprintInput {
                name: name.to_owned(),
                repository: repository.to_owned(),
                path: path.to_owned(),
            })
        })
        .collect()
}

/// Git working-tree root containing `path`.
fn toplevel(path: &Path) -> Result<PathBuf> {
    let root = PathBuf::from(git_text(path, &["rev-parse", "--show-toplevel"])?);
    if !root.is_absolute() {
        return Err(DeliveryError::environment(
            "git reported a relative working-tree root for the repository checkout",
        ));
    }
    std::fs::canonicalize(&root).map_err(|error| {
        DeliveryError::environment(format!(
            "cannot canonicalize the repository root git reported: {error}"
        ))
    })
}

fn object_format(root: &Path) -> Result<GitObjectFormat> {
    match git_text(root, &["rev-parse", "--show-object-format"])?.as_str() {
        "sha1" => Ok(GitObjectFormat::Sha1),
        "sha256" => Ok(GitObjectFormat::Sha256),
        other => Err(DeliveryError::new(format!(
            "unsupported Git object format {other:?} in the repository checkout"
        ))),
    }
}

/// A candidate binds committed content, so an unclean working tree is refused
/// rather than silently snapshotted as its last commit.
fn ensure_committed(root: &Path, id: &str) -> Result<()> {
    let status = git_output(root, &["status", "--porcelain", "--untracked-files=no"])?;
    if !status.is_empty() {
        return Err(DeliveryError::new(format!(
            "repository {id} has uncommitted tracked changes; a candidate binds committed content"
        )));
    }
    Ok(())
}

fn resolve(root: &Path, revision: &str, peel: &str) -> Result<String> {
    let spec = format!("{revision}{peel}");
    git_text(root, &["rev-parse", "--verify", "--quiet", &spec])
}

/// SHA-256 of one fingerprinted object as committed at `head`.
///
/// A blob is hashed byte-for-byte. A tree is hashed over its recursive
/// `ls-tree` listing, whose entries already carry each blob's object ID, so any
/// content change under the tree changes the digest.
fn object_digest(root: &Path, head: &str, path: &str) -> Result<String> {
    validate_repo_relative_path(Path::new(path))?;
    let spec = format!("{head}:{path}");
    let bytes = match git_text(root, &["cat-file", "-t", &spec])?.as_str() {
        "blob" => git_output(root, &["cat-file", "blob", &spec])?,
        "tree" => git_output(root, &["ls-tree", "-r", "-z", "--full-tree", &spec])?,
        other => {
            return Err(DeliveryError::new(format!(
                "fingerprinted path {path} is a {other}, which cannot be digested"
            )));
        }
    };
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(DeliveryError::new(format!(
            "fingerprinted path {path} exceeds {MAX_ARTIFACT_BYTES} bytes"
        )));
    }
    Ok(sha256_bytes(&bytes))
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        // Keep repeated reads from writing to a repository the integrator may
        // be using concurrently.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|error| {
            // Name the operation by its (repository-relative) git arguments and
            // the errno only. The absolute checkout root and any raw git stderr
            // are deliberately withheld: both routinely carry host paths, and
            // this diagnostic reaches operator stderr and CI logs verbatim.
            DeliveryError::environment(format!(
                "cannot run git {} in the repository checkout: {error}",
                args.join(" ")
            ))
        })?;
    if !output.status.success() {
        // Report the git operation and its exit status, never the checkout path
        // or the raw subprocess stderr (git error text routinely interpolates
        // absolute paths).
        return Err(DeliveryError::new(format!(
            "git {} failed in the repository checkout{}",
            args.join(" "),
            match output.status.code() {
                Some(code) => format!(" (exit status {code})"),
                None => String::new(),
            }
        )));
    }
    Ok(output.stdout)
}

fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    let stdout = git_output(root, args)?;
    let text = String::from_utf8(stdout).map_err(|_| {
        DeliveryError::new(format!(
            "git {} produced non-UTF-8 output in the repository checkout",
            args.join(" ")
        ))
    })?;
    let text = text.trim();
    if text.is_empty() {
        return Err(DeliveryError::new(format!(
            "git {} produced no output in the repository checkout",
            args.join(" ")
        )));
    }
    Ok(text.to_owned())
}

/// Bounded read of a delivery artifact addressed by an explicit path.
pub(crate) fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DeliveryError::new(
            "delivery artifact is not a regular file",
        ));
    }
    let mut buffer = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut buffer)?;
    if buffer.len() > limit {
        return Err(DeliveryError::new(format!(
            "delivery artifact exceeds {limit} bytes"
        )));
    }
    Ok(buffer)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::delivery::{
        DeliveryErrorKind,
        storage::tests::{Scratch, assert_no_absolute_path, repo_root},
    };

    /// A throwaway Git repository under the ignored build tree.
    pub(crate) struct GitFixture {
        scratch: Scratch,
    }

    impl GitFixture {
        pub(crate) fn new(label: &str) -> Self {
            let fixture = Self {
                scratch: Scratch::new(label),
            };
            std::fs::create_dir_all(fixture.repo()).expect("create repository directory");
            fixture.git(&["init", "--quiet", "--initial-branch=main"]);
            fixture.git(&["config", "user.name", "delivery-test"]);
            fixture.git(&["config", "user.email", "delivery-test@example.invalid"]);
            fixture.write("docs/spec.json", "{\"wave\":\"w0\"}\n");
            fixture.write("schemas/one.json", "{\"a\":1}\n");
            fixture.write("schemas/two.json", "{\"b\":2}\n");
            fixture.write("Cargo.lock", "# lock\n");
            fixture.commit("base commit");
            fixture
        }

        pub(crate) fn repo(&self) -> PathBuf {
            self.scratch.path.join("repo")
        }

        pub(crate) fn state(&self) -> PathBuf {
            self.scratch.path.join("state")
        }

        pub(crate) fn scratch(&self) -> &Path {
            &self.scratch.path
        }

        pub(crate) fn write(&self, relative: &str, contents: &str) {
            let path = self.repo().join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent directory");
            }
            std::fs::write(path, contents).expect("write fixture file");
        }

        pub(crate) fn commit(&self, message: &str) {
            self.git(&["add", "--all"]);
            self.git(&["commit", "--quiet", "--message", message]);
        }

        pub(crate) fn git(&self, args: &[&str]) {
            let status = Command::new("git")
                .arg("-C")
                .arg(self.repo())
                .args(args)
                // Hermetic: never read the operator's global or system config.
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00+00:00")
                .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00+00:00")
                .output()
                .expect("run git");
            assert!(
                status.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&status.stderr)
            );
        }

        pub(crate) fn head(&self) -> String {
            let output = Command::new("git")
                .arg("-C")
                .arg(self.repo())
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("run git");
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }

        /// Command line for a snapshot of this fixture.
        pub(crate) fn snapshot_args(&self) -> Vec<String> {
            [
                "--program",
                "ADR046",
                "--wave",
                "W0",
                "--repo",
                &format!("github.com/example/d2b={}", self.repo().display()),
                "--base",
                "github.com/example/d2b=HEAD",
                "--edge",
                "adr046-w0=adr046-w1",
                "--generated",
                "spec-set=github.com/example/d2b:docs/spec.json",
                "--dependency",
                "cargo-lock=github.com/example/d2b:Cargo.lock",
                "--contract",
                "schemas=github.com/example/d2b:schemas",
            ]
            .iter()
            .map(|value| (*value).to_owned())
            .collect()
        }
    }

    pub(crate) fn take(fixture: &GitFixture) -> WaveSnapshot {
        let request = SnapshotRequest::parse(&fixture.snapshot_args()).expect("parse request");
        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        run_with_root(&request, &root).expect("snapshot");
        read_file(
            &root
                .path()
                .join("W0")
                .join(current_candidate(fixture).as_str())
                .join(SNAPSHOT_FILE),
        )
        .expect("read snapshot back")
    }

    fn current_candidate(fixture: &GitFixture) -> CandidateId {
        let request = SnapshotRequest::parse(&fixture.snapshot_args()).expect("parse request");
        WaveSnapshot::seal(discover(&request).expect("discover"))
            .expect("seal")
            .candidate_id
    }

    fn snapshot_of(fixture: &GitFixture) -> WaveSnapshot {
        let request = SnapshotRequest::parse(&fixture.snapshot_args()).expect("parse request");
        WaveSnapshot::seal(discover(&request).expect("discover")).expect("seal")
    }

    #[test]
    fn a_failed_git_command_leaks_neither_the_checkout_path_nor_raw_git_stderr() {
        // Ask an established repository for an object that does not exist. Git
        // exits nonzero and writes its own diagnostic (prefixed `fatal:`) to
        // stderr, routinely interpolating absolute paths. The delivery
        // diagnostic reaches operator stderr and CI logs verbatim, so it must
        // name only the git operation and its exit status - never the absolute
        // checkout path, and never the raw subprocess stderr.
        let fixture = GitFixture::new("snapshot-git-redaction");
        let error = git_text(&fixture.repo(), &["cat-file", "-t", "HEAD:no/such/file"])
            .expect_err("cat-file on a missing object must fail");
        let message = error.message();
        assert_no_absolute_path(message, &[fixture.scratch(), &fixture.repo()]);
        assert!(
            message.contains("git cat-file -t HEAD:no/such/file"),
            "the diagnostic must name the git operation: {message}"
        );
        assert!(
            !message.contains("fatal:") && !message.to_ascii_lowercase().contains("not a git"),
            "the diagnostic must not echo raw git stderr: {message}"
        );
    }

    #[test]
    fn a_git_spawn_failure_names_the_operation_not_the_checkout_path() {
        // A bounded read of a directory that is not a regular file exercises
        // the sibling read-side leak class: the diagnostic names the artifact
        // role, never the absolute path.
        let scratch = Scratch::new("snapshot-read-redaction");
        let error = read_bounded(&scratch.path, MAX_ARTIFACT_BYTES)
            .expect_err("a directory must not read as a bounded artifact");
        assert_no_absolute_path(error.message(), &[&scratch.path]);
    }

    #[test]
    fn identical_inputs_produce_identical_digests() {
        let fixture = GitFixture::new("snapshot-stable");
        let first = snapshot_of(&fixture);
        let second = snapshot_of(&fixture);
        assert_eq!(first, second);
        assert_eq!(first.content_id, second.content_id);
        assert_eq!(first.candidate_id, second.candidate_id);
        assert_eq!(first.snapshot_sha256, second.snapshot_sha256);
    }

    #[test]
    fn option_ordering_does_not_change_the_digests() {
        let fixture = GitFixture::new("snapshot-ordering");
        let baseline = snapshot_of(&fixture);
        let mut reordered = fixture.snapshot_args();
        let tail = reordered.split_off(reordered.len() - 6);
        let mut shuffled = tail;
        shuffled.extend(reordered);
        let request = SnapshotRequest::parse(&shuffled).expect("parse reordered request");
        let shuffled = WaveSnapshot::seal(discover(&request).expect("discover")).expect("seal");
        assert_eq!(baseline, shuffled);
    }

    #[test]
    fn a_single_byte_content_change_produces_a_different_content_id() {
        let fixture = GitFixture::new("snapshot-content");
        let baseline = snapshot_of(&fixture);
        fixture.write("docs/spec.json", "{\"wave\":\"W0\"}\n");
        fixture.commit("flip one byte");
        let changed = snapshot_of(&fixture);
        assert_ne!(baseline.content_id, changed.content_id);
        assert_ne!(baseline.candidate_id, changed.candidate_id);
        assert_ne!(baseline.snapshot_sha256, changed.snapshot_sha256);
    }

    #[test]
    fn a_single_byte_change_under_a_fingerprinted_tree_changes_the_content_id() {
        let fixture = GitFixture::new("snapshot-tree");
        let baseline = snapshot_of(&fixture);
        fixture.write("schemas/two.json", "{\"b\":3}\n");
        fixture.commit("flip one byte under the fingerprinted tree");
        let changed = snapshot_of(&fixture);
        assert_ne!(baseline.content_id, changed.content_id);
    }

    #[test]
    fn a_history_only_rebase_keeps_the_candidate_address() {
        let fixture = GitFixture::new("snapshot-rebase");
        let baseline = take(&fixture);
        let before = fixture.head();
        fixture.git(&["commit", "--quiet", "--amend", "--message", "reworded"]);
        assert_ne!(before, fixture.head(), "the amend must rewrite history");
        let rebased = take(&fixture);

        assert_eq!(baseline.content_id, rebased.content_id);
        assert_eq!(baseline.candidate_id, rebased.candidate_id);
        assert_ne!(baseline.snapshot_sha256, rebased.snapshot_sha256);
        assert_eq!(
            std::fs::read_dir(fixture.state().join("W0"))
                .expect("list wave directory")
                .count(),
            1,
            "a history-only rebase rewrites the snapshot in place"
        );
    }

    #[test]
    fn a_real_git_rebase_keeps_the_panel_but_never_the_validator_lanes() {
        use crate::delivery::{
            evidence::{self, EvidenceLane},
            panel::{
                self,
                tests::{record_files, write_record_dir},
            },
            seal::{
                self,
                tests::{evidence as lane_evidence, import as import_evidence},
            },
            storage::{SEAL_FILE, tests::Scratch},
        };

        // Snapshot the wave from a real Git repository, then attest a
        // unanimous panel and import both validator lanes - every artifact
        // bound to the baseline history.
        let fixture = GitFixture::new("rebase-e2e");
        let baseline = take(&fixture);
        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let snapshot_path = root
            .path()
            .join("W0")
            .join(baseline.candidate_id.as_str())
            .join(SNAPSHOT_FILE);
        let (candidate, view) =
            panel::open_candidate(&root, &snapshot_path).expect("open candidate");

        panel::request(&candidate, &view).expect("panel request");
        let records = Scratch::new("rebase-e2e-records");
        let dir = write_record_dir(&records, &record_files(&view));
        panel::attest(&candidate, &view, &dir).expect("unanimous panel attests");
        import_evidence(
            &candidate,
            &lane_evidence(&view, EvidenceLane::GithubCi, "layer1-check"),
        );
        import_evidence(
            &candidate,
            &lane_evidence(&view, EvidenceLane::LocalHost, "make-test-integration"),
        );
        seal::seal(&candidate, &view).expect("the baseline candidate seals");

        // A real history-only rebase: amend HEAD so the commit OID changes
        // while the tree is byte-identical.
        let before = fixture.head();
        fixture.git(&[
            "commit",
            "--quiet",
            "--amend",
            "--message",
            "rebased onto the new base",
        ]);
        assert_ne!(before, fixture.head(), "the amend must rewrite history");

        // Rederiving from the rebased repository reproduces the content-only
        // address but moves the snapshot digest with the head.
        let checkouts = BTreeMap::from([("github.com/example/d2b".to_owned(), fixture.repo())]);
        let rederived = rederive(&baseline, &checkouts).expect("rederive after the rebase");
        assert_eq!(
            rederived.content_id, baseline.content_id,
            "content survives a history-only rebase"
        );
        assert_eq!(
            rederived.candidate_id, baseline.candidate_id,
            "the candidate address survives a history-only rebase"
        );
        assert_ne!(
            rederived.snapshot_sha256, baseline.snapshot_sha256,
            "the snapshot digest moves with the head"
        );

        // Re-snapshot in place so the candidate now holds the rebased history.
        let rebased = take(&fixture);
        assert_eq!(rebased.candidate_id, baseline.candidate_id);
        assert_ne!(rebased.snapshot_sha256, baseline.snapshot_sha256);
        let (candidate, rebased_view) =
            panel::open_candidate(&root, &snapshot_path).expect("reopen the rebased candidate");

        // Panel evidence SURVIVES: the ten records bind content, which the
        // rebase preserved, so they re-attest against the rebased snapshot.
        let request = panel::stored_request(&candidate, &rebased_view)
            .expect("the stored panel request still matches the rebased content");
        panel::attested_records(&candidate, &request)
            .expect("panel evidence survives a history-only rebase");

        // Validator evidence does NOT survive: the imported lanes bind the
        // baseline snapshot digest, so they read as stale against the rebased
        // snapshot and a reseal fails closed until every lane re-imports.
        let stale = evidence::require_passing_lanes(
            &candidate,
            &rebased_view.candidate_id,
            &rebased_view.content_id,
            &rebased_view.snapshot_sha256,
        )
        .expect_err("validator evidence must not survive a rebase");
        assert!(
            stale.message().contains("stale snapshot"),
            "unexpected message: {stale}"
        );
        seal::seal(&candidate, &rebased_view)
            .expect_err("a reseal must fail until every lane re-imports");

        // Once the lanes rerun and re-import against the rebased snapshot the
        // candidate seals again, proving the asymmetry end to end.
        import_evidence(
            &candidate,
            &lane_evidence(&rebased_view, EvidenceLane::GithubCi, "layer1-check"),
        );
        import_evidence(
            &candidate,
            &lane_evidence(
                &rebased_view,
                EvidenceLane::LocalHost,
                "make-test-integration",
            ),
        );
        seal::seal(&candidate, &rebased_view).expect("reseal after the lanes re-import");
        let record: seal::SealRecord = candidate.read_json(SEAL_FILE).expect("seal record");
        assert_eq!(record.snapshot_sha256, rebased_view.snapshot_sha256);
        assert_eq!(record.candidate_id, baseline.candidate_id);
    }

    #[test]
    fn a_snapshot_of_different_content_is_refused_at_the_same_address() {
        let fixture = GitFixture::new("snapshot-immutable");
        let snapshot = take(&fixture);
        let root = StateRoot::for_tests(&fixture.state()).expect("anchor state root");
        let candidate = root
            .existing_candidate("W0", &snapshot.candidate_id)
            .expect("candidate");

        let mut forged = snapshot.clone();
        forged.content_id = ContentId::parse("f".repeat(64)).expect("digest");
        let error = write(&candidate, &forged).expect_err("different content must be refused");
        assert!(
            error.message().contains("different integrated content"),
            "unexpected message: {error}"
        );
        assert_eq!(
            read(&candidate).expect("reread").expect("present"),
            snapshot,
            "the refused write must not have replaced the snapshot"
        );
    }

    #[test]
    fn a_tampered_snapshot_fails_verification() {
        let fixture = GitFixture::new("snapshot-tamper");
        let mut snapshot = take(&fixture);
        snapshot.verify().expect("an untouched snapshot verifies");
        snapshot.material.wave = "W1".to_owned();
        assert!(
            snapshot.verify().is_err(),
            "edited material must fail verification"
        );

        let mut wrong_kind = take(&fixture);
        wrong_kind.artifact_kind = "d2b-delivery/wave-seal".to_owned();
        assert!(wrong_kind.verify().is_err());

        let mut wrong_schema = take(&fixture);
        wrong_schema.schema_version = DELIVERY_SCHEMA_VERSION + 1;
        assert!(wrong_schema.verify().is_err());
    }

    #[test]
    fn the_snapshot_is_written_only_under_the_candidate_address() {
        let fixture = GitFixture::new("snapshot-address");
        let snapshot = take(&fixture);
        let expected = fixture
            .state()
            .join("W0")
            .join(snapshot.candidate_id.as_str())
            .join(SNAPSHOT_FILE);
        assert!(expected.is_file(), "snapshot must live at {expected:?}");
        assert_eq!(
            read_file(&expected).expect("read back"),
            snapshot,
            "the artifact must round-trip"
        );
    }

    #[test]
    fn an_uncommitted_change_is_refused() {
        let fixture = GitFixture::new("snapshot-dirty");
        fixture.write("docs/spec.json", "{\"wave\":\"dirty\"}\n");
        let request = SnapshotRequest::parse(&fixture.snapshot_args()).expect("parse request");
        let error = discover(&request).expect_err("a dirty checkout must be refused");
        assert!(
            error.message().contains("uncommitted tracked changes"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn an_untracked_file_does_not_block_a_snapshot() {
        let fixture = GitFixture::new("snapshot-untracked");
        let baseline = snapshot_of(&fixture);
        fixture.write("scratch.log", "validator output\n");
        assert_eq!(
            snapshot_of(&fixture),
            baseline,
            "an untracked file is not integrated content"
        );
    }

    #[test]
    fn a_repository_without_a_base_is_a_usage_error() {
        let fixture = GitFixture::new("snapshot-base");
        let args = fixture
            .snapshot_args()
            .into_iter()
            .filter(|value| value != "--base" && !value.ends_with("d2b=HEAD"))
            .collect::<Vec<_>>();
        let error = SnapshotRequest::parse(&args).expect_err("a missing base is a usage error");
        assert_eq!(error.kind(), DeliveryErrorKind::Usage);
        assert!(error.message().contains("--base"));
    }

    #[test]
    fn malformed_inputs_are_refused() {
        let fixture = GitFixture::new("snapshot-inputs");
        let cases: [(&str, &str); 5] = [
            ("--base", "not-a-repository=HEAD"),
            ("--head", "github.com/example/other=HEAD"),
            ("--edge", "adr046-w0"),
            ("--generated", "spec-set=github.com/example/d2b"),
            ("--contract", "schemas=github.com/example/d2b:../escape"),
        ];
        for (option, value) in cases {
            let mut args = fixture.snapshot_args();
            args.push(option.to_owned());
            args.push(value.to_owned());
            assert!(
                SnapshotRequest::parse(&args).is_err(),
                "{option} {value} must be refused"
            );
        }
    }

    #[test]
    fn a_state_root_inside_the_repository_is_refused_by_the_entry_point() {
        let fixture = GitFixture::new("snapshot-external");
        let mut args = fixture.snapshot_args();
        args.push("--state-dir".to_owned());
        args.push(fixture.repo().join("delivery-state").display().to_string());
        let error = run(&args).expect_err("state inside a checkout must be refused");
        assert!(
            error.message().contains("must not live inside"),
            "unexpected message: {error}"
        );
        assert!(!fixture.repo().join("delivery-state").exists());
    }

    #[test]
    fn a_state_root_inside_this_repository_is_refused_by_the_entry_point() {
        let fixture = GitFixture::new("snapshot-external-self");
        let mut args = fixture.snapshot_args();
        args.push("--state-dir".to_owned());
        args.push(
            repo_root()
                .join("packages/target/should-never-exist-snapshot")
                .display()
                .to_string(),
        );
        let error = run(&args).expect_err("state inside a Git working tree must be refused");
        assert!(
            error.message().contains("Git working tree")
                || error.message().contains("must not live inside"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn rederiving_unchanged_content_reproduces_the_candidate() {
        let fixture = GitFixture::new("snapshot-rederive");
        let snapshot = take(&fixture);
        let checkouts = BTreeMap::from([("github.com/example/d2b".to_owned(), fixture.repo())]);
        let rederived = rederive(&snapshot, &checkouts).expect("rederive");
        assert_eq!(rederived.content_id, snapshot.content_id);
        assert_eq!(rederived.candidate_id, snapshot.candidate_id);

        fixture.write("Cargo.lock", "# lock changed\n");
        fixture.commit("change dependency metadata");
        let rederived = rederive(&snapshot, &checkouts).expect("rederive");
        assert_ne!(rederived.content_id, snapshot.content_id);
    }

    #[test]
    fn rederiving_without_a_repository_mapping_is_a_usage_error() {
        let fixture = GitFixture::new("snapshot-rederive-usage");
        let snapshot = take(&fixture);
        let error = rederive(&snapshot, &BTreeMap::new()).expect_err("missing mapping");
        assert_eq!(error.kind(), DeliveryErrorKind::Usage);
    }

    #[test]
    fn a_bounded_read_refuses_an_oversized_artifact() {
        let fixture = GitFixture::new("snapshot-bounded");
        let path = fixture.scratch().join("big.json");
        std::fs::write(&path, vec![b'x'; 64]).expect("write");
        assert!(read_bounded(&path, 63).is_err());
        assert_eq!(read_bounded(&path, 64).expect("read").len(), 64);
    }
}
