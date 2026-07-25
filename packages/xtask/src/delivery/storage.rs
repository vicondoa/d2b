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
//! # Implementation note: path safety
//!
//! The write path anchors every operation on a chain of verified directory
//! file descriptors rather than resolving a pathname per syscall. A pathname
//! write is atomic only within whichever directory each syscall happens to
//! resolve; an attacker who replaces an intermediate directory or the parent
//! between validation and use can redirect the temp create, the rename, and
//! the parent fsync into another writable directory, including a reviewed
//! checkout. Anchoring closes that window:
//!
//! * external-path refusal - the state root is rejected inside any declared
//!   repository checkout and inside any enclosing Git working tree;
//! * directory-descriptor walk - the candidate directory is opened once, and
//!   every component beneath it is opened relative to its parent's descriptor
//!   with `O_DIRECTORY | O_NOFOLLOW` (or `openat2` with
//!   `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS` where the kernel provides it), so
//!   a symlinked component is rejected rather than traversed and no ancestor is
//!   ever re-resolved by name;
//! * each opened directory is verified by `fstat` to be a real `0700`
//!   directory owned by the current effective user before it is used;
//! * create-new temp then atomic rename, all fd-relative - the temp is created
//!   with `openat(O_CREAT | O_EXCL | O_NOFOLLOW)` in the pinned parent, verified
//!   by `fstat` to be a regular `0600` file owned by us, written, fsynced, and
//!   `renameat`d into place against the *same* pinned parent fd, which is then
//!   fsynced; a newly created directory has its parent fsynced too;
//! * an `O_NOFOLLOW` `fstatat` rejects a symlink planted at the leaf name, so a
//!   pre-planted symlink fails closed rather than being silently replaced;
//! * an RAII temp guard `unlinkat`s the temp against its pinned parent until a
//!   successful rename disarms it, so no failure path leaves a stale temp
//!   behind, and a cleanup failure is folded into the primary error;
//! * `0700` directories and `0600` files, verified after creation;
//! * bounded reads, so a hostile artifact cannot exhaust memory;
//! * traversal-proof relative paths, so no caller can address a file outside
//!   the candidate directory.
//!
//! Public write diagnostics name only the candidate-relative artifact key, so
//! a storage failure written to stderr never carries `HOME`, the local
//! username, or a checkout, store, or temp path.

use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{Read, Write},
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::{
            ffi::OsStringExt,
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags};
use serde::{Serialize, de::DeserializeOwned};

use super::{
    DeliveryError, DeliveryErrorKind, Result,
    model::{CandidateId, sha256_bytes, validate_wave},
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
            DeliveryError::environment(format!("cannot resolve the delivery state root: {error}"))
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

    /// Resolves an artifact reference a prior stage reported.
    ///
    /// An absolute path is taken verbatim, so an operator may still point a
    /// stage at any snapshot or seal on disk. A relative reference - exactly
    /// the `<wave>/<candidate>/<artifact>` value a prior stage printed in its
    /// `artifact` field (see [`CandidateDir::state_relative_key`]) - resolves
    /// under this state root, so one stage's output chains directly into the
    /// next stage's `--snapshot` or `--seal`.
    pub fn resolve_artifact_ref(&self, reference: &Path) -> PathBuf {
        if reference.is_absolute() {
            reference.to_path_buf()
        } else {
            self.path.join(reference)
        }
    }

    /// Opens, creating if absent, the candidate directory for one wave.
    ///
    /// The wave and candidate directories are created fd-relatively beneath a
    /// verified state-root anchor, and each directory's immediate parent is
    /// fsynced right after it is created, so the wave link in the root and the
    /// candidate link in the wave are durable before any artifact is written
    /// into the candidate.
    pub fn candidate(&self, wave: &str, candidate_id: &CandidateId) -> Result<CandidateDir> {
        validate_wave(wave)?;
        let root = open_anchored_directory(&self.path)?;
        let wave_fd = open_or_create_directory(root.as_fd(), wave)?;
        open_or_create_directory(wave_fd.as_fd(), candidate_id.as_str())?;
        let path = self.path.join(wave).join(candidate_id.as_str());
        self.anchor(wave, candidate_id, path)
    }

    /// Opens an existing candidate directory, failing when it is absent.
    pub fn existing_candidate(
        &self,
        wave: &str,
        candidate_id: &CandidateId,
    ) -> Result<CandidateDir> {
        validate_wave(wave)?;
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
        // Pin the candidate directory once by an anchored walk from `/` and
        // retain the descriptor for the lifetime of the handle. Every later
        // operation - snapshot and seal reads, evidence traversal, and the
        // final write - resolves relative to this same descriptor, so a
        // same-uid attacker cannot present a forged candidate subtree for the
        // reads and restore the legitimate tree before the write.
        let dir_fd = open_anchored_directory(&path)?;
        Ok(CandidateDir {
            root: self.path.clone(),
            wave: wave.to_owned(),
            candidate_id: candidate_id.clone(),
            path,
            dir_fd,
        })
    }

    /// Resolves an operator-supplied artifact reference to the candidate that
    /// owns it and reads that artifact through the candidate's pinned
    /// directory descriptor.
    ///
    /// The candidate address (wave and candidate id) is derived from the
    /// reference itself, never from the bytes at a supplied path: the
    /// reference must resolve to `<wave>/<candidate>/<artifact>` inside this
    /// state root, so there is no supplied-path read and no separate
    /// canonicalize-and-compare. The artifact is then read through the very
    /// descriptor that backs every other operation on the returned handle, so
    /// a same-uid attacker can neither forge the tree for this read nor
    /// redirect the candidate address to a directory it controls.
    pub fn open_candidate_artifact<T: DeserializeOwned>(
        &self,
        reference: &Path,
        artifact: &str,
        label: &str,
    ) -> Result<(CandidateDir, T)> {
        let (wave, candidate_id) = self.candidate_address(reference, artifact, label)?;
        let candidate = self.existing_candidate(&wave, &candidate_id)?;
        let value: T = candidate.read_json(artifact)?;
        Ok((candidate, value))
    }

    /// Derives the `(wave, candidate id)` address an artifact reference names,
    /// requiring it to resolve to `<wave>/<candidate>/<artifact>` under this
    /// state root.
    ///
    /// A reference outside the state root, one with the wrong depth, one whose
    /// leaf is not the expected artifact, or one carrying a traversal or
    /// non-UTF-8 component fails closed with the same redacted diagnostic, so
    /// no supplied path is trusted to name the candidate.
    fn candidate_address(
        &self,
        reference: &Path,
        artifact: &str,
        label: &str,
    ) -> Result<(String, CandidateId)> {
        let refuse = || {
            DeliveryError::new(format!(
                "the {label} must be the candidate's own artifact inside external delivery \
                 state, not the supplied path"
            ))
        };
        let relative = reference.strip_prefix(&self.path).map_err(|_| refuse())?;
        let mut names = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(name) => {
                    names.push(name.to_str().ok_or_else(refuse)?);
                }
                _ => return Err(refuse()),
            }
        }
        match names.as_slice() {
            [wave, candidate, leaf] if *leaf == artifact => {
                validate_wave(wave)?;
                let candidate_id = CandidateId::parse(*candidate)?;
                Ok(((*wave).to_owned(), candidate_id))
            }
            _ => Err(refuse()),
        }
    }

    /// Anchors a root without the external-path check, for hermetic tests that
    /// keep their scratch state inside the ignored build directory.
    #[cfg(test)]
    pub(crate) fn for_tests(path: &Path) -> Result<Self> {
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
///
/// Reads, listings, and writes all resolve on the *same* pinned inode chain:
/// the candidate directory is opened once, from `/` a component at a time with
/// `O_NOFOLLOW`, verified to be a real `0700` directory owned by us, and its
/// descriptor is retained on the handle. Every operation walks the components
/// beneath it relative to that pinned descriptor rather than re-resolving the
/// candidate pathname per call. The read side therefore has the identical
/// anchoring discipline as the write side, and the candidate directory itself
/// is never reopened: an attacker who controls a writable ancestor can no
/// longer swap the candidate subtree for a forged tree during a check-then-open
/// read, restore the legitimate tree before the write, and have forged
/// evidence sealed - a symlinked ancestor fails the initial walk with `ELOOP`,
/// a swapped inode fails the identity/mode/owner check, and a swap after the
/// pin has no effect because the pinned descriptor still names the original
/// inode.
#[derive(Debug)]
pub struct CandidateDir {
    root: PathBuf,
    wave: String,
    candidate_id: CandidateId,
    path: PathBuf,
    dir_fd: OwnedFd,
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

    /// The state-root-relative reference for an artifact path under this
    /// candidate directory: `<wave>/<candidate>/<artifact>`.
    ///
    /// This leaks no absolute path, `HOME`, username, checkout, or store path,
    /// yet names the wave and candidate, so a later stage resolves it under the
    /// same state root (see [`StateRoot::resolve_artifact_ref`]) and one
    /// stage's reported `artifact` chains directly into the next stage's
    /// `--snapshot` or `--seal` without a contributor reconstructing the path.
    pub fn state_relative_key(&self, path: &Path) -> Result<String> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            DeliveryError::new("delivery artifact is not under the delivery state root")
        })?;
        relative
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| DeliveryError::new("delivery artifact reference is not UTF-8"))
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
    ///
    /// The write is anchored on the candidate directory descriptor: every
    /// intermediate directory is opened, verified, and (when absent) created
    /// relative to its parent's descriptor, and the temp create, rename, and
    /// directory fsync all run against those pinned descriptors. A symlink
    /// swapped into any component after the walk is rejected rather than
    /// traversed, so a write can never be redirected into another directory.
    pub fn write_bytes(&self, relative: impl AsRef<Path>, bytes: &[u8]) -> Result<String> {
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(DeliveryError::new(format!(
                "delivery artifact exceeds {MAX_ARTIFACT_BYTES} bytes"
            )));
        }
        let relative = relative.as_ref();
        validate_anchored_relative(relative)?;
        write_anchored(self.dir_fd.as_fd(), relative, bytes)?;
        Ok(sha256_bytes(bytes))
    }

    pub fn read_json<T: DeserializeOwned>(&self, relative: impl AsRef<Path>) -> Result<T> {
        let relative = relative.as_ref();
        validate_anchored_relative(relative)?;
        let key = relative.to_string_lossy().into_owned();
        let bytes = read_limited_anchored(self.dir_fd.as_fd(), relative, MAX_JSON_BYTES, &key)?;
        serde_json::from_slice(&bytes).map_err(|error| {
            DeliveryError::new(format!("invalid JSON in delivery artifact {key}: {error}"))
        })
    }

    pub fn read_bytes(&self, relative: impl AsRef<Path>) -> Result<Vec<u8>> {
        let relative = relative.as_ref();
        validate_anchored_relative(relative)?;
        let key = relative.to_string_lossy().into_owned();
        read_limited_anchored(self.dir_fd.as_fd(), relative, MAX_ARTIFACT_BYTES, &key)
    }

    /// Lists the entry names of a directory under the candidate directory.
    ///
    /// The directory is reached by an fd-relative walk from the pinned
    /// candidate descriptor - the same inode chain the anchored writer uses -
    /// and its entries are read through the pinned descriptor, so the listing
    /// observes exactly the tree the writer produced rather than one a
    /// check-then-open race could swap in.
    pub fn list(&self, relative: impl AsRef<Path>) -> Result<Vec<OsString>> {
        let relative = relative.as_ref();
        validate_anchored_relative(relative)?;
        let key = relative.to_string_lossy().into_owned();
        list_anchored(self.dir_fd.as_fd(), relative, &key)
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
                "cannot resolve a declared repository checkout: {error}"
            ))
        })?;
        if absolute.starts_with(&root) {
            return Err(DeliveryError::new(
                "the delivery state root must not live inside a declared repository checkout",
            ));
        }
    }
    if enclosing_git_worktree(&absolute).is_some() {
        return Err(DeliveryError::new(
            "the delivery state root must not live inside a Git working tree",
        ));
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
                return Err(DeliveryError::new(
                    "a delivery path contains parent traversal",
                ));
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
                        return Err(DeliveryError::new(
                            "a delivery state path traverses a symlink component",
                        ));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) => {
                        return Err(DeliveryError::of(
                            DeliveryErrorKind::Environment,
                            format!("cannot inspect a delivery state path: {error}"),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Creates the delivery state root durably, walking its absolute path from the
/// filesystem root and fsyncing each parent after a `mkdir` so every new link
/// is persisted before the root is reported ready. See
/// [`create_anchored_private_dir`] for the anchored-walk contract.
fn create_private_dir(path: &Path) -> Result<()> {
    create_anchored_private_dir(path).map(|_fd| ())
}

fn verify_private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(DeliveryError::new(
            "a delivery state path is not a directory",
        ));
    }
    if metadata.permissions().mode() & 0o777 != STATE_DIR_MODE {
        return Err(DeliveryError::new(
            "a delivery state directory must have mode 0700",
        ));
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
        return Err(DeliveryError::new(
            "delivery artifact path contains traversal or a Git component",
        ));
    }
    Ok(())
}

/// Opening flags shared by every leaf open: never follow a final-component
/// symlink, and never leak the descriptor across an exec.
const LEAF_OPEN_FLAGS: i32 = nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A process-unique temp-file suffix for create-new-then-rename writes.
fn unique_suffix() -> String {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}.{counter}", std::process::id())
}

/// Confirms an opened handle is a regular file, optionally of an exact mode and
/// owned by the current effective user.
///
/// The checks run against the open descriptor (`fstat`), not the path, so they
/// describe the object the caller will actually read or write rather than a
/// name an attacker could have swapped after the component walk.
fn verify_regular_file(
    file: &File,
    label: &str,
    require_mode: Option<u32>,
    require_owner: bool,
) -> Result<()> {
    let metadata = file.metadata().map_err(|error| {
        DeliveryError::environment(format!("cannot stat delivery artifact {label}: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(DeliveryError::new(format!(
            "delivery artifact is not a regular file: {label}"
        )));
    }
    if let Some(mode) = require_mode
        && metadata.permissions().mode() & 0o777 != mode
    {
        return Err(DeliveryError::new(format!(
            "delivery artifact must have mode {mode:04o}: {label}"
        )));
    }
    if require_owner && metadata.uid() != nix::unistd::geteuid().as_raw() {
        return Err(DeliveryError::new(format!(
            "delivery artifact is not owned by the current user: {label}"
        )));
    }
    Ok(())
}

/// Writes bytes beneath the candidate directory, anchored on verified
/// directory descriptors the whole way down.
///
/// `anchor` is the candidate directory descriptor the caller pinned once (see
/// [`CandidateDir`]); every intermediate directory is opened (or created and
/// its parent fsynced) relative to its parent's descriptor and verified by
/// `fstat` to be a real `0700` directory owned by us. The temp create, the
/// rename, and the containing-directory fsync all run against the *same*
/// pinned parent descriptor, so replacing a directory by name after the walk
/// cannot redirect any of them. A per-syscall `O_NOFOLLOW` on each
/// single-component open is equivalent to `RESOLVE_NO_SYMLINKS`, and opening
/// each name relative to its parent descriptor is equivalent to
/// `RESOLVE_BENEATH`, so the walk cannot be diverted upward or through a
/// symlink.
fn write_anchored(anchor: BorrowedFd<'_>, relative: &Path, bytes: &[u8]) -> Result<()> {
    let relative_key = relative
        .to_str()
        .ok_or_else(|| DeliveryError::new("delivery artifact key is not UTF-8"))?
        .to_owned();

    let components: Vec<&OsStr> = relative
        .components()
        .map(|component| component.as_os_str())
        .collect();
    // `validate_anchored_relative` guarantees at least one `Normal` component
    // and no traversal, so a leaf always exists and every name is a plain
    // component.
    let (leaf, dirs) = components
        .split_last()
        .ok_or_else(|| DeliveryError::new("delivery artifact path has no leaf"))?;
    let leaf = leaf
        .to_str()
        .ok_or_else(|| DeliveryError::new("delivery artifact name is not UTF-8"))?;

    // Retain every directory descriptor so each open is anchored on a pinned
    // parent, never re-resolved by name. The candidate directory itself is the
    // caller's already-pinned `anchor`, so it is never reopened here.
    let mut chain: Vec<OwnedFd> = Vec::new();
    for dir in dirs {
        let name = dir
            .to_str()
            .ok_or_else(|| DeliveryError::new("delivery directory name is not UTF-8"))?;
        let child = {
            let parent = chain.last().map_or(anchor, OwnedFd::as_fd);
            open_or_create_directory(parent, name)?
        };
        chain.push(child);
    }

    let parent = chain.last().map_or(anchor, OwnedFd::as_fd);
    write_leaf(parent, leaf, &relative_key, bytes)
}

/// Splits an absolute, already-normalized path into its `Normal` component
/// names, rejecting a relative path or any non-`Normal` component (`.`, `..`,
/// a repeated root). Callers pass paths that have been through
/// [`absolute_path`], so this is a defense-in-depth decomposition rather than
/// the primary traversal guard.
fn absolute_components(path: &Path) -> Result<Vec<&OsStr>> {
    if !path.is_absolute() {
        return Err(DeliveryError::new(
            "a delivery state path must be absolute to anchor",
        ));
    }
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => names.push(name),
            _ => {
                return Err(DeliveryError::new(
                    "a delivery state path contains a traversal component",
                ));
            }
        }
    }
    if names.is_empty() {
        return Err(DeliveryError::new(
            "a delivery state path resolves to the filesystem root",
        ));
    }
    Ok(names)
}

/// Converts a single path component to UTF-8, rejecting a non-UTF-8 name so it
/// can never reach a diagnostic or be interpreted loosely.
fn component_name(name: &OsStr) -> Result<&str> {
    name.to_str()
        .ok_or_else(|| DeliveryError::new("a delivery state path component is not UTF-8"))
}

/// Opens `/` as the trusted root fd from which every anchored walk begins.
fn open_root_dir() -> Result<OwnedFd> {
    rustix::fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        DeliveryError::environment(format!("cannot open the filesystem root: {error}"))
    })
}

/// Opens the delivery candidate directory as the write anchor by walking its
/// absolute path from `/` one component at a time.
///
/// Each hop is an `openat` on the pinned parent fd with
/// `O_DIRECTORY|O_NOFOLLOW`, so an attacker cannot swap an intermediate
/// state-root or wave component for a symlink between the check and the write:
/// a symlinked component makes `openat` fail with `ELOOP` rather than
/// redirecting the walk. The final descriptor is verified to be a real `0700`
/// directory owned by us, and its `fstat` dev/inode is cross-checked against a
/// `statat(.., SYMLINK_NOFOLLOW)` of the same name in its parent so the pinned
/// inode is provably the one named on the path. The returned fd is retained for
/// the lifetime of the operation.
fn open_anchored_directory(directory: &Path) -> Result<OwnedFd> {
    let names = absolute_components(directory)?;
    let mut current = open_root_dir()?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    for (index, raw) in names.iter().enumerate() {
        let name = component_name(raw)?;
        let child =
            rustix::fs::openat(current.as_fd(), name, flags, Mode::empty()).map_err(|error| {
                DeliveryError::environment(format!(
                    "cannot open an anchored delivery state component: {error}"
                ))
            })?;
        if index + 1 == names.len() {
            verify_anchored_directory(child.as_fd(), "delivery candidate directory")?;
            verify_anchor_identity(current.as_fd(), name, child.as_fd())?;
        }
        current = child;
    }
    Ok(current)
}

/// Creates a delivery state directory durably by walking its absolute path from
/// `/`, creating each missing component fd-relatively and fsyncing its parent
/// after the `mkdir` so the new link is persisted before success is reported.
///
/// Only the final component is verified to be a `0700` directory owned by us;
/// ancestors (`/home`, `$HOME/.local`, ...) are opened loosely because they are
/// outside the delivery contract and often carry non-`0700` modes. Concurrent
/// creation is tolerated: see [`open_or_create_child`].
fn create_anchored_private_dir(path: &Path) -> Result<OwnedFd> {
    let names = absolute_components(path)?;
    let mut current = open_root_dir()?;
    let last = names.len() - 1;
    for (index, raw) in names.iter().enumerate() {
        let name = component_name(raw)?;
        let child = open_or_create_child(current.as_fd(), name)?;
        if index == last {
            verify_anchored_directory(child.as_fd(), "the delivery state root")?;
            verify_anchor_identity(current.as_fd(), name, child.as_fd())?;
        }
        current = child;
    }
    Ok(current)
}

/// Cross-checks that the descriptor pinned by an anchored walk is the very
/// inode named in its parent, defeating a swap of the final component between
/// its `openat` and first use.
///
/// `statat(parent, name, SYMLINK_NOFOLLOW)` describes the name as it resolves
/// now (without following a leaf symlink); `fstat(anchor)` describes the pinned
/// object. Matching device + inode proves they are the same file.
fn verify_anchor_identity(
    parent: BorrowedFd<'_>,
    name: &str,
    anchor: BorrowedFd<'_>,
) -> Result<()> {
    let by_name = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
        DeliveryError::environment(format!(
            "cannot re-stat an anchored delivery state component: {error}"
        ))
    })?;
    let pinned = rustix::fs::fstat(anchor).map_err(|error| {
        DeliveryError::environment(format!(
            "cannot stat a pinned delivery state descriptor: {error}"
        ))
    })?;
    if by_name.st_dev != pinned.st_dev || by_name.st_ino != pinned.st_ino {
        return Err(DeliveryError::new(
            "a delivery state component changed identity during the anchored walk",
        ));
    }
    Ok(())
}

/// Opens a child directory relative to `parent`, creating it (and fsyncing
/// `parent` so the new entry is durable) when it is absent. A concurrent writer
/// that wins the `mkdirat` race (`EEXIST`) is treated as success: the parent is
/// still fsynced and the directory reopened with the same no-follow flags. The
/// opened descriptor is NOT verified here so ancestors outside the delivery
/// contract can be walked; callers that own the leaf verify it.
fn open_or_create_child(parent: BorrowedFd<'_>, name: &str) -> Result<OwnedFd> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    match rustix::fs::openat(parent, name, flags, Mode::empty()) {
        Ok(fd) => Ok(fd),
        Err(rustix::io::Errno::NOENT) => {
            // Test-only barrier: fired after the component is observed absent
            // and before the `mkdirat`, so the concurrent-creation regression
            // can force both racing writers past `ENOENT` before either creates
            // the directory, guaranteeing exactly one hits the `EEXIST` branch.
            // Compiled out entirely in non-test builds.
            #[cfg(test)]
            create_race_hook::before_mkdirat();
            match rustix::fs::mkdirat(parent, name, Mode::from_bits_truncate(STATE_DIR_MODE)) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => {
                    #[cfg(test)]
                    create_race_hook::record_eexist();
                }
                Err(error) => {
                    return Err(DeliveryError::environment(format!(
                        "cannot create a delivery state directory: {error}"
                    )));
                }
            }
            rustix::fs::fsync(parent).map_err(|error| {
                DeliveryError::environment(format!(
                    "cannot fsync a delivery directory after creation: {error}"
                ))
            })?;
            rustix::fs::openat(parent, name, flags, Mode::empty()).map_err(|error| {
                DeliveryError::environment(format!(
                    "cannot open a delivery state directory: {error}"
                ))
            })
        }
        Err(error) => Err(DeliveryError::environment(format!(
            "cannot open a delivery state directory: {error}"
        ))),
    }
}

/// Test-only synchronization for the concurrent-creation regression.
///
/// [`open_or_create_child`] fires [`before_mkdirat`] after a component is
/// observed absent (`openat` returned `ENOENT`) and before the `mkdirat`. The
/// regression test installs a two-party barrier there so both racing writers
/// provably observe `ENOENT` before either creates the directory: a writer
/// cannot pass the barrier - and therefore cannot `mkdirat` - until the other
/// has also reached it, which it can only do after its own `ENOENT`. That makes
/// exactly one `mkdirat` win and the other take the `EEXIST` branch, the path
/// the regression must exercise.
///
/// The hook state is process-global, but tests run in parallel, so both the
/// barrier and the `EEXIST` counter are gated on a per-thread opt-in
/// ([`set_participant`]). Unrelated tests that create directories while the
/// hook is installed run the no-op path and never touch the barrier or the
/// counter. Compiled out entirely in non-test builds.
#[cfg(test)]
mod create_race_hook {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    type Hook = Arc<dyn Fn() + Send + Sync>;

    static HOOK: OnceLock<Mutex<Option<Hook>>> = OnceLock::new();
    static EEXIST_HITS: AtomicUsize = AtomicUsize::new(0);

    thread_local! {
        static PARTICIPATE: Cell<bool> = const { Cell::new(false) };
    }

    fn cell() -> &'static Mutex<Option<Hook>> {
        HOOK.get_or_init(|| Mutex::new(None))
    }

    fn is_participant() -> bool {
        PARTICIPATE.with(Cell::get)
    }

    /// Opts the current thread in (or out) of the installed hook. Only opted-in
    /// threads run the barrier and count `EEXIST` outcomes.
    pub(super) fn set_participant(on: bool) {
        PARTICIPATE.with(|flag| flag.set(on));
    }

    /// Installs (or clears with `None`) the pre-`mkdirat` hook and resets the
    /// `EEXIST` counter.
    pub(super) fn install(hook: Option<Hook>) {
        EEXIST_HITS.store(0, Ordering::SeqCst);
        *cell().lock().expect("create-race hook mutex") = hook;
    }

    /// Fired after `ENOENT` and before `mkdirat`. The hook is cloned out of the
    /// lock before being called so two racing writers can both run it
    /// concurrently (a barrier inside it must not be held behind the mutex).
    /// Only opted-in threads run it.
    pub(super) fn before_mkdirat() {
        if !is_participant() {
            return;
        }
        let hook = cell().lock().expect("create-race hook mutex").clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Records that the `EEXIST` branch was taken on an opted-in thread.
    pub(super) fn record_eexist() {
        if is_participant() {
            EEXIST_HITS.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The number of `EEXIST` outcomes observed since the last [`install`].
    pub(super) fn eexist_hits() -> usize {
        EEXIST_HITS.load(Ordering::SeqCst)
    }
}

/// Opens a child directory relative to `parent`, creating it durably when
/// absent (see [`open_or_create_child`]), then verifies the opened descriptor
/// is a real `0700` directory owned by us. Used for the delivery-owned wave,
/// candidate, and per-artifact subdirectories.
fn open_or_create_directory(parent: BorrowedFd<'_>, name: &str) -> Result<OwnedFd> {
    let fd = open_or_create_child(parent, name)?;
    verify_anchored_directory(fd.as_fd(), name)?;
    Ok(fd)
}

/// Confirms an opened directory descriptor is a real `0700` directory owned by
/// the current effective user, using `fstat` on the descriptor rather than a
/// path so it describes the object actually pinned. `label` is a fixed, safe
/// string or a delivery-owned component name; never an operator path.
fn verify_anchored_directory(fd: BorrowedFd<'_>, label: &str) -> Result<()> {
    let stat = rustix::fs::fstat(fd)
        .map_err(|error| DeliveryError::environment(format!("cannot stat {label}: {error}")))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Err(DeliveryError::new(format!("{label} is not a directory")));
    }
    if (stat.st_mode as u32) & 0o777 != STATE_DIR_MODE {
        return Err(DeliveryError::new(format!("{label} must have mode 0700")));
    }
    if stat.st_uid != nix::unistd::geteuid().as_raw() {
        return Err(DeliveryError::new(format!(
            "{label} is not owned by the current user"
        )));
    }
    Ok(())
}

/// Rejects a symlink planted at the leaf name in the pinned parent.
///
/// `renameat` replaces the destination name rather than following it, so
/// without this check a symlinked leaf would be silently replaced rather than
/// refused; the delivery contract is to fail closed on a symlinked leaf.
fn reject_leaf_symlink(parent: BorrowedFd<'_>, leaf: &str, relative_key: &str) -> Result<()> {
    match rustix::fs::statat(parent, leaf, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink {
                Err(DeliveryError::new(format!(
                    "delivery artifact leaf is a symlink: {relative_key}"
                )))
            } else {
                Ok(())
            }
        }
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(error) => Err(DeliveryError::environment(format!(
            "cannot inspect delivery artifact {relative_key}: {error}"
        ))),
    }
}

/// Writes bytes into a fresh temp in the pinned parent and atomically renames
/// it into place, all fd-relative to that parent.
///
/// The temp is created with `O_CREAT | O_EXCL | O_NOFOLLOW`, verified to be a
/// regular `0600` file owned by us, written, fsynced, and `renameat`d into the
/// leaf name against the *same* parent descriptor, which is then fsynced so the
/// rename is durable. A [`TempFile`] guard unlinks the temp against the parent
/// descriptor on every failure path until the rename disarms it, and folds a
/// cleanup failure into the returned error so no stale temp hides silently.
fn write_leaf(parent: BorrowedFd<'_>, leaf: &str, relative_key: &str, bytes: &[u8]) -> Result<()> {
    reject_leaf_symlink(parent, leaf, relative_key)?;

    let temp_name = format!(".{leaf}.tmp.{}", unique_suffix());
    let temp_fd = rustix::fs::openat(
        parent,
        temp_name.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_bits_truncate(STATE_FILE_MODE),
    )
    .map_err(|error| {
        DeliveryError::environment(format!(
            "cannot create delivery artifact {relative_key}: {error}"
        ))
    })?;

    let mut guard = TempFile::new(parent, temp_name);
    let mut file = File::from(temp_fd);

    // Pin the mode exactly, so an unusual umask cannot leave it wider than
    // 0600, then verify the descriptor before writing.
    if let Err(error) = file.set_permissions(fs::Permissions::from_mode(STATE_FILE_MODE)) {
        return Err(guard.fail(DeliveryError::environment(format!(
            "cannot set mode on delivery artifact {relative_key}: {error}"
        ))));
    }
    if let Err(error) = verify_regular_file(&file, relative_key, Some(STATE_FILE_MODE), true) {
        return Err(guard.fail(error));
    }

    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        return Err(guard.fail(DeliveryError::environment(format!(
            "cannot write delivery artifact {relative_key}: {error}"
        ))));
    }
    drop(file);

    if let Err(error) = rustix::fs::renameat(parent, guard.name(), parent, leaf) {
        return Err(guard.fail(DeliveryError::environment(format!(
            "cannot install delivery artifact {relative_key}: {error}"
        ))));
    }
    guard.disarm();

    rustix::fs::fsync(parent).map_err(|error| {
        DeliveryError::environment(format!(
            "cannot fsync delivery directory for {relative_key}: {error}"
        ))
    })
}

/// An RAII guard that unlinks a create-new temp against its pinned parent
/// descriptor until a successful rename disarms it.
///
/// The guard borrows the parent descriptor and unlinks by name with
/// `unlinkat`, so cleanup targets the same pinned directory the temp was
/// created in and can never be redirected. [`TempFile::fail`] folds a cleanup
/// failure into the primary error; [`Drop`] is a best-effort backstop for any
/// path that returns without calling `fail` or `disarm`.
struct TempFile<'parent> {
    parent: BorrowedFd<'parent>,
    name: String,
    armed: bool,
}

impl<'parent> TempFile<'parent> {
    fn new(parent: BorrowedFd<'parent>, name: String) -> Self {
        Self {
            parent,
            name,
            armed: true,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    /// Consumes a primary error, unlinks the temp, and folds any cleanup
    /// failure into the returned error so a stale temp cannot hide silently.
    fn fail(&mut self, primary: DeliveryError) -> DeliveryError {
        match self.unlink() {
            Some(cleanup) => DeliveryError::of(primary.kind(), format!("{primary}; {cleanup}")),
            None => primary,
        }
    }

    /// Best-effort unlink; returns a candidate-relative message on a
    /// non-`ENOENT` failure and disarms so `Drop` does not retry.
    fn unlink(&mut self) -> Option<String> {
        if !self.armed {
            return None;
        }
        self.armed = false;
        match rustix::fs::unlinkat(self.parent, self.name.as_str(), AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => None,
            Err(error) => Some(format!(
                "the temporary delivery artifact could not be removed: {error}"
            )),
        }
    }
}

impl Drop for TempFile<'_> {
    fn drop(&mut self) {
        let _ = self.unlink();
    }
}

fn read_limited(path: &Path, limit: usize, label: &str) -> Result<Vec<u8>> {
    reject_symlink_components(path)?;
    let file = File::options()
        .read(true)
        .custom_flags(LEAF_OPEN_FLAGS)
        .open(path)
        .map_err(|error| {
            DeliveryError::environment(format!("cannot open delivery artifact {label}: {error}"))
        })?;
    verify_regular_file(&file, label, None, false)?;
    let mut buffer = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut buffer)?;
    if buffer.len() > limit {
        return Err(DeliveryError::new(format!(
            "delivery artifact {label} exceeds {limit} bytes"
        )));
    }
    Ok(buffer)
}

/// Reads a bounded artifact beneath the candidate directory on the same
/// pinned inode chain the writer uses - the read-side mirror of
/// [`write_anchored`]. This closes the check-then-open window a path-based read
/// leaves: `anchor` is the candidate directory descriptor the caller pinned
/// once, [`open_anchored_leaf`] walks every intermediate directory relative to
/// it and verifies the leaf is a regular file, so a symlink or directory
/// swapped into any component after the pin is rejected rather than followed
/// and the read observes exactly the tree the writer produced.
fn read_limited_anchored(
    anchor: BorrowedFd<'_>,
    relative: &Path,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>> {
    let file = open_anchored_leaf(anchor, relative, label)?;
    let mut buffer = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut buffer)?;
    if buffer.len() > limit {
        return Err(DeliveryError::new(format!(
            "delivery artifact {label} exceeds {limit} bytes"
        )));
    }
    Ok(buffer)
}

/// Opens a leaf file for reading beneath the candidate directory, anchored on
/// verified directory descriptors the whole way down. `anchor` is the
/// candidate directory descriptor the caller pinned once (see
/// [`CandidateDir`]); every intermediate directory is opened relative to its
/// parent's descriptor and verified to be a real `0700` directory owned by us,
/// and the leaf is opened relative to the final pinned parent with
/// `O_RDONLY | O_NOFOLLOW` and verified to be a regular file.
fn open_anchored_leaf(anchor: BorrowedFd<'_>, relative: &Path, label: &str) -> Result<File> {
    let components: Vec<&OsStr> = relative
        .components()
        .map(|component| component.as_os_str())
        .collect();
    let (leaf, dirs) = components
        .split_last()
        .ok_or_else(|| DeliveryError::new("delivery artifact path has no leaf"))?;
    let leaf = leaf
        .to_str()
        .ok_or_else(|| DeliveryError::new("delivery artifact name is not UTF-8"))?;

    let mut chain: Vec<OwnedFd> = Vec::new();
    for dir in dirs {
        let name = dir
            .to_str()
            .ok_or_else(|| DeliveryError::new("delivery directory name is not UTF-8"))?;
        let parent = chain.last().map_or(anchor, OwnedFd::as_fd);
        chain.push(open_existing_directory(parent, name, label)?);
    }

    let parent = chain.last().map_or(anchor, OwnedFd::as_fd);
    let leaf_fd = rustix::fs::openat(
        parent,
        leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        DeliveryError::environment(format!("cannot open delivery artifact {label}: {error}"))
    })?;
    let file = File::from(leaf_fd);
    verify_regular_file(&file, label, None, false)?;
    Ok(file)
}

/// Lists the entry names of a directory beneath the candidate directory on the
/// same pinned inode chain the writer uses. `anchor` is the candidate
/// directory descriptor the caller pinned once, the target directory is
/// reached by an fd-relative walk, opened with `O_DIRECTORY | O_NOFOLLOW`, and
/// verified to be a real `0700` directory owned by us before its entries are
/// read through that pinned descriptor, so a swapped-in symlink or foreign
/// directory is rejected rather than listed.
fn list_anchored(anchor: BorrowedFd<'_>, relative: &Path, label: &str) -> Result<Vec<OsString>> {
    let components: Vec<&OsStr> = relative
        .components()
        .map(|component| component.as_os_str())
        .collect();

    // `validate_anchored_relative` guarantees at least one `Normal` component,
    // so the chain is never empty and the loop yields the target directory.
    let mut chain: Vec<OwnedFd> = Vec::new();
    for dir in &components {
        let name = dir
            .to_str()
            .ok_or_else(|| DeliveryError::new("delivery directory name is not UTF-8"))?;
        let parent = chain.last().map_or(anchor, OwnedFd::as_fd);
        chain.push(open_existing_directory(parent, name, label)?);
    }
    let dir_fd = chain
        .last()
        .ok_or_else(|| DeliveryError::new("delivery artifact directory path is empty"))?;

    let dir = rustix::fs::Dir::read_from(dir_fd.as_fd()).map_err(|error| {
        DeliveryError::environment(format!(
            "cannot list delivery artifact directory {label}: {error}"
        ))
    })?;
    let mut names = Vec::new();
    for entry in dir {
        let entry = entry.map_err(|error| {
            DeliveryError::environment(format!(
                "cannot read delivery artifact directory {label}: {error}"
            ))
        })?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        names.push(OsString::from_vec(raw.to_vec()));
    }
    names.sort();
    Ok(names)
}

/// Opens an existing child directory relative to `parent` with no-follow flags
/// and verifies it is a real `0700` directory owned by us. Unlike
/// [`open_or_create_directory`] it never creates: a missing directory fails
/// closed. Used by the anchored read and list walk so an intermediate artifact
/// directory is proven delivery-owned before its contents are read. `label` is
/// the candidate-relative artifact key, so a missing intermediate directory
/// still names the safe key rather than an absolute path.
fn open_existing_directory(parent: BorrowedFd<'_>, name: &str, label: &str) -> Result<OwnedFd> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = rustix::fs::openat(parent, name, flags, Mode::empty()).map_err(|error| {
        DeliveryError::environment(format!(
            "cannot open delivery artifact directory {label}: {error}"
        ))
    })?;
    verify_anchored_directory(fd.as_fd(), name)?;
    Ok(fd)
}

/// SHA-256 of a delivery artifact on disk.
pub fn sha256_file(path: &Path) -> Result<String> {
    Ok(sha256_bytes(&read_limited(
        path,
        MAX_ARTIFACT_BYTES,
        "delivery artifact",
    )?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT_SCRATCH: AtomicU32 = AtomicU32::new(0);

    pub(crate) fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("xtask lives under packages/xtask")
            .to_path_buf()
    }

    /// Scratch directory inside the ignored build tree, so tests never touch
    /// a tracked path and never write outside the project.
    pub(crate) struct Scratch {
        pub(crate) path: PathBuf,
    }

    impl Scratch {
        pub(crate) fn new(label: &str) -> Self {
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

    fn other_candidate_id() -> CandidateId {
        CandidateId::parse("b".repeat(64)).expect("hex digest")
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
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");
        assert_eq!(
            candidate.path(),
            root.path().join("W0").join(candidate_id().as_str())
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
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");

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
    fn an_artifact_reference_chains_under_the_state_root() {
        let scratch = Scratch::new("artifact-ref");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");
        candidate
            .write_json(SNAPSHOT_FILE, &serde_json::json!({ "wave": "w0" }))
            .expect("write snapshot");

        let reference = candidate
            .state_relative_key(&candidate.snapshot_path())
            .expect("state-relative reference");
        assert_eq!(
            reference,
            format!("W0/{}/snapshot.json", candidate_id().as_str())
        );
        assert!(
            !reference.starts_with('/'),
            "the reference must not be an absolute path: {reference}"
        );
        assert!(
            !reference.contains(&scratch.path.display().to_string()),
            "the reference must not leak the state path: {reference}"
        );

        // A later stage resolves that reference to the same file, so one
        // stage's reported artifact chains straight into the next.
        assert_eq!(
            root.resolve_artifact_ref(Path::new(&reference)),
            candidate.snapshot_path()
        );
        // An absolute path is still honored verbatim.
        let absolute = candidate.snapshot_path();
        assert_eq!(root.resolve_artifact_ref(&absolute), absolute);
    }

    #[test]
    fn open_candidate_artifact_derives_the_address_from_the_reference() {
        let scratch = Scratch::new("open-artifact");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");
        candidate
            .write_json(SNAPSHOT_FILE, &serde_json::json!({ "wave": "w0" }))
            .expect("write snapshot");

        // The reference resolves to `<wave>/<candidate>/snapshot.json`; the
        // address is derived from it and the artifact is read through the
        // pinned candidate descriptor, not from the supplied path.
        let reference = root.resolve_artifact_ref(Path::new(&format!(
            "W0/{}/snapshot.json",
            candidate_id().as_str()
        )));
        let (opened, value): (CandidateDir, serde_json::Value) = root
            .open_candidate_artifact(&reference, SNAPSHOT_FILE, "candidate snapshot")
            .expect("open the candidate from its reference");
        assert_eq!(opened.candidate_id(), &candidate_id());
        assert_eq!(value["wave"], "w0");

        // A reference whose leaf is not the expected artifact is refused.
        let wrong_leaf = root.resolve_artifact_ref(Path::new(&format!(
            "W0/{}/seal.json",
            candidate_id().as_str()
        )));
        let error = root
            .open_candidate_artifact::<serde_json::Value>(
                &wrong_leaf,
                SNAPSHOT_FILE,
                "candidate snapshot",
            )
            .expect_err("a mismatched artifact leaf must be refused");
        assert!(
            error.message().contains("external delivery state"),
            "{error}"
        );

        // A reference of the wrong depth is refused.
        let wrong_depth =
            root.resolve_artifact_ref(Path::new(&format!("W0/{}", candidate_id().as_str())));
        assert!(
            root.open_candidate_artifact::<serde_json::Value>(
                &wrong_depth,
                SNAPSHOT_FILE,
                "candidate snapshot",
            )
            .is_err(),
            "a reference of the wrong depth must be refused"
        );

        // A supplied path outside the state root is refused, and the diagnostic
        // names only the semantic label.
        let foreign = scratch.path.join("foreign").join("snapshot.json");
        fs::create_dir_all(foreign.parent().expect("parent")).expect("foreign dir");
        fs::write(&foreign, b"{}").expect("foreign snapshot");
        let error = root
            .open_candidate_artifact::<serde_json::Value>(
                &foreign,
                SNAPSHOT_FILE,
                "candidate snapshot",
            )
            .expect_err("a supplied path outside the state root must be refused");
        assert!(
            error.message().contains("external delivery state"),
            "{error}"
        );
        assert_no_absolute_path(error.message(), &[&scratch.path, root.path()]);
    }

    #[test]
    fn a_traversing_artifact_path_is_refused() {
        let scratch = Scratch::new("traversal");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");
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
        assert!(root.existing_candidate("W0", &candidate_id()).is_err());
        assert!(!root.path().join("W0").exists());
    }

    #[test]
    fn a_symlinked_artifact_leaf_is_refused_and_its_target_is_untouched() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new("symlink-leaf");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");

        // A file outside delivery state that a hostile symlink aims at. Writing
        // through the symlink would truncate it, breaking the never-in-Git
        // guarantee if it lived inside a checkout.
        let victim = scratch.path.join("victim.txt");
        fs::write(&victim, b"precious").expect("write victim");

        // Seed the evidence directory through the writer so it exists at 0700,
        // then plant a symlink at the artifact leaf.
        candidate
            .write_bytes(Path::new(EVIDENCE_DIR).join("seed.json"), b"{}")
            .expect("seed evidence directory");
        let leaf = candidate.path().join(EVIDENCE_DIR).join("layer1.json");
        symlink(&victim, &leaf).expect("plant the leaf symlink");

        let relative = Path::new(EVIDENCE_DIR).join("layer1.json");
        let error = candidate
            .write_bytes(&relative, b"{}")
            .expect_err("a symlinked leaf must be refused");
        assert!(error.message().contains("symlink"), "{error}");
        assert_eq!(
            fs::read(&victim).expect("read victim"),
            b"precious",
            "the symlink target must not be truncated or overwritten"
        );
        assert!(
            candidate.read_bytes(&relative).is_err(),
            "reading through a symlinked leaf must be refused"
        );
    }

    #[test]
    fn a_symlinked_intermediate_directory_is_refused_for_read_write_and_list() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new("symlink-dir");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");

        // A real directory outside delivery state, then a symlink standing in
        // for the candidate's evidence directory that points at it.
        let elsewhere = scratch.path.join("elsewhere");
        fs::create_dir_all(&elsewhere).expect("create the external directory");
        let evidence = candidate.path().join(EVIDENCE_DIR);
        symlink(&elsewhere, &evidence).expect("plant the directory symlink");

        let relative = Path::new(EVIDENCE_DIR).join("layer1.json");
        assert!(
            candidate.write_bytes(&relative, b"{}").is_err(),
            "writing through a symlinked directory must be refused"
        );
        assert!(
            !elsewhere.join("layer1.json").exists(),
            "nothing may be written into the symlink target"
        );
        assert!(
            candidate.read_bytes(&relative).is_err(),
            "reading through a symlinked directory must be refused"
        );
        assert!(
            candidate.list(EVIDENCE_DIR).is_err(),
            "listing through a symlinked directory must be refused"
        );
    }

    #[test]
    fn a_write_replaces_an_artifact_atomically() {
        let scratch = Scratch::new("atomic-replace");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");
        let relative = Path::new(EVIDENCE_DIR).join("layer1.json");
        candidate
            .write_bytes(&relative, b"first")
            .expect("first write");
        candidate
            .write_bytes(&relative, b"second")
            .expect("second write");
        assert_eq!(candidate.read_bytes(&relative).expect("read"), b"second");
        // The replace leaves no temp files behind.
        assert_eq!(
            candidate.list(EVIDENCE_DIR).expect("list"),
            vec![OsString::from("layer1.json")]
        );
    }

    #[test]
    fn a_failed_rename_leaves_no_temp_behind() {
        let scratch = Scratch::new("rename-failure");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");

        // Seed the evidence directory, then plant a *directory* at the artifact
        // leaf so the final rename fails after the temp has been created.
        candidate
            .write_bytes(Path::new(EVIDENCE_DIR).join("seed.json"), b"{}")
            .expect("seed evidence directory");
        let leaf_dir = candidate.path().join(EVIDENCE_DIR).join("layer1.json");
        fs::create_dir(&leaf_dir).expect("plant a directory at the leaf");

        let relative = Path::new(EVIDENCE_DIR).join("layer1.json");
        let error = candidate
            .write_bytes(&relative, b"{}")
            .expect_err("a rename onto a directory must fail");
        assert_eq!(error.kind(), DeliveryErrorKind::Environment);

        // The RAII guard removed the temp: only the seed file and the planted
        // directory remain, with no `.layer1.json.tmp.*` leftover.
        let names = candidate.list(EVIDENCE_DIR).expect("list evidence");
        assert!(
            names
                .iter()
                .all(|name| !name.to_string_lossy().starts_with(".layer1.json.tmp")),
            "a failed rename must not leave a temp behind: {names:?}"
        );
    }

    #[test]
    fn a_write_failure_diagnostic_carries_no_absolute_path() {
        let scratch = Scratch::new("leak-free-error");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");

        candidate
            .write_bytes(Path::new(EVIDENCE_DIR).join("seed.json"), b"{}")
            .expect("seed evidence directory");
        let leaf_dir = candidate.path().join(EVIDENCE_DIR).join("layer1.json");
        fs::create_dir(&leaf_dir).expect("plant a directory at the leaf");

        let relative = Path::new(EVIDENCE_DIR).join("layer1.json");
        let error = candidate
            .write_bytes(&relative, b"{}")
            .expect_err("the write must fail");

        // `run_cli` writes `error` verbatim to stderr, so asserting on the
        // message is equivalent to asserting on failure stderr. It must carry
        // no absolute path: not the state root, the candidate directory, the
        // scratch checkout, `HOME`, the build tree, or a temp name.
        let message = error.message();
        for leaked in [
            candidate.path().to_string_lossy(),
            root.path().to_string_lossy(),
            scratch.path.to_string_lossy(),
        ] {
            assert!(
                !message.contains(leaked.as_ref()),
                "a write failure must not leak {leaked}: {message}"
            );
        }
        assert!(
            !message.contains(".tmp.")
                && !message.contains("/home")
                && !message.contains("/target/"),
            "a write failure must not leak HOME, a build path, or a temp path: {message}"
        );
        assert!(
            message.contains("evidence/layer1.json"),
            "a write failure must name the candidate-relative key: {message}"
        );
    }

    /// Asserts a public diagnostic carries no absolute path: not one of the
    /// supplied roots, nor `HOME`, the build tree, or a temp name. Shared by
    /// the per-error-class redaction tests so every class is held to the same
    /// bar the write path already meets.
    pub(crate) fn assert_no_absolute_path(message: &str, roots: &[&Path]) {
        for root in roots {
            let root = root.to_string_lossy();
            assert!(
                !message.contains(root.as_ref()),
                "a diagnostic must not leak {root}: {message}"
            );
        }
        assert!(
            !message.contains(".tmp.")
                && !message.contains("/home")
                && !message.contains("/target/"),
            "a diagnostic must not leak HOME, a build path, or a temp path: {message}"
        );
    }

    #[test]
    fn an_ancestor_swapped_after_the_pin_cannot_redirect_operations() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let scratch = Scratch::new("pinned-candidate-swap");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let cand = candidate_id();
        let candidate = root.candidate("W0", &cand).expect("candidate");
        candidate
            .write_bytes(Path::new(EVIDENCE_DIR).join("seed.json"), b"legit-content")
            .expect("seed evidence directory");

        // Build a fully valid-looking forged wave tree: same candidate id, same
        // 0700 owner and mode a re-walk would accept, but forged bytes. This is
        // exactly the tree the finding's attack presents.
        let forged_wave = root.path().join("W0.forged");
        let forged_candidate = forged_wave.join(cand.as_str());
        let forged_evidence = forged_candidate.join(EVIDENCE_DIR);
        fs::create_dir_all(&forged_evidence).expect("forged tree");
        fs::write(forged_evidence.join("seed.json"), b"forged-content").expect("forged seed");
        for dir in [&forged_wave, &forged_candidate, &forged_evidence] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).expect("forged mode");
        }

        // Move the real wave aside and swap `W0` for a symlink to the forged
        // tree. The candidate handle still remembers `<state>/W0/<candidate>`,
        // so an operation that re-resolved `W0` by name would follow the
        // symlink into the attacker tree.
        let real_wave = root.path().join("W0");
        let relocated_wave = root.path().join("W0.real");
        fs::rename(&real_wave, &relocated_wave).expect("move the real wave dir aside");
        symlink(&forged_wave, &real_wave).expect("plant the wave symlink");

        // The read goes through the descriptor pinned at candidate-open time, so
        // it observes the legitimate bytes rather than the forged tree behind
        // the swapped ancestor.
        assert_eq!(
            candidate
                .read_bytes(Path::new(EVIDENCE_DIR).join("seed.json"))
                .expect("read seed through the pinned candidate"),
            b"legit-content",
            "a read through the pinned candidate must ignore an ancestor swapped after the pin"
        );

        // A write lands in the legitimate (relocated) candidate inode, never
        // the forged tree, so forged evidence can never be sealed into the real
        // candidate by swapping an ancestor between reads and the write.
        let layer1 = Path::new(EVIDENCE_DIR).join("layer1.json");
        candidate
            .write_bytes(&layer1, b"{}")
            .expect("write lands in the pinned candidate");
        assert!(
            relocated_wave
                .join(cand.as_str())
                .join(EVIDENCE_DIR)
                .join("layer1.json")
                .is_file(),
            "the write must land in the pinned legitimate candidate"
        );
        assert!(
            !forged_candidate
                .join(EVIDENCE_DIR)
                .join("layer1.json")
                .exists(),
            "the write must never reach the forged tree behind the symlinked ancestor"
        );

        // The listing likewise reflects the legitimate tree.
        let names = candidate
            .list(EVIDENCE_DIR)
            .expect("list the pinned evidence dir");
        assert!(
            names.iter().any(|name| name == "seed.json")
                && names.iter().any(|name| name == "layer1.json"),
            "the listing must reflect the pinned legitimate tree, not the forged one"
        );
    }

    #[test]
    fn concurrent_creation_of_an_absent_candidate_directory_both_succeed() {
        use std::sync::{Arc, Barrier};

        let scratch = Scratch::new("concurrent-create");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let id = candidate_id();

        // Pre-seed `W0` with a *different* candidate so the shared wave
        // directory already exists. Only the shared `<state>/W0/<candidate>`
        // leaf then races, so the pre-`mkdirat` hook fires exactly once per
        // racing thread rather than also firing for the wave directory.
        root.candidate("W0", &other_candidate_id())
            .expect("pre-seed W0");

        // A two-party barrier fired *after* both threads observe `ENOENT` and
        // *before* either `mkdirat`s. Neither thread can create the leaf until
        // the other has also seen it absent, so `mkdirat` wins exactly once and
        // the loser must take the `EEXIST` branch - the path this regression
        // must exercise. Without this forced interleave the scheduler could let
        // one thread finish creation before the other's first `openat`, and the
        // `EEXIST` branch would never run.
        let barrier = Arc::new(Barrier::new(2));
        let hook_barrier = Arc::clone(&barrier);
        create_race_hook::install(Some(Arc::new(move || {
            hook_barrier.wait();
        })));

        // Two writers race to create the same initially-absent leaf. `mkdirat`
        // can only win once; the loser must treat `EEXIST` as concurrent
        // creation rather than failing an otherwise valid evidence import.
        let (a, b) = std::thread::scope(|scope| {
            let one = scope.spawn(|| {
                create_race_hook::set_participant(true);
                root.candidate("W0", &id).map(|_| ())
            });
            let two = scope.spawn(|| {
                create_race_hook::set_participant(true);
                root.candidate("W0", &id).map(|_| ())
            });
            (one.join().expect("thread a"), two.join().expect("thread b"))
        });

        let eexist_hits = create_race_hook::eexist_hits();
        create_race_hook::install(None);

        assert!(a.is_ok(), "first concurrent writer failed: {a:?}");
        assert!(b.is_ok(), "second concurrent writer failed: {b:?}");
        assert!(
            eexist_hits >= 1,
            "the forced interleave must exercise the mkdirat EEXIST branch, \
             observed {eexist_hits} EEXIST outcomes"
        );
        assert!(root.path().join("W0").join(id.as_str()).is_dir());
    }

    #[test]
    fn a_prepare_failure_diagnostic_carries_no_absolute_path() {
        // Root-resolution / prepare class: an in-repository state root is
        // refused, and the refusal must not echo the requested path.
        let inside = repo_root().join("packages/target/prepare-leak-check/state");
        let error = StateRoot::prepare(&[repo_root()], Some(&inside))
            .expect_err("an in-repository state root must be refused");
        assert!(error.message().contains("must not live inside"), "{error}");
        assert_no_absolute_path(error.message(), &[&inside, &repo_root()]);
        assert!(!inside.exists(), "a refusal must not create the directory");
    }

    #[test]
    fn a_read_failure_diagnostic_carries_no_absolute_path() {
        let scratch = Scratch::new("read-leak-free");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");

        // A missing artifact: the open failure names the candidate-relative key.
        let missing = Path::new(EVIDENCE_DIR).join("missing.json");
        let error = candidate
            .read_bytes(&missing)
            .expect_err("a missing artifact must fail");
        assert!(
            error.message().contains("evidence/missing.json"),
            "a read failure must name the candidate-relative key: {error}"
        );
        assert_no_absolute_path(error.message(), &[&scratch.path, root.path()]);

        // Malformed JSON: the parse failure names the key, never the path.
        candidate
            .write_bytes(SNAPSHOT_FILE, b"not json")
            .expect("write malformed json");
        let error = candidate
            .read_json::<serde_json::Value>(SNAPSHOT_FILE)
            .expect_err("malformed JSON must fail");
        assert!(
            error.message().contains("snapshot.json"),
            "an invalid-JSON failure must name the key: {error}"
        );
        assert_no_absolute_path(error.message(), &[&scratch.path, root.path()]);
    }

    #[test]
    fn a_list_failure_diagnostic_carries_no_absolute_path() {
        let scratch = Scratch::new("list-leak-free");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let candidate = root.candidate("W0", &candidate_id()).expect("candidate");

        let error = candidate
            .list("nonexistent-dir")
            .expect_err("listing an absent directory must fail");
        assert!(
            error.message().contains("nonexistent-dir"),
            "a list failure must name the candidate-relative key: {error}"
        );
        assert_no_absolute_path(error.message(), &[&scratch.path, root.path()]);
    }

    #[test]
    fn a_root_verification_failure_diagnostic_carries_no_absolute_path() {
        // Root-verification class: a wave directory with a too-permissive mode
        // is refused by the descriptor `fstat` check, and the diagnostic names
        // only the safe component label.
        let scratch = Scratch::new("verify-leak-free");
        let root = StateRoot::for_tests(&scratch.path.join("state")).expect("anchor root");
        let wave = root.path().join("W0");
        fs::create_dir(&wave).expect("create the wave dir");
        fs::set_permissions(&wave, fs::Permissions::from_mode(0o755)).expect("loosen the mode");

        let error = root
            .candidate("W0", &candidate_id())
            .expect_err("a non-0700 wave directory must be refused");
        assert!(
            error.message().contains("mode 0700"),
            "a verification failure must state the required mode: {error}"
        );
        assert_no_absolute_path(error.message(), &[&scratch.path, root.path()]);
    }
}
