//! Daemon-side enforcer for the per-VM state directory ownership matrix
//! declared in
//! `nixos-modules/options-ownership-matrix.nix`.
//!
//! # Invariant: hardlink-farm carve-out
//!
//! `/var/lib/d2b/vms/<vm>/store/` is a per-generation hardlink
//! farm whose inodes are SHARED with `/nix/store`. Recursive
//! ownership / mode / ACL operations across that subtree propagate
//! INTO `/nix/store` via the shared inodes, which breaks the openssh
//! `safe_path()` checks on per-VM ssh host keys (the canonical
//! regression hit on personal-dev - see plan.md §"Ownership matrix
//! for `/var/lib/d2b/vms/<vm>/`" critical-detail note).
//!
//! The enforcer therefore:
//!
//! 1. NEVER recurses into the `store` subdirectory regardless of the
//!    declared `recursive` field. The carve-out is asserted in
//!    [`should_recurse`] and covered by unit tests.
//! 2. Performs only `stat(2)` + comparison; it does NOT mutate
//!    ownership/mode. Mutation belongs to the broker's
//!    host-prepare dispatch surface (audited path).
//! 3. Returns a typed [`OwnershipMismatch`] list per drift so the
//!    caller (d2bd VM-start preflight) can surface a
//!    structured operator message.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Whether a matrix entry is a directory or a regular file. Mirrors the
/// `kind` enum in `nixos-modules/options-ownership-matrix.nix`.
///
/// `File` entries are checked with no-follow `symlink_metadata`, must be
/// a regular file when present, reassert owner/group/mode on the file
/// inode, and are NEVER walked recursively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    #[default]
    Dir,
    File,
}

impl EntryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dir => "dir",
            Self::File => "file",
        }
    }
}

fn default_required() -> bool {
    true
}

/// One row of the per-VM state ownership matrix. Matches the Nix
/// submodule in `nixos-modules/options-ownership-matrix.nix`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipEntry {
    /// Subdirectory under `/var/lib/d2b/vms/<vm>/`. Use `"."` for
    /// the per-VM root itself.
    pub path: String,
    /// Expected uid (already resolved by name on the daemon side).
    pub expected_uid: u32,
    /// Expected gid (already resolved by name on the daemon side).
    pub expected_gid: u32,
    /// Expected mode in the low 12 bits (suid/sgid/sticky + rwx),
    /// matching the value returned by `Metadata::mode() & 0o7777`.
    pub expected_mode: u32,
    /// Whether the entry is a directory or a regular file. File-kind
    /// entries reassert mode/uid/gid on the file inode and never
    /// recurse.
    #[serde(default)]
    pub kind: EntryKind,
    /// Whether the entry must exist by preflight time. When `false`,
    /// the entry is posture-if-present: a not-found (`ENOENT`) stat
    /// result is skipped silently; every other stat error still
    /// surfaces as drift/error.
    #[serde(default = "default_required")]
    pub required: bool,
    /// Whether the daemon may recurse into the directory when
    /// checking. The enforcer additionally rejects recursion into the
    /// `store` / `store-view/live` subdirectories regardless of this
    /// flag (hardlink-farm carve-out), and never recurses into
    /// `file`-kind entries.
    pub recursive: bool,
}

/// Stat snapshot of a path's owner/group/mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ownership {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

/// Structured drift record. Returned per per-entry mismatch so the
/// caller can render a single operator-facing envelope listing every
/// drifted leaf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum OwnershipMismatch {
    /// `stat(2)` (`symlink_metadata`, no-follow) failed on the declared
    /// path. `not_found` distinguishes `ENOENT` (the path is absent -
    /// for `required = false` entries this is never emitted; for
    /// `required = true` entries it is emitted but treated as a
    /// migration-window warning by the preflight) from every other
    /// stat error (`EACCES`, `EIO`, `ELOOP`, …), which is always
    /// fail-closed drift.
    StatFailed {
        path: PathBuf,
        detail: String,
        not_found: bool,
    },
    /// The path exists but its inode type disagrees with the entry
    /// `kind` (a `file` entry resolved to a non-regular-file, or a
    /// `dir` entry resolved to a non-directory). No-follow: a symlink
    /// is reported as a kind mismatch rather than being traversed.
    KindMismatch {
        path: PathBuf,
        expected_kind: String,
        actual_kind: String,
    },
    /// The path exists but owner/group/mode differ from the matrix.
    Drift {
        path: PathBuf,
        expected: Ownership,
        actual: Ownership,
        drift_reason: DriftReason,
    },
    /// Recursive walk found a child whose owner/group/mode differs.
    /// Children of the `store` / `store-view/live` subdirectories are
    /// NEVER reported here: the enforcer refuses to recurse into the
    /// hardlink pool to avoid even READING ownership on inodes shared
    /// with /nix/store in a way that could be misinterpreted as a
    /// fix-up signal.
    ChildDrift {
        path: PathBuf,
        expected_uid: u32,
        expected_gid: u32,
        expected_mode: u32,
        actual: Ownership,
    },
}

/// Bit-field describing which axes of an [`OwnershipMismatch::Drift`]
/// disagree with the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DriftReason {
    pub owner: bool,
    pub group: bool,
    pub mode: bool,
}

impl OwnershipMismatch {
    /// Stable identifier for envelope rendering.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::StatFailed { .. } => "ownership-matrix-stat-failed",
            Self::KindMismatch { .. } => "ownership-matrix-kind-mismatch",
            Self::Drift { .. } => "ownership-matrix-drift",
            Self::ChildDrift { .. } => "ownership-matrix-child-drift",
        }
    }

    /// Path the mismatch refers to (for operator-visible messaging).
    pub fn path(&self) -> &Path {
        match self {
            Self::StatFailed { path, .. }
            | Self::KindMismatch { path, .. }
            | Self::Drift { path, .. }
            | Self::ChildDrift { path, .. } => path.as_path(),
        }
    }
}

/// Per-VM hardlink-pool paths the enforcer NEVER recurses into.
///
/// Each string is compared byte-for-byte against `entry.path`. Covers
/// the canonical `store-view/live` pool and the legacy `store` farm;
/// both share inodes with /nix/store, so recursing would risk
/// propagating ownership/ACL changes into the system store.
const HARDLINK_FARM_CARVE_OUTS: &[&str] = &["store", "store-view/live"];

/// Return whether the enforcer is permitted to recurse into this
/// entry. Combines the operator-declared `recursive` flag with the
/// hardlink-farm carve-out: even if a future operator typo flips
/// `recursive = true` on the `store` / `store-view/live` entry, this
/// function still returns `false`. A `file`-kind entry is a single
/// inode and is never walked.
pub fn should_recurse(entry: &OwnershipEntry) -> bool {
    if entry.kind == EntryKind::File {
        return false;
    }
    if HARDLINK_FARM_CARVE_OUTS.contains(&entry.path.as_str()) {
        return false;
    }
    entry.recursive
}

/// Check the per-VM state directory at `base` against the declared
/// `matrix`. Returns the empty `Vec` when every entry matches.
///
/// `_vm` is currently informational (it's already baked into `base`
/// by the caller). It's retained in the signature so future audit
/// records can carry the VM name without a downstream refactor.
// Deliberately synchronous: this is the crate's public ownership
// preflight surface consumed synchronously by d2bd-runtime's
// `ownership_preflight` before broker dispatch. The per-entry stats
// and bounded walk have no async caller in this crate's contract.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn check_ownership_matrix(
    _vm: &str,
    base: &Path,
    matrix: &[OwnershipEntry],
) -> Vec<OwnershipMismatch> {
    let mut drifts = Vec::new();

    for entry in matrix {
        let target = if entry.path == "." {
            base.to_path_buf()
        } else {
            base.join(&entry.path)
        };

        // No-follow stat: never traverse a symlink at the leaf.
        let meta = match fs::symlink_metadata(&target) {
            Ok(m) => m,
            Err(err) => {
                if err.kind() == ErrorKind::NotFound {
                    // ENOENT. Optional entries skip silently; required
                    // entries surface a `not_found` StatFailed that the
                    // preflight policy downgrades during the migration
                    // window.
                    if entry.required {
                        drifts.push(OwnershipMismatch::StatFailed {
                            path: target,
                            detail: err.to_string(),
                            not_found: true,
                        });
                    }
                } else {
                    // EACCES / EIO / ELOOP / … - always fail-closed,
                    // independent of `required`.
                    drifts.push(OwnershipMismatch::StatFailed {
                        path: target,
                        detail: err.to_string(),
                        not_found: false,
                    });
                }
                continue;
            }
        };

        // Kind check (no-follow): a `file` entry must resolve to a
        // regular file; a `dir` entry must resolve to a directory. A
        // symlink (or any other type) is a kind mismatch, never
        // traversed.
        let ft = meta.file_type();
        let kind_ok = match entry.kind {
            EntryKind::File => ft.is_file(),
            EntryKind::Dir => ft.is_dir(),
        };
        if !kind_ok {
            drifts.push(OwnershipMismatch::KindMismatch {
                path: target,
                expected_kind: entry.kind.as_str().to_owned(),
                actual_kind: actual_kind_str(&meta).to_owned(),
            });
            continue;
        }

        // Owner/group/mode reassertion, for both file- and dir-kind
        // entries, on the stat'd inode.
        let actual = Ownership {
            uid: meta.uid(),
            gid: meta.gid(),
            mode: meta.mode() & 0o7777,
        };
        let expected = Ownership {
            uid: entry.expected_uid,
            gid: entry.expected_gid,
            mode: entry.expected_mode,
        };

        if actual != expected {
            let drift_reason = DriftReason {
                owner: actual.uid != expected.uid,
                group: actual.gid != expected.gid,
                mode: actual.mode != expected.mode,
            };
            drifts.push(OwnershipMismatch::Drift {
                path: target.clone(),
                expected,
                actual,
                drift_reason,
            });
        }

        if should_recurse(entry) {
            walk_children(&target, &expected, &mut drifts);
        }
    }

    drifts
}

/// Human-readable inode type for a [`OwnershipMismatch::KindMismatch`].
fn actual_kind_str(meta: &fs::Metadata) -> &'static str {
    let ft = meta.file_type();
    if ft.is_dir() {
        "dir"
    } else if ft.is_file() {
        "file"
    } else if ft.is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

/// Bounded shallow walk used only for `recursive = true` entries that
/// pass the hardlink-farm carve-out. Does NOT follow symlinks; does
/// NOT cross filesystem boundaries (the per-VM tree is required to
/// live on a single FS by the `hardlink_farm::assert_same_filesystem`
/// invariant).
// Sync companion of `check_ownership_matrix` (same public-surface
// boundary): bounded shallow walk of recursively-managed entries.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn walk_children(root: &Path, expected: &Ownership, out: &mut Vec<OwnershipMismatch>) {
    let read = match fs::read_dir(root) {
        Ok(it) => it,
        Err(err) => {
            out.push(OwnershipMismatch::StatFailed {
                path: root.to_path_buf(),
                detail: format!("read_dir failed: {err}"),
                not_found: err.kind() == ErrorKind::NotFound,
            });
            return;
        }
    };
    let root_dev = fs::symlink_metadata(root).ok().map(|m| m.dev());
    for entry in read.flatten() {
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(err) => {
                let not_found = err.kind() == ErrorKind::NotFound;
                out.push(OwnershipMismatch::StatFailed {
                    path,
                    detail: err.to_string(),
                    not_found,
                });
                continue;
            }
        };
        if let Some(dev) = root_dev
            && meta.dev() != dev
        {
            continue;
        }
        let actual = Ownership {
            uid: meta.uid(),
            gid: meta.gid(),
            mode: meta.mode() & 0o7777,
        };
        if actual.uid != expected.uid || actual.gid != expected.gid || actual.mode != expected.mode
        {
            out.push(OwnershipMismatch::ChildDrift {
                path: path.clone(),
                expected_uid: expected.uid,
                expected_gid: expected.gid,
                expected_mode: expected.mode,
                actual,
            });
        }
        if meta.is_dir() {
            walk_children(&path, expected, out);
        }
    }
}

// ---------------------------------------------------------------------------
// The destructive reset ownership boundary (U32, KTD15)
// ---------------------------------------------------------------------------
//
// A clean-break reset removes host state. Two questions have to have the
// same answer before a single name is unlinked, and neither of them is
// "the directory looked empty":
//
// 1. Is this byte d2b's to remove? Answered from the exact ownership
//    description the admitted reset Operation carries, checked against a
//    no-follow observation of the deployment root. A foreign ownership
//    marker, a symlink on any component, an owned path that resolves
//    outside the deployment root, an owned path on another filesystem, or
//    an external Volume source the owned set would swallow each fail
//    closed - a marker is never an authorization to overwrite.
// 2. Is anything still using it? Answered from live evidence: cgroup
//    members, managed processes, ownership markers, held leases and the
//    active host generation. Fresh empty state is exactly what the new
//    model wants to look like, so "empty" is never the drain proof.
//
// Nothing here mutates an inode. The reset unlinks names; it never
// chmods or chowns, because a store-view farm shares its inodes with the
// system store and a permission change on one name is a permission change
// on every name.

/// The mandatory ownership marker a d2b-managed host surface carries.
/// The `# d2b managed: ` comment every d2b-managed host surface carries,
/// exactly as the nftables and NetworkManager markers do.
pub const OWNERSHIP_MARKER_PREFIX: &str = "# d2b managed: ";

/// The delimiters that bracket a d2b-managed block.
pub const OWNERSHIP_BLOCK_BEGIN: &str = "# d2b-managed begin";
pub const OWNERSHIP_BLOCK_END: &str = "# d2b-managed end";

/// The file at a deployment root that names its ownership id.
pub const OWNERSHIP_MARKER_FILE: &str = "d2b-ownership";

/// The bounded read for a deployment root ownership marker.
///
/// A marker is one delimited block carrying one ownership id. A larger
/// file is not a marker this release reads, so it is refused rather than
/// scanned for something that happens to look like an id.
pub const MAX_OWNERSHIP_MARKER_BYTES: usize = 4096;

/// How one owned path may be treated by the destructive reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OwnedPathKind {
    /// A directory whose entries are unlinked. No permission is changed
    /// on anything inside it.
    Tree,
    /// A single regular file.
    File,
    /// A per-Guest store-view hardlink farm. Only the farm's own direct
    /// basenames are unlinked: the farm is never descended into, so a
    /// `gcroots/...` link into the system store is never followed and no
    /// shared inode is ever stat'd for repair.
    HardlinkFarm,
}

impl OwnedPathKind {
    /// The inode kind the ownership description declares for this path.
    pub const fn expected_inode_kind(self) -> &'static str {
        match self {
            Self::Tree | Self::HardlinkFarm => "dir",
            Self::File => "file",
        }
    }
}

/// One exact owned path in a reset's ownership description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedPath {
    /// Absolute path, lexically beneath the deployment root.
    pub path: PathBuf,
    /// What the reset may do with it.
    pub kind: OwnedPathKind,
}

impl OwnedPath {
    /// An owned directory tree.
    pub fn tree(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: OwnedPathKind::Tree,
        }
    }

    /// An owned regular file.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: OwnedPathKind::File,
        }
    }

    /// An owned store-view hardlink farm.
    pub fn hardlink_farm(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: OwnedPathKind::HardlinkFarm,
        }
    }
}

/// The exact ownership description one admitted reset Operation carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetOwnership {
    /// Every path the reset may unlink, with the treatment it may use.
    pub owned: Vec<OwnedPath>,
    /// Volume sources the operator owns that live outside the deployment
    /// root. They are named so a reset can prove it is leaving them
    /// alone; one that would reach into one is refused, not narrowed.
    pub external_sources: Vec<PathBuf>,
}

impl ResetOwnership {
    /// The exact owned set, in declaration order.
    pub fn owned(&self) -> &[OwnedPath] {
        &self.owned
    }
}

/// The inode kind one no-follow stat observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeKind {
    Dir,
    File,
    Symlink,
    Other,
}

impl InodeKind {
    /// The stable label a refusal renders.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dir => "dir",
            Self::File => "file",
            Self::Symlink => "symlink",
            Self::Other => "other",
        }
    }
}

/// One owned path as the filesystem presents it, no-follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedObservation {
    /// The owned path.
    pub path: PathBuf,
    /// The treatment the ownership description declared.
    pub kind: OwnedPathKind,
    /// Whether the path exists at all. An absent owned path is a
    /// completed reset, not a failure, so a repeated reset is safe.
    pub present: bool,
    /// The observed inode kind when present.
    pub inode_kind: Option<InodeKind>,
    /// The `st_dev` the path lives on, when present. An owned path on a
    /// different device from the deployment root is a mounted foreign
    /// filesystem: unlinking into it would delete data this ownership
    /// description never described.
    pub device: Option<u64>,
}

/// Everything the pure reset decision reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetObservation {
    /// The deployment root the reset is bounded to.
    pub deployment_root: PathBuf,
    /// The `st_dev` of the deployment root itself.
    pub deployment_device: u64,
    /// The ownership id the deployment root's marker must carry.
    pub ownership_id: String,
    /// The marker body found at the deployment root, when it was read.
    pub ownership_marker: Option<String>,
    /// The no-follow observation of every owned path.
    pub owned: Vec<OwnedObservation>,
}

/// The live evidence a reset must observe before it unlinks anything.
///
/// Empty new state is the shape the new model wants, so no field here is
/// derived from the absence of records: each is a positive observation of
/// something still using the state the reset is about to remove.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainEvidence {
    /// Live members under the managed cgroup subtree.
    pub live_cgroup_members: Vec<PathBuf>,
    /// Live managed processes.
    pub live_processes: Vec<PathBuf>,
    /// Managed ownership or readiness markers still on disk.
    pub live_markers: Vec<PathBuf>,
    /// Lease or lock files still held.
    pub held_leases: Vec<PathBuf>,
    /// The host generation still recorded as active, if any.
    pub active_host_generation: Option<String>,
}

/// Why a destructive reset refuses to act.
///
/// Every variant is a fail-closed decision: an unresolved question is a
/// refusal, never a permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "reason")]
pub enum ResetRefusal {
    /// The deployment root is not a directory.
    DeploymentRootKindMismatch {
        path: PathBuf,
        observed: String,
    },
    /// An owned path is not strictly beneath the deployment root.
    OwnershipOutsideDeploymentRoot { path: PathBuf },
    /// A symlink, or a non-directory, stands on a component of an owned
    /// path.
    SymlinkEscape { path: PathBuf },
    /// An owned path exists but is not the kind its declaration named.
    OwnershipKindMismatch {
        path: PathBuf,
        expected: String,
        observed: String,
    },
    /// An owned path could not be stat-ed at all.
    OwnershipStatFailed { path: PathBuf, detail: String },
    /// The deployment root's ownership marker is missing, oversized, or
    /// carries no managed ownership id, so ownership cannot be
    /// established at all.
    OwnershipMarkerUnreadable { path: PathBuf, detail: String },
    /// The deployment root's ownership marker names a different owner. A
    /// foreign marker is never an authorization to overwrite.
    ForeignOwnershipMarker { path: PathBuf, marker: String },
    /// An owned path is on another filesystem than the deployment root,
    /// so unlinking into it would delete a mounted foreign filesystem.
    ForeignFilesystem { path: PathBuf },
    /// An external Volume source falls inside the owned set, so the reset
    /// would delete operator data this ownership description never
    /// described.
    ExternalSourceInsideOwnership { path: PathBuf },
    /// A managed workload is still live.
    WorkloadLive { path: PathBuf },
    /// A lease or lock is still held.
    LeaseHeld { path: PathBuf },
    /// A host generation is still recorded as active.
    HostGenerationActive { generation: String },
}

impl ResetRefusal {
    /// The stable identifier an envelope renders.
    pub fn code(&self) -> &'static str {
        match self {
            Self::DeploymentRootKindMismatch { .. } => "reset-deployment-root-kind-mismatch",
            Self::OwnershipOutsideDeploymentRoot { .. } => "reset-ownership-outside-root",
            Self::SymlinkEscape { .. } => "reset-symlink-escape",
            Self::OwnershipKindMismatch { .. } => "reset-ownership-kind-mismatch",
            Self::OwnershipStatFailed { .. } => "reset-ownership-stat-failed",
            Self::OwnershipMarkerUnreadable { .. } => "reset-ownership-marker-unreadable",
            Self::ForeignOwnershipMarker { .. } => "reset-foreign-ownership-marker",
            Self::ForeignFilesystem { .. } => "reset-foreign-filesystem",
            Self::ExternalSourceInsideOwnership { .. } => "reset-external-source-owned",
            Self::WorkloadLive { .. } => "reset-workload-live",
            Self::LeaseHeld { .. } => "reset-lease-held",
            Self::HostGenerationActive { .. } => "reset-host-generation-active",
        }
    }

    /// The path the refusal is about, when it names one.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::DeploymentRootKindMismatch { path, .. }
            | Self::OwnershipOutsideDeploymentRoot { path }
            | Self::SymlinkEscape { path }
            | Self::OwnershipKindMismatch { path, .. }
            | Self::OwnershipStatFailed { path, .. }
            | Self::OwnershipMarkerUnreadable { path, .. }
            | Self::ForeignOwnershipMarker { path, .. }
            | Self::ForeignFilesystem { path }
            | Self::ExternalSourceInsideOwnership { path }
            | Self::WorkloadLive { path }
            | Self::LeaseHeld { path } => Some(path.as_path()),
            Self::HostGenerationActive { .. } => None,
        }
    }
}

impl core::fmt::Display for ResetRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ResetRefusal {}

/// One owned path the ownership boundary verified, ready to unlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedOwnedPath {
    /// The owned path.
    pub path: PathBuf,
    /// The treatment it may receive.
    pub kind: OwnedPathKind,
    /// Whether it is currently present. An absent entry is reported so a
    /// repeated completed reset is visible as a no-op rather than a
    /// silent success.
    pub present: bool,
}

/// The verified exact ownership set: the complete unlink inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedResetOwnership {
    /// The deployment root the reset is bounded to.
    pub deployment_root: PathBuf,
    /// The ownership id the deployment root's marker carried.
    pub ownership_id: String,
    /// Every verified entry, in ownership-description order.
    pub entries: Vec<VerifiedOwnedPath>,
    /// External Volume sources the reset proved it leaves alone.
    pub external_sources: Vec<PathBuf>,
}

impl VerifiedResetOwnership {
    /// The entries that currently exist and would be unlinked.
    pub fn present_entries(&self) -> impl Iterator<Item = &VerifiedOwnedPath> {
        self.entries.iter().filter(|entry| entry.present)
    }
}

/// Read the ownership id out of a managed marker body.
///
/// The body is the `# d2b-managed begin` / `# d2b-managed end` delimited
/// block whose line is the mandatory `# d2b managed: <ownership-id>`
/// comment. Anything else - a foreign comment, an unterminated block, a
/// block with no id - is `None`, and the caller resolves that as a refusal
/// rather than as an empty id that would match anything.
pub fn parse_ownership_marker(body: &str) -> Option<&str> {
    let begin = body.find(OWNERSHIP_BLOCK_BEGIN)?;
    let rest = &body[begin + OWNERSHIP_BLOCK_BEGIN.len()..];
    let end = rest.find(OWNERSHIP_BLOCK_END)?;
    let block = &rest[..end];
    for line in block.lines() {
        if let Some(id) = line.trim().strip_prefix(OWNERSHIP_MARKER_PREFIX) {
            let id = id.trim();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

/// Render the managed ownership marker body for one id.
pub fn render_ownership_marker(ownership_id: &str) -> String {
    format!(
        "{OWNERSHIP_BLOCK_BEGIN}\n{OWNERSHIP_MARKER_PREFIX}{ownership_id}\n{OWNERSHIP_BLOCK_END}\n"
    )
}

/// Observe one owned path without traversing a single symlink.
///
/// Every component beneath the deployment root is `symlink_metadata`-ed in
/// turn: a symlink anywhere on the path is
/// [`ResetRefusal::SymlinkEscape`] rather than something to resolve, so an
/// owned name can never be redirected outside the root by planting a link
/// under it. A final component that is simply absent is a completed reset
/// and is reported as absent; an absent intermediate component is a broken
/// ownership description and is refused.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn observe_owned(
    deployment_root: &Path,
    owned: &OwnedPath,
    deployment_device: u64,
) -> Result<OwnedObservation, ResetRefusal> {
    let relative = owned
        .path
        .strip_prefix(deployment_root)
        .map_err(|_| ResetRefusal::OwnershipOutsideDeploymentRoot {
            path: owned.path.clone(),
        })?;
    let components: Vec<_> = relative.components().collect();
    if components.is_empty() {
        return Err(ResetRefusal::OwnershipOutsideDeploymentRoot {
            path: owned.path.clone(),
        });
    }
    let mut walked = deployment_root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let name = component.as_os_str();
        if name == ".." || name == "." || name == "/" {
            return Err(ResetRefusal::OwnershipOutsideDeploymentRoot {
                path: owned.path.clone(),
            });
        }
        walked.push(name);
        let meta = match fs::symlink_metadata(&walked) {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound && index + 1 == components.len() => {
                return Ok(OwnedObservation {
                    path: owned.path.clone(),
                    kind: owned.kind,
                    present: false,
                    inode_kind: None,
                    device: None,
                });
            }
            Err(error) => {
                return Err(ResetRefusal::OwnershipStatFailed {
                    path: walked,
                    detail: error.to_string(),
                });
            }
        };
        if meta.file_type().is_symlink() {
            return Err(ResetRefusal::SymlinkEscape { path: walked });
        }
        let is_last = index + 1 == components.len();
        if !is_last && !meta.is_dir() {
            return Err(ResetRefusal::SymlinkEscape { path: walked });
        }
        if is_last {
            let inode_kind = if meta.is_dir() {
                InodeKind::Dir
            } else if meta.is_file() {
                InodeKind::File
            } else {
                InodeKind::Other
            };
            let device = meta.dev();
            if device != deployment_device {
                return Err(ResetRefusal::ForeignFilesystem { path: walked });
            }
            return Ok(OwnedObservation {
                path: owned.path.clone(),
                kind: owned.kind,
                present: true,
                inode_kind: Some(inode_kind),
                device: Some(device),
            });
        }
    }
    Err(ResetRefusal::OwnershipStatFailed {
        path: owned.path.clone(),
        detail: "the owned path declared no final component".to_owned(),
    })
}

/// Observe the deployment root and its exact owned set, without judging.
///
/// The returned observation is the input to the pure [`verify_reset`]
/// decision, so every filesystem read lives here and every refusal
/// predicate stays decidable without a mount table.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn observe_reset(
    deployment_root: &Path,
    ownership_id: &str,
    ownership: &ResetOwnership,
) -> Result<ResetObservation, ResetRefusal> {
    let root_meta = match fs::symlink_metadata(deployment_root) {
        Ok(meta) => meta,
        Err(error) => {
            return Err(ResetRefusal::OwnershipStatFailed {
                path: deployment_root.to_path_buf(),
                detail: error.to_string(),
            });
        }
    };
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Err(ResetRefusal::DeploymentRootKindMismatch {
            path: deployment_root.to_path_buf(),
            observed: actual_kind_str(&root_meta).to_owned(),
        });
    }
    let deployment_device = root_meta.dev();
    let marker_path = deployment_root.join(OWNERSHIP_MARKER_FILE);
    let ownership_marker = match fs::read(&marker_path) {
        Ok(bytes) => {
            if bytes.len() > MAX_OWNERSHIP_MARKER_BYTES {
                return Err(ResetRefusal::OwnershipMarkerUnreadable {
                    path: marker_path,
                    detail: "the ownership marker exceeds the bounded read".to_owned(),
                });
            }
            String::from_utf8(bytes).ok()
        }
        Err(error) => {
            return Err(ResetRefusal::OwnershipMarkerUnreadable {
                path: marker_path,
                detail: error.to_string(),
            });
        }
    };
    let mut owned = Vec::with_capacity(ownership.owned.len());
    for entry in &ownership.owned {
        owned.push(observe_owned(deployment_root, entry, deployment_device)?);
    }
    Ok(ResetObservation {
        deployment_root: deployment_root.to_path_buf(),
        deployment_device,
        ownership_id: ownership_id.to_owned(),
        ownership_marker,
        owned,
    })
}

/// Decide whether the observed ownership may be removed.
///
/// Pure: no clock, no store, no I/O. It resolves the ownership marker,
/// each owned path's declared kind, each owned path's filesystem, every
/// external Volume source, and the live drain evidence.
pub fn verify_reset(
    observation: &ResetObservation,
    ownership: &ResetOwnership,
    evidence: &DrainEvidence,
) -> Result<VerifiedResetOwnership, ResetRefusal> {
    let marker_path = observation.deployment_root.join(OWNERSHIP_MARKER_FILE);
    let marker = observation.ownership_marker.as_deref().ok_or_else(|| {
        ResetRefusal::OwnershipMarkerUnreadable {
            path: marker_path.clone(),
            detail: "the ownership marker is not valid UTF-8".to_owned(),
        }
    })?;
    match parse_ownership_marker(marker) {
        None => {
            return Err(ResetRefusal::OwnershipMarkerUnreadable {
                path: marker_path,
                detail: "the ownership marker carries no managed ownership id".to_owned(),
            });
        }
        Some(id) if id != observation.ownership_id => {
            return Err(ResetRefusal::ForeignOwnershipMarker {
                path: marker_path,
                marker: id.to_owned(),
            });
        }
        Some(_) => {}
    }

    for source in &ownership.external_sources {
        if source.starts_with(&observation.deployment_root) {
            return Err(ResetRefusal::ExternalSourceInsideOwnership {
                path: source.clone(),
            });
        }
    }

    let mut entries = Vec::with_capacity(observation.owned.len());
    for observed in &observation.owned {
        if !observed.present {
            entries.push(VerifiedOwnedPath {
                path: observed.path.clone(),
                kind: observed.kind,
                present: false,
            });
            continue;
        }
        let observed_kind = observed.inode_kind.unwrap_or(InodeKind::Other);
        let expected = observed.kind.expected_inode_kind();
        if observed_kind.as_str() != expected {
            return Err(ResetRefusal::OwnershipKindMismatch {
                path: observed.path.clone(),
                expected: expected.to_owned(),
                observed: observed_kind.as_str().to_owned(),
            });
        }
        if observed.device != Some(observation.deployment_device) {
            return Err(ResetRefusal::ForeignFilesystem {
                path: observed.path.clone(),
            });
        }
        entries.push(VerifiedOwnedPath {
            path: observed.path.clone(),
            kind: observed.kind,
            present: true,
        });
    }

    if let Some(path) = evidence.live_cgroup_members.first() {
        return Err(ResetRefusal::WorkloadLive { path: path.clone() });
    }
    if let Some(path) = evidence.live_processes.first() {
        return Err(ResetRefusal::WorkloadLive { path: path.clone() });
    }
    if let Some(path) = evidence.live_markers.first() {
        return Err(ResetRefusal::WorkloadLive { path: path.clone() });
    }
    if let Some(path) = evidence.held_leases.first() {
        return Err(ResetRefusal::LeaseHeld { path: path.clone() });
    }
    if let Some(generation) = evidence.active_host_generation.as_ref() {
        return Err(ResetRefusal::HostGenerationActive {
            generation: generation.clone(),
        });
    }

    Ok(VerifiedResetOwnership {
        deployment_root: observation.deployment_root.clone(),
        ownership_id: observation.ownership_id.clone(),
        entries,
        external_sources: ownership.external_sources.clone(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;
    use std::os::unix::fs::PermissionsExt;

    fn current_uid() -> u32 {
        // Tests run as the invoking user; we compare against the
        // process owner so the happy-path entries match without
        // needing root.
        nix::unistd::Uid::current().as_raw()
    }
    fn current_gid() -> u32 {
        nix::unistd::Gid::current().as_raw()
    }

    fn mk_entry(path: &str, mode: u32) -> OwnershipEntry {
        OwnershipEntry {
            path: path.to_owned(),
            expected_uid: current_uid(),
            expected_gid: current_gid(),
            expected_mode: mode,
            kind: EntryKind::Dir,
            required: true,
            recursive: false,
        }
    }

    fn mk_file_entry(path: &str, mode: u32, required: bool) -> OwnershipEntry {
        OwnershipEntry {
            path: path.to_owned(),
            expected_uid: current_uid(),
            expected_gid: current_gid(),
            expected_mode: mode,
            kind: EntryKind::File,
            required,
            recursive: false,
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn prepare(base: &Path, sub: &str, mode: u32) -> PathBuf {
        let p = if sub == "." {
            base.to_path_buf()
        } else {
            base.join(sub)
        };
        if sub != "." {
            stdfs::create_dir_all(&p).unwrap();
        }
        stdfs::set_permissions(&p, stdfs::Permissions::from_mode(mode)).unwrap();
        p
    }

    #[test]
    fn happy_path_no_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        prepare(base, ".", 0o2770);
        prepare(base, "state", 0o0750);

        let matrix = vec![mk_entry(".", 0o2770), mk_entry("state", 0o0750)];
        let drifts = check_ownership_matrix("vm1", base, &matrix);
        assert!(drifts.is_empty(), "unexpected drift: {drifts:?}");
    }

    #[test]
    fn mode_drift_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        prepare(base, ".", 0o0755);

        let matrix = vec![mk_entry(".", 0o2770)];
        let drifts = check_ownership_matrix("vm1", base, &matrix);
        assert_eq!(drifts.len(), 1);
        match &drifts[0] {
            OwnershipMismatch::Drift { drift_reason, .. } => {
                assert!(drift_reason.mode);
                assert!(!drift_reason.owner);
                assert!(!drift_reason.group);
            }
            other => panic!("expected Drift, got {other:?}"),
        }
    }

    #[test]
    fn owner_drift_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        prepare(base, ".", 0o0750);

        let mut entry = mk_entry(".", 0o0750);
        // Pick a uid the test process is guaranteed not to be (root
        // unless tests are run as root, in which case use nobody=65534).
        entry.expected_uid = if current_uid() == 0 { 65534 } else { 0 };
        let drifts = check_ownership_matrix("vm1", base, &[entry]);
        assert_eq!(drifts.len(), 1);
        match &drifts[0] {
            OwnershipMismatch::Drift { drift_reason, .. } => {
                assert!(drift_reason.owner);
                assert!(!drift_reason.mode);
            }
            other => panic!("expected Drift, got {other:?}"),
        }
    }

    #[test]
    fn group_drift_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        prepare(base, ".", 0o0750);

        let mut entry = mk_entry(".", 0o0750);
        entry.expected_gid = if current_gid() == 0 { 65534 } else { 0 };
        let drifts = check_ownership_matrix("vm1", base, &[entry]);
        assert_eq!(drifts.len(), 1);
        match &drifts[0] {
            OwnershipMismatch::Drift { drift_reason, .. } => {
                assert!(drift_reason.group);
                assert!(!drift_reason.mode);
            }
            other => panic!("expected Drift, got {other:?}"),
        }
    }

    /// CRITICAL regression for the hardlink-farm carve-out.
    ///
    /// Even if the operator declares `recursive = true` on the
    /// `store`/`store-view/live` entry (a typo, or a misguided
    /// migration), the enforcer
    /// MUST NOT recurse. We assert this two ways:
    ///
    /// 1. [`should_recurse`] returns false for hardlink-farm paths
    ///    regardless of the `recursive` flag.
    /// 2. [`check_ownership_matrix`] does not emit any
    ///    `ChildDrift` for files under `store/`, even when those
    ///    files have intentionally bad ownership.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn hardlink_farm_carve_out_holds_for_legacy_store() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        prepare(base, ".", 0o2770);
        let store = prepare(base, "store", 0o2775);
        // A child that WOULD trip ChildDrift if the enforcer
        // recursed: a regular file with mode 0600 (does not match
        // the entry's 0o2775 directory expectations).
        let child = store.join("hardlinked-file");
        stdfs::write(&child, b"x").unwrap();
        stdfs::set_permissions(&child, stdfs::Permissions::from_mode(0o0600)).unwrap();

        let entry = OwnershipEntry {
            path: "store".to_owned(),
            expected_uid: current_uid(),
            expected_gid: current_gid(),
            expected_mode: 0o2775,
            kind: EntryKind::Dir,
            required: false,
            // Hostile override: operator (or test) declared recursive.
            // The carve-out MUST still hold.
            recursive: true,
        };

        assert!(
            !should_recurse(&entry),
            "carve-out must override `recursive = true`"
        );

        let drifts = check_ownership_matrix("vm1", base, &[entry]);
        // Top-level entry matches; no ChildDrift may appear under
        // store/.
        for d in &drifts {
            if let OwnershipMismatch::ChildDrift { path, .. } = d {
                panic!("enforcer recursed into hardlink farm: {path:?}");
            }
        }
        assert!(
            drifts.is_empty(),
            "top-level matches; got unexpected drift(s): {drifts:?}",
        );
    }

    #[test]
    fn hardlink_farm_carve_out_holds_for_store_view_live() {
        let entry = OwnershipEntry {
            path: "store-view/live".to_owned(),
            expected_uid: current_uid(),
            expected_gid: current_gid(),
            expected_mode: 0o0755,
            kind: EntryKind::Dir,
            required: true,
            recursive: true,
        };

        assert!(
            !should_recurse(&entry),
            "store-view/live carve-out must override `recursive = true`"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn recursive_walk_reports_child_drift_outside_carve_out() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let sub = prepare(base, "state", 0o0750);
        let child = sub.join("bad-child");
        stdfs::write(&child, b"x").unwrap();
        stdfs::set_permissions(&child, stdfs::Permissions::from_mode(0o0777)).unwrap();

        let entry = OwnershipEntry {
            path: "state".to_owned(),
            expected_uid: current_uid(),
            expected_gid: current_gid(),
            expected_mode: 0o0750,
            kind: EntryKind::Dir,
            required: true,
            recursive: true,
        };
        let drifts = check_ownership_matrix("vm1", base, &[entry]);
        assert!(
            drifts
                .iter()
                .any(|d| matches!(d, OwnershipMismatch::ChildDrift { .. })),
            "expected ChildDrift for non-carve-out recursive walk: {drifts:?}",
        );
    }

    #[test]
    fn kind_strings_are_stable() {
        let m = OwnershipMismatch::StatFailed {
            path: PathBuf::from("/x"),
            detail: "no".to_owned(),
            not_found: true,
        };
        assert_eq!(m.kind(), "ownership-matrix-stat-failed");
        let m = OwnershipMismatch::KindMismatch {
            path: PathBuf::from("/x"),
            expected_kind: "file".to_owned(),
            actual_kind: "dir".to_owned(),
        };
        assert_eq!(m.kind(), "ownership-matrix-kind-mismatch");
        let m = OwnershipMismatch::Drift {
            path: PathBuf::from("/x"),
            expected: Ownership {
                uid: 0,
                gid: 0,
                mode: 0,
            },
            actual: Ownership {
                uid: 1,
                gid: 0,
                mode: 0,
            },
            drift_reason: DriftReason {
                owner: true,
                group: false,
                mode: false,
            },
        };
        assert_eq!(m.kind(), "ownership-matrix-drift");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn file_kind_happy_path_no_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let f = base.join("sync.lock");
        stdfs::write(&f, b"").unwrap();
        stdfs::set_permissions(&f, stdfs::Permissions::from_mode(0o0600)).unwrap();

        let drifts =
            check_ownership_matrix("vm1", base, &[mk_file_entry("sync.lock", 0o0600, true)]);
        assert!(drifts.is_empty(), "unexpected drift: {drifts:?}");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn file_kind_reasserts_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let f = base.join("sync.lock");
        stdfs::write(&f, b"").unwrap();
        // 0644 on disk but the entry expects 0600: file-kind must
        // still reassert mode on the file inode.
        stdfs::set_permissions(&f, stdfs::Permissions::from_mode(0o0644)).unwrap();

        let drifts =
            check_ownership_matrix("vm1", base, &[mk_file_entry("sync.lock", 0o0600, true)]);
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        match &drifts[0] {
            OwnershipMismatch::Drift { drift_reason, .. } => {
                assert!(drift_reason.mode);
            }
            other => panic!("expected Drift, got {other:?}"),
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn file_kind_on_directory_is_kind_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        // The path exists but is a directory while the entry is
        // file-kind.
        stdfs::create_dir(base.join("sync.lock")).unwrap();

        let drifts =
            check_ownership_matrix("vm1", base, &[mk_file_entry("sync.lock", 0o0600, true)]);
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        match &drifts[0] {
            OwnershipMismatch::KindMismatch {
                expected_kind,
                actual_kind,
                ..
            } => {
                assert_eq!(expected_kind, "file");
                assert_eq!(actual_kind, "dir");
            }
            other => panic!("expected KindMismatch, got {other:?}"),
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn dir_kind_on_file_is_kind_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        stdfs::write(base.join("state"), b"x").unwrap();

        let drifts = check_ownership_matrix("vm1", base, &[mk_entry("state", 0o0750)]);
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        match &drifts[0] {
            OwnershipMismatch::KindMismatch {
                expected_kind,
                actual_kind,
                ..
            } => {
                assert_eq!(expected_kind, "dir");
                assert_eq!(actual_kind, "file");
            }
            other => panic!("expected KindMismatch, got {other:?}"),
        }
    }

    #[test]
    fn optional_missing_entry_is_silently_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        prepare(base, ".", 0o2770);

        // Optional file-kind entry whose path is absent: NO mismatch
        // is emitted at all (optional skips only ENOENT).
        let drifts = check_ownership_matrix(
            "vm1",
            base,
            &[mk_file_entry("does-not-exist", 0o0640, false)],
        );
        assert!(
            drifts.is_empty(),
            "optional-missing must be silent: {drifts:?}"
        );
    }

    #[test]
    fn required_missing_entry_reports_not_found_stat_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        prepare(base, ".", 0o2770);

        let drifts =
            check_ownership_matrix("vm1", base, &[mk_file_entry("sync.lock", 0o0600, true)]);
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        match &drifts[0] {
            OwnershipMismatch::StatFailed { not_found, .. } => {
                assert!(*not_found, "required-missing must flag not_found");
            }
            other => panic!("expected StatFailed, got {other:?}"),
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn no_follow_symlink_at_leaf_is_kind_mismatch() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        // A symlink standing in for a dir-kind entry must be reported,
        // not traversed (no-follow `symlink_metadata`).
        let real = base.join("real-dir");
        stdfs::create_dir(&real).unwrap();
        symlink(&real, base.join("state")).unwrap();

        let drifts = check_ownership_matrix("vm1", base, &[mk_entry("state", 0o0750)]);
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        match &drifts[0] {
            OwnershipMismatch::KindMismatch { actual_kind, .. } => {
                assert_eq!(actual_kind, "symlink");
            }
            other => panic!("expected KindMismatch for symlink, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Reset ownership boundary
// ---------------------------------------------------------------------------

#[cfg(test)]
mod reset_tests {
    use super::*;
    use std::fs as stdfs;
    use std::os::unix::fs::symlink;

    const OWNERSHIP_ID: &str = "host:d2b";

    /// A deployment root with its ownership marker and the named entries.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn deployment(root: &Path, owned: &[(&str, OwnedPathKind)]) -> ResetOwnership {
        stdfs::create_dir_all(root).unwrap();
        stdfs::write(
            root.join(OWNERSHIP_MARKER_FILE),
            render_ownership_marker(OWNERSHIP_ID),
        )
        .unwrap();
        ResetOwnership {
            owned: owned
                .iter()
                .map(|(relative, kind)| match kind {
                    OwnedPathKind::Tree => OwnedPath::tree(root.join(relative)),
                    OwnedPathKind::File => OwnedPath::file(root.join(relative)),
                    OwnedPathKind::HardlinkFarm => OwnedPath::hardlink_farm(root.join(relative)),
                })
                .collect(),
            external_sources: Vec::new(),
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn fixture(owned: &[(&str, OwnedPathKind)]) -> (tempfile::TempDir, ResetOwnership) {
        let tmp = tempfile::tempdir().unwrap();
        let ownership = deployment(tmp.path(), owned);
        (tmp, ownership)
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn observed(root: &Path, ownership: &ResetOwnership) -> ResetObservation {
        observe_reset(root, OWNERSHIP_ID, ownership).expect("observe")
    }

    #[test]
    fn marker_round_trips_and_foreign_bodies_have_no_id() {
        assert_eq!(
            parse_ownership_marker(&render_ownership_marker(OWNERSHIP_ID)),
            Some(OWNERSHIP_ID)
        );
        // An unterminated block is not a marker, so ownership cannot be
        // established and the caller refuses rather than reading "" as an id.
        assert_eq!(parse_ownership_marker("# d2b-managed begin\n"), None);
        // Neither is a block that names nothing.
        assert_eq!(
            parse_ownership_marker("# d2b-managed begin\n# note\n# d2b-managed end\n"),
            None
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn matching_marker_verifies_the_exact_inventory() {
        let (tmp, ownership) = fixture(&[("zones", OwnedPathKind::Tree)]);
        stdfs::create_dir_all(tmp.path().join("zones/one")).unwrap();
        let observation = observed(tmp.path(), &ownership);
        let verified = verify_reset(&observation, &ownership, &DrainEvidence::default()).unwrap();
        assert_eq!(verified.ownership_id, OWNERSHIP_ID);
        assert_eq!(verified.entries.len(), 1);
        assert!(verified.entries[0].present);
        assert_eq!(verified.present_entries().count(), 1);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn a_foreign_marker_is_never_an_authorization_to_overwrite() {
        let (tmp, ownership) = fixture(&[("zones", OwnedPathKind::Tree)]);
        stdfs::create_dir(tmp.path().join("zones")).unwrap();
        stdfs::write(
            tmp.path().join(OWNERSHIP_MARKER_FILE),
            render_ownership_marker("host:somebody-else"),
        )
        .unwrap();
        let observation = observed(tmp.path(), &ownership);
        let refusal =
            verify_reset(&observation, &ownership, &DrainEvidence::default()).unwrap_err();
        assert_eq!(refusal.code(), "reset-foreign-ownership-marker");
        assert_eq!(
            refusal,
            ResetRefusal::ForeignOwnershipMarker {
                path: tmp.path().join(OWNERSHIP_MARKER_FILE),
                marker: "host:somebody-else".to_owned(),
            }
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn a_missing_marker_refuses_rather_than_treating_it_as_unowned() {
        let (tmp, ownership) = fixture(&[]);
        stdfs::remove_file(tmp.path().join(OWNERSHIP_MARKER_FILE)).unwrap();
        let refusal = observe_reset(tmp.path(), OWNERSHIP_ID, &ownership).unwrap_err();
        assert_eq!(refusal.code(), "reset-ownership-marker-unreadable");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn a_symlink_on_an_owned_path_is_refused_rather_than_resolved() {
        let (tmp, ownership) = fixture(&[("escape", OwnedPathKind::Tree)]);
        let outside = tmp.path().join("outside");
        stdfs::create_dir(&outside).unwrap();
        symlink(&outside, tmp.path().join("escape")).unwrap();
        let refusal = observe_reset(tmp.path(), OWNERSHIP_ID, &ownership).unwrap_err();
        assert_eq!(refusal.code(), "reset-symlink-escape");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn an_owned_path_outside_the_deployment_root_is_refused() {
        let (tmp, _empty) = fixture(&[]);
        let elsewhere = tempfile::tempdir().unwrap();
        let ownership = ResetOwnership {
            owned: vec![OwnedPath::tree(elsewhere.path().join("data"))],
            external_sources: Vec::new(),
        };
        stdfs::create_dir(elsewhere.path().join("data")).unwrap();
        let refusal = observe_reset(tmp.path(), OWNERSHIP_ID, &ownership).unwrap_err();
        assert_eq!(refusal.code(), "reset-ownership-outside-root");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn a_declared_directory_that_is_a_file_is_refused() {
        let (tmp, ownership) = fixture(&[("zones", OwnedPathKind::Tree)]);
        stdfs::write(tmp.path().join("zones"), b"not a directory").unwrap();
        let observation = observed(tmp.path(), &ownership);
        let refusal =
            verify_reset(&observation, &ownership, &DrainEvidence::default()).unwrap_err();
        assert_eq!(refusal.code(), "reset-ownership-kind-mismatch");
    }

    /// A mounted foreign filesystem cannot be created unprivileged, so the
    /// decision that refuses it is exercised on the observation it reads:
    /// an owned path whose `st_dev` is not the deployment root's.
    #[test]
    fn an_owned_path_on_another_filesystem_is_refused() {
        let (tmp, ownership) = fixture(&[("mounted", OwnedPathKind::Tree)]);
        let mut observation = observed(tmp.path(), &ownership);
        observation.owned[0].present = true;
        observation.owned[0].inode_kind = Some(InodeKind::Dir);
        observation.owned[0].device = Some(observation.deployment_device + 1);
        let refusal =
            verify_reset(&observation, &ownership, &DrainEvidence::default()).unwrap_err();
        assert_eq!(refusal.code(), "reset-foreign-filesystem");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn an_external_volume_source_inside_the_owned_set_is_refused() {
        let (tmp, mut ownership) = fixture(&[("zones", OwnedPathKind::Tree)]);
        stdfs::create_dir(tmp.path().join("zones")).unwrap();
        ownership
            .external_sources
            .push(tmp.path().join("zones/operator-volume"));
        let observation = observed(tmp.path(), &ownership);
        let refusal =
            verify_reset(&observation, &ownership, &DrainEvidence::default()).unwrap_err();
        assert_eq!(refusal.code(), "reset-external-source-owned");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn empty_state_is_not_drain_proof() {
        let (tmp, ownership) = fixture(&[("zones", OwnedPathKind::Tree)]);
        stdfs::create_dir(tmp.path().join("zones")).unwrap();
        let observation = observed(tmp.path(), &ownership);
        // The owned tree is empty; that is the shape fresh state wants and
        // it is not evidence of a drain.
        for evidence in [
            DrainEvidence {
                live_cgroup_members: vec![PathBuf::from("/sys/fs/cgroup/d2b.slice/zones/a/guest")],
                ..DrainEvidence::default()
            },
            DrainEvidence {
                live_processes: vec![PathBuf::from("/proc/1234")],
                ..DrainEvidence::default()
            },
            DrainEvidence {
                live_markers: vec![tmp.path().join("runtime/host-runtime.json")],
                ..DrainEvidence::default()
            },
            DrainEvidence {
                held_leases: vec![tmp.path().join("locks/usbip/1-1.2")],
                ..DrainEvidence::default()
            },
            DrainEvidence {
                active_host_generation: Some("generation-7".to_owned()),
                ..DrainEvidence::default()
            },
        ] {
            let refusal = verify_reset(&observation, &ownership, &evidence).unwrap_err();
            assert_ne!(refusal.code(), "reset-ownership-outside-root");
        }
        assert!(verify_reset(&observation, &ownership, &DrainEvidence::default()).is_ok());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn an_absent_owned_path_is_a_completed_reset_not_a_failure() {
        let (tmp, ownership) = fixture(&[("zones", OwnedPathKind::Tree)]);
        let observation = observed(tmp.path(), &ownership);
        let verified = verify_reset(&observation, &ownership, &DrainEvidence::default()).unwrap();
        assert_eq!(verified.entries.len(), 1);
        assert!(!verified.entries[0].present);
        assert_eq!(verified.present_entries().count(), 0);
    }
}
