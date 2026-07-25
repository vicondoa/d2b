//! External, candidate-ID-addressed delivery state.
//!
//! Spec sections 12.2 and 12.5 are absolute about where wave evidence lives:
//! validation command output, panel transcripts, and attestation payloads
//! never enter Git, generated source, a PR body, or a release archive. This
//! module is the only place delivery artifacts are written, and its API is
//! shaped so that constraint cannot be violated by a caller:
//!
//! * [`StateRoot::prepare`] resolves the state root, then refuses it unless it
//!   is outside every supplied repository checkout and outside every enclosing
//!   Git working tree.
//! * A [`CandidateDir`] is the only writable handle, it is anchored under a
//!   root that already passed that check, and every write takes a validated
//!   relative path. No public entry point accepts an absolute destination, so
//!   there is no way to address a file inside a reviewed checkout.
//!
//! The layout is:
//!
//! ```text
//! <state root>/<wave>/<candidate_id>/snapshot.json
//! <state root>/<wave>/<candidate_id>/evidence/...
//! <state root>/<wave>/<candidate_id>/panel/...
//! <state root>/<wave>/<candidate_id>/panel-request.json
//! <state root>/<wave>/<candidate_id>/seal.json
//! <state root>/<wave>/<candidate_id>/history-proof.json
//! ```
//!
//! # Implementation note: deliberate de-escalation from the reuse source
//!
//! The implementation this module was adapted from anchored every operation on
//! directory file descriptors, using `openat`, `mkdirat`, `renameat_with`, and
//! `fchmodat` through `rustix` and `nix`. This one uses plain `std` instead.
//! That is a deliberate tradeoff, recorded here so it is visible in review
//! rather than having to be reconstructed.
//!
//! Security properties preserved:
//!
//! * external-path refusal — the state root is rejected inside any declared
//!   repository checkout and inside any enclosing Git working tree;
//! * symlink-component rejection over the whole existing path prefix;
//! * `0700` directories and `0600` files, verified after creation;
//! * bounded reads, so a hostile artifact cannot exhaust memory;
//! * traversal-proof relative paths, so no caller can address a file outside
//!   the candidate directory.
//!
//! Properties **not** carried over:
//!
//! * TOCTOU-hardened file-descriptor anchoring. Path checks here are performed
//!   against names, so an attacker with write access to an ancestor directory
//!   could in principle swap a component between the check and the use.
//! * `geteuid` file-ownership verification. An existing state directory owned
//!   by another user is accepted as long as its mode is `0700`.
//!
//! Both would be additive changes and neither is load-bearing for the
//! not-in-Git guarantee this module exists to enforce. Delivery state lives
//! under a per-user state directory, and the threat model for it is operator
//! error, not a local attacker racing the integrator.

use std::{
    ffi::{OsStr, OsString},
    fs::{self, DirBuilder, File},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};

use super::{
    DeliveryError, DeliveryErrorKind, Result,
    model::{CandidateId, sha256_bytes, validate_identifier},
};

/// Suffix appended to the XDG state directory when no root is requested.
const STATE_DIRECTORY: &str = "d2b/delivery";
/// Directory mode for every delivery state directory.
const STATE_DIR_MODE: u32 = 0o700;
/// File mode for every delivery artifact.
const STATE_FILE_MODE: u32 = 0o600;
/// Bounded ancestor walk when looking for an enclosing Git working tree.
const MAX_GIT_ANCESTOR_WALK: usize = 64;

pub const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;

pub const SNAPSHOT_FILE: &str = "snapshot.json";
pub const PANEL_REQUEST_FILE: &str = "panel-request.json";
pub const SEAL_FILE: &str = "seal.json";
pub const HISTORY_PROOF_FILE: &str = "history-proof.json";
pub const EVIDENCE_DIR: &str = "evidence";
pub const PANEL_DIR: &str = "panel";

/// A delivery state root proven to sit outside every reviewed checkout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateRoot {
    path: PathBuf,
}

impl StateRoot {
    /// Resolves, validates, and creates the delivery state root.
    ///
    /// `repository_roots` are the checkouts the wave spans. The root is
    /// rejected when it is at or under any of them, when it sits inside a Git
    /// working tree, when any component is a symlink, or when it already
    /// exists with a mode other than `0700`.
    pub fn prepare(repository_roots: &[PathBuf], requested_root: Option<&Path>) -> Result<Self> {
        let root = match requested_root {
            Some(path) => absolute_path(path)?,
            None => default_state_root()?,
        };
        ensure_external_path(&root, repository_roots)?;
        create_private_dir(&root)?;
        let root = fs::canonicalize(&root).map_err(|error| {
            DeliveryError::environment(format!(
                "cannot canonicalize delivery state root {}: {error}",
                root.display()
            ))
        })?;
        // Re-check after resolution so a symlinked component cannot move the
        // realized root back inside a checkout.
        ensure_external_path(&root, repository_roots)?;
        verify_private_directory(&root)?;
        Ok(Self { path: root })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opens, creating if absent, the candidate directory for one wave.
    pub fn candidate(&self, wave: &str, candidate_id: &CandidateId) -> Result<CandidateDir> {
        validate_identifier(wave, "wave")?;
        let wave_dir = self.path.join(wave);
        create_private_dir(&wave_dir)?;
        let path = wave_dir.join(candidate_id.as_str());
        create_private_dir(&path)?;
        self.anchor(wave, candidate_id, path)
    }

    /// Opens an existing candidate directory, failing when it is absent.
    pub fn existing_candidate(
        &self,
        wave: &str,
        candidate_id: &CandidateId,
    ) -> Result<CandidateDir> {
        validate_identifier(wave, "wave")?;
        let path = self.path.join(wave).join(candidate_id.as_str());
        if !path.is_dir() {
            return Err(DeliveryError::new(format!(
                "no delivery state for wave {wave} candidate {candidate_id}"
            )));
        }
        self.anchor(wave, candidate_id, path)
    }

    fn anchor(
        &self,
        wave: &str,
        candidate_id: &CandidateId,
        path: PathBuf,
    ) -> Result<CandidateDir> {
        verify_private_directory(&path)?;
        let path = fs::canonicalize(&path)?;
        if !path.starts_with(&self.path) {
            return Err(DeliveryError::new(
                "candidate directory resolved outside the delivery state root",
            ));
        }
        Ok(CandidateDir {
            root: self.path.clone(),
            wave: wave.to_owned(),
            candidate_id: candidate_id.clone(),
            path,
        })
    }

    /// Anchors a root without the external-path check, for hermetic tests that
    /// keep their scratch state inside the ignored build directory.
    #[cfg(test)]
    fn for_tests(path: &Path) -> Result<Self> {
        create_private_dir(path)?;
        Ok(Self {
            path: fs::canonicalize(path)?,
        })
    }
}

/// A writable handle addressed by `candidate_id`.
///
/// Every accessor takes a relative path validated against traversal, so a
/// caller cannot escape the state root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateDir {
    root: PathBuf,
    wave: String,
    candidate_id: CandidateId,
    path: PathBuf,
}

impl CandidateDir {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn wave(&self) -> &str {
        &self.wave
    }

    pub fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    pub fn snapshot_path(&self) -> PathBuf {
        self.path.join(SNAPSHOT_FILE)
    }

    pub fn panel_request_path(&self) -> PathBuf {
        self.path.join(PANEL_REQUEST_FILE)
    }

    pub fn seal_path(&self) -> PathBuf {
        self.path.join(SEAL_FILE)
    }

    pub fn history_proof_path(&self) -> PathBuf {
        self.path.join(HISTORY_PROOF_FILE)
    }

    pub fn evidence_dir(&self) -> PathBuf {
        self.path.join(EVIDENCE_DIR)
    }

    pub fn panel_dir(&self) -> PathBuf {
        self.path.join(PANEL_DIR)
    }

    /// Resolves a relative artifact path under the candidate directory.
    pub fn resolve(&self, relative: impl AsRef<Path>) -> Result<PathBuf> {
        let relative = relative.as_ref();
        validate_anchored_relative(relative)?;
        Ok(self.path.join(relative))
    }

    /// Writes a JSON artifact and returns its SHA-256 digest.
    pub fn write_json<T: Serialize>(
        &self,
        relative: impl AsRef<Path>,
        value: &T,
    ) -> Result<String> {
        let bytes = serde_json::to_vec(value)?;
        self.write_bytes(relative, &bytes)
    }

    /// Writes raw artifact bytes and returns their SHA-256 digest.
    pub fn write_bytes(&self, relative: impl AsRef<Path>, bytes: &[u8]) -> Result<String> {
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(DeliveryError::new(format!(
                "delivery artifact exceeds {MAX_ARTIFACT_BYTES} bytes"
            )));
        }
        let path = self.resolve(relative)?;
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        let mut file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(STATE_FILE_MODE)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(sha256_bytes(bytes))
    }

    pub fn read_json<T: DeserializeOwned>(&self, relative: impl AsRef<Path>) -> Result<T> {
        let path = self.resolve(relative)?;
        let bytes = read_limited(&path, MAX_JSON_BYTES)?;
        serde_json::from_slice(&bytes).map_err(|error| {
            DeliveryError::new(format!("invalid JSON in {}: {error}", path.display()))
        })
    }

    pub fn read_bytes(&self, relative: impl AsRef<Path>) -> Result<Vec<u8>> {
        read_limited(&self.resolve(relative)?, MAX_ARTIFACT_BYTES)
    }

    /// Lists the entry names of a directory under the candidate directory.
    pub fn list(&self, relative: impl AsRef<Path>) -> Result<Vec<OsString>> {
        let path = self.resolve(relative)?;
        let mut names = fs::read_dir(&path)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }
}

fn default_state_root() -> Result<PathBuf> {
    default_state_root_from(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// Pure resolver behind [`default_state_root`], so the precedence rule is
/// testable without mutating process environment.
fn default_state_root_from(
    xdg_state_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf> {
    if let Some(value) = xdg_state_home.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(DeliveryError::environment(
                "XDG_STATE_HOME must be absolute",
            ));
        }
        return Ok(path.join(STATE_DIRECTORY));
    }
    let home = home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            DeliveryError::environment("HOME is required when XDG_STATE_HOME is unset")
        })?;
    if !home.is_absolute() {
        return Err(DeliveryError::environment("HOME must be absolute"));
    }
    Ok(home.join(".local/state").join(STATE_DIRECTORY))
}

/// Rejects any path that would place delivery evidence inside a reviewed
/// checkout or a Git working tree.
pub fn ensure_external_path(path: &Path, repository_roots: &[PathBuf]) -> Result<()> {
    let absolute = absolute_path(path)?;
    for root in repository_roots {
        let root = fs::canonicalize(root).map_err(|error| {
            DeliveryError::environment(format!(
                "cannot canonicalize repository root {}: {error}",
                root.display()
            ))
        })?;
        if absolute.starts_with(&root) {
            return Err(DeliveryError::new(format!(
                "delivery state must not live inside repository {}: {}",
                root.display(),
                absolute.display()
            )));
        }
    }
    if let Some(worktree) = enclosing_git_worktree(&absolute) {
        return Err(DeliveryError::new(format!(
            "delivery state must not live inside the Git working tree at {}: {}",
            worktree.display(),
            absolute.display()
        )));
    }
    reject_symlink_components(&absolute)
}

/// Returns the nearest ancestor holding a `.git` entry, if any. A worktree's
/// `.git` is a file rather than a directory, so both are matched, as is a path
/// inside the Git directory itself.
pub fn enclosing_git_worktree(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors().take(MAX_GIT_ANCESTOR_WALK) {
        if ancestor.file_name() == Some(OsStr::new(".git")) {
            return Some(ancestor.to_path_buf());
        }
        if ancestor.join(".git").symlink_metadata().is_ok() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

/// Lexically normalizes a path to absolute form, rejecting parent traversal.
pub fn absolute_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(DeliveryError::new(format!(
                    "path contains parent traversal: {}",
                    path.display()
                )));
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    if !normalized.is_absolute() {
        return Err(DeliveryError::new("delivery path is not absolute"));
    }
    Ok(normalized)
}

/// Rejects a path whose existing prefix traverses a symlink.
pub fn reject_symlink_components(path: &Path) -> Result<()> {
    let absolute = absolute_path(path)?;
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => current.push(component.as_os_str()),
            Component::CurDir | Component::ParentDir => {}
            Component::Normal(name) => {
                current.push(name);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(DeliveryError::new(format!(
                            "path contains a symlink component: {}",
                            current.display()
                        )));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) => {
                        return Err(DeliveryError::of(
                            DeliveryErrorKind::Environment,
                            format!("cannot inspect {}: {error}", current.display()),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    reject_symlink_components(path)?;
    if path.is_dir() {
        return verify_private_directory(path);
    }
    DirBuilder::new()
        .recursive(true)
        .mode(STATE_DIR_MODE)
        .create(path)
        .map_err(|error| {
            DeliveryError::environment(format!(
                "cannot create delivery state directory {}: {error}",
                path.display()
            ))
        })?;
    fs::set_permissions(path, fs::Permissions::from_mode(STATE_DIR_MODE))?;
    verify_private_directory(path)
}

fn verify_private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(DeliveryError::new(format!(
            "delivery state path is not a directory: {}",
            path.display()
        )));
    }
    if metadata.permissions().mode() & 0o777 != STATE_DIR_MODE {
        return Err(DeliveryError::new(format!(
            "delivery state directory must have mode 0700: {}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_anchored_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(DeliveryError::new(
            "delivery artifact path must be non-empty and relative",
        ));
    }
    if path.components().any(|component| {
        !matches!(component, Component::Normal(_)) || component.as_os_str() == OsStr::new(".git")
    }) {
        return Err(DeliveryError::new(format!(
            "delivery artifact path contains traversal or a Git component: {}",
            path.display()
        )));
    }
    Ok(())
}

fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DeliveryError::new(format!(
            "delivery artifact is not a regular file: {}",
            path.display()
        )));
    }
    let mut buffer = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut buffer)?;
    if buffer.len() > limit {
        return Err(DeliveryError::new(format!(
            "delivery artifact exceeds {limit} bytes: {}",
            path.display()
        )));
    }
    Ok(buffer)
}

/// SHA-256 of a delivery artifact on disk.
pub fn sha256_file(path: &Path) -> Result<String> {
    Ok(sha256_bytes(&read_limited(path, MAX_ARTIFACT_BYTES)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT_SCRATCH: AtomicU32 = AtomicU32::new(0);

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("xtask lives under packages/xtask")
            .to_path_buf()
    }

    /// Scratch directory inside the ignored build tree, so tests never touch
    /// a tracked path and never write outside the project.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(label: &str) -> Self {
            let ordinal = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
            let path = repo_root()
                .join("packages/target/xtask-delivery-tests")
                .join(format!("{label}-{}-{ordinal}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create scratch directory");
            Self { path }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn candidate_id() -> CandidateId {
        CandidateId::parse("a".repeat(64)).expect("hex digest")
    }

    #[test]
    fn a_state_root_inside_the_repository_is_refused() {
        let inside = repo_root().join("packages/target/should-never-exist");
        let error = StateRoot::prepare(&[repo_root()], Some(&inside))
            .expect_err("an in-repository state root must be refused");
        assert!(
            error.message().contains("must not live inside"),
            "unexpected message: {error}"
        );
        assert!(!inside.exists(), "refusal must not create the directory");
    }

    #[test]
    fn a_state_root_inside_a_git_working_tree_is_refused_without_declared_roots() {
        let inside = repo_root().join("packages/target/should-never-exist-git");
        let error = StateRoot::prepare(&[], Some(&inside))
            .expect_err("a state root inside a Git working tree must be refused");
        assert!(
            error.message().contains("Git working tree"),
            "unexpected message: {error}"
        );
        assert!(!inside.exists());
    }

    #[test]
    fn a_state_root_inside_the_git_directory_is_refused() {
        let inside = repo_root().join(".git/d2b-delivery");
        let error = StateRoot::prepare(&[], Some(&inside))
            .expect_err("a state root inside .git must be refused");
        assert!(
            error.message().contains("Git"),
            "unexpected message: {error}"
        );
        assert!(!inside.exists());
    }

    #[test]
    fn ensure_external_path_refuses_a_repository_subdirectory() {
        let root = repo_root();
        assert!(
            ensure_external_path(&root.join("packages/xtask"), std::slice::from_ref(&root))
                .is_err()
        );
        assert!(ensure_external_path(&root, std::slice::from_ref(&root)).is_err());
    }

    #[test]
    fn enclosing_git_worktree_finds_the_repository() {
        let found = enclosing_git_worktree(&repo_root().join("packages/xtask/src"))
            .expect("the repository is a Git working tree");
        assert_eq!(found, repo_root());
    }

    #[test]
    fn absolute_path_rejects_parent_traversal() {
        assert!(absolute_path(Path::new("/var/lib/../etc")).is_err());
    }

    #[test]
    fn the_default_state_root_prefers_xdg_state_home() {
        let root = default_state_root_from(Some(OsStr::new("/state")), Some(OsStr::new("/home/a")))
            .expect("resolve");
        assert_eq!(root, PathBuf::from("/state/d2b/delivery"));

        let root = default_state_root_from(None, Some(OsStr::new("/home/a"))).expect("resolve");
        assert_eq!(root, PathBuf::from("/home/a/.local/state/d2b/delivery"));

        assert!(default_state_root_from(Some(OsStr::new("state")), None).is_err());
        assert!(default_state_root_from(None, None).is_err());
    }

    #[test]
    fn candidate_directories_are_addressed_by_candidate_id() {
        let scratch = Scratch::new("addressing");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("w0", &candidate_id()).expect("candidate");
        assert_eq!(
            candidate.path(),
            root.path().join("w0").join(candidate_id().as_str())
        );
        assert_eq!(candidate.candidate_id(), &candidate_id());
        assert_eq!(
            fs::symlink_metadata(candidate.path())
                .expect("stat")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn artifacts_round_trip_through_the_candidate_directory() {
        let scratch = Scratch::new("round-trip");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("w0", &candidate_id()).expect("candidate");

        let digest = candidate
            .write_json(SNAPSHOT_FILE, &serde_json::json!({ "wave": "w0" }))
            .expect("write snapshot");
        assert_eq!(digest.len(), 64);
        let value: serde_json::Value = candidate.read_json(SNAPSHOT_FILE).expect("read snapshot");
        assert_eq!(value["wave"], "w0");

        candidate
            .write_bytes(Path::new(EVIDENCE_DIR).join("layer1.json"), b"{}")
            .expect("write evidence");
        assert_eq!(
            candidate.list(EVIDENCE_DIR).expect("list evidence"),
            vec![OsString::from("layer1.json")]
        );
        assert_eq!(
            fs::symlink_metadata(candidate.snapshot_path())
                .expect("stat")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn a_traversing_artifact_path_is_refused() {
        let scratch = Scratch::new("traversal");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("w0", &candidate_id()).expect("candidate");
        for relative in ["../escape.json", "/etc/escape.json", ".git/config", ""] {
            assert!(
                candidate.write_bytes(relative, b"x").is_err(),
                "{relative} must be refused"
            );
        }
    }

    #[test]
    fn an_absent_candidate_directory_is_not_created_by_existing_candidate() {
        let scratch = Scratch::new("absent");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        assert!(root.existing_candidate("w0", &candidate_id()).is_err());
        assert!(!root.path().join("w0").exists());
    }
}
