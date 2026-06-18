//! Daemon-side enforcer for the per-VM state directory ownership matrix
//! declared in
//! `nixos-modules/options-ownership-matrix.nix`.
//!
//! # Invariant: hardlink-farm carve-out
//!
//! `/var/lib/nixling/vms/<vm>/store/` is a per-generation hardlink
//! farm whose inodes are SHARED with `/nix/store`. Recursive
//! ownership / mode / ACL operations across that subtree propagate
//! INTO `/nix/store` via the shared inodes, which breaks the openssh
//! `safe_path()` checks on per-VM ssh host keys (the canonical
//! regression hit on personal-dev — see plan.md §"Ownership matrix
//! for `/var/lib/nixling/vms/<vm>/`" critical-detail note).
//!
//! The enforcer therefore:
//!
//! 1. NEVER recurses into the `store` subdirectory regardless of the
//!    declared `recursive` field. The carve-out is asserted in
//!    [`should_recurse`] and unit-tested in [`tests`].
//! 2. Performs only `stat(2)` + comparison; it does NOT mutate
//!    ownership/mode. Mutation belongs to the broker's
//!    host-prepare dispatch surface (audited path).
//! 3. Returns a typed [`OwnershipMismatch`] list per drift so the
//!    caller (nixlingd VM-start preflight) can surface a
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
    /// Subdirectory under `/var/lib/nixling/vms/<vm>/`. Use `"."` for
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
    /// path. `not_found` distinguishes `ENOENT` (the path is absent —
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
                    // EACCES / EIO / ELOOP / … — always fail-closed,
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

    #[test]
    fn stat_failed_for_missing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        prepare(base, ".", 0o0750);

        let matrix = vec![mk_entry("does-not-exist", 0o0750)];
        let drifts = check_ownership_matrix("vm1", base, &matrix);
        assert_eq!(drifts.len(), 1);
        assert!(matches!(drifts[0], OwnershipMismatch::StatFailed { .. }));
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
