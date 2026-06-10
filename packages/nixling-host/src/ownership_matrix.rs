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
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

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
    /// Whether the daemon may recurse into the directory when
    /// checking. The enforcer additionally rejects recursion into the
    /// `store` subdirectory regardless of this flag (hardlink-farm
    /// carve-out).
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
    /// `stat(2)` failed on the declared path (missing directory,
    /// broken symlink under a parent, permission denied for the
    /// daemon's uid).
    StatFailed { path: PathBuf, detail: String },
    /// The path exists but owner/group/mode differ from the matrix.
    Drift {
        path: PathBuf,
        expected: Ownership,
        actual: Ownership,
        drift_reason: DriftReason,
    },
    /// Recursive walk found a child whose owner/group/mode differs.
    /// Children of the `store` subdirectory are NEVER reported here:
    /// the enforcer refuses to recurse into the hardlink farm to
    /// avoid even READING ownership on inodes shared with /nix/store
    /// in a way that could be misinterpreted as a fix-up signal.
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
            Self::Drift { .. } => "ownership-matrix-drift",
            Self::ChildDrift { .. } => "ownership-matrix-child-drift",
        }
    }

    /// Path the mismatch refers to (for operator-visible messaging).
    pub fn path(&self) -> &Path {
        match self {
            Self::StatFailed { path, .. }
            | Self::Drift { path, .. }
            | Self::ChildDrift { path, .. } => path.as_path(),
        }
    }
}

/// Per-VM state root identifier of the hardlink-farm carve-out.
///
/// The string is compared byte-for-byte against `entry.path`. Kept
/// pub(crate) so the matching test in [`tests`] can re-assert the
/// canonical value.
const HARDLINK_FARM_CARVE_OUTS: &[&str] = &["store", "store-view/live"];

/// Return whether the enforcer is permitted to recurse into this
/// entry. Combines the operator-declared `recursive` flag with the
/// hardlink-farm carve-out: even if a future operator typo flips
/// `recursive = true` on the `store` entry, this function still
/// returns `false`.
pub fn should_recurse(entry: &OwnershipEntry) -> bool {
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

        let meta = match fs::symlink_metadata(&target) {
            Ok(m) => m,
            Err(err) => {
                drifts.push(OwnershipMismatch::StatFailed {
                    path: target,
                    detail: err.to_string(),
                });
                continue;
            }
        };

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
                out.push(OwnershipMismatch::StatFailed {
                    path,
                    detail: err.to_string(),
                });
                continue;
            }
        };
        if let Some(dev) = root_dev {
            if meta.dev() != dev {
                continue;
            }
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
        };
        assert_eq!(m.kind(), "ownership-matrix-stat-failed");
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
}
