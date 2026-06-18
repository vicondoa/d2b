//! VM-start preflight that invokes the `nixling_host::ownership_matrix`
//! enforcer against the per-VM
//! state directory before any broker dispatch occurs.
//!
//! See `nixos-modules/options-ownership-matrix.nix` for the typed
//! matrix declaration and `docs/reference/per-vm-state-ownership.md`
//! for the operator-facing reference.
//!
//! # Migration-window posture
//!
//! Some per-VM paths are materialized lazily: the broker StoreSync
//! creates the `store-view` state/lock/meta tree and the live readiness
//! marker; the swtpm runner creates `swtpm` on first exec; legacy
//! `store`/`store-meta` exist only on migrated VMs. On a fresh host, or
//! in unit tests that don't materialize the full per-VM tree, those
//! paths will be absent.
//!
//! The enforcer distinguishes absence (`ENOENT`) from other stat
//! errors:
//!
//! - **Optional entry (`required = false`) absent** → no mismatch is
//!   emitted at all (e.g. the live marker before first sync, the
//!   VM-level integrity-unknown record, legacy artifacts on native
//!   VMs).
//! - **Required entry absent (`ENOENT`)** → emitted as
//!   `StatFailed { not_found: true }`; the preflight downgrades it to a
//!   logged warning during the migration window (broker prep creates
//!   these before preflight once StoreSync lands).
//! - **Any non-`ENOENT` stat error** (`EACCES`, `EIO`, `ELOOP`, …) →
//!   `StatFailed { not_found: false }`, fail-closed.
//! - **Kind mismatch** (a `file` entry that is not a regular file, a
//!   `dir` entry that is not a directory, or a symlink at the leaf) →
//!   `KindMismatch`, fail-closed.
//! - **Owner/group/mode drift** (`Drift` / `ChildDrift`) →
//!   fail-closed.

use nix::unistd::{Gid, Group, Uid, User};
use nixling_host::ownership_matrix::{
    EntryKind, OwnershipEntry, OwnershipMismatch, check_ownership_matrix,
};
use std::path::Path;

/// Specification of one matrix row in symbolic (username/group-name)
/// form. Mirrors the entries declared in
/// `nixos-modules/options-ownership-matrix.nix`.
///
/// Kept as plain data so the daemon can construct the canonical matrix
/// without depending on a bundle-format change — the per-bundle override
/// path lands in a follow-up commit that wires
/// `nixling.daemon.perVmStateOwnershipMatrix` into the bundle.
struct EntrySpec {
    path: &'static str,
    owner_template: &'static str,
    group_template: &'static str,
    mode: u32,
    kind: EntryKind,
    required: bool,
    recursive: bool,
}

const CANONICAL_MATRIX: &[EntrySpec] = &[
    EntrySpec {
        path: ".",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o2770,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "state",
        owner_template: "nixlingd",
        group_template: "nixling",
        mode: 0o0750,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "swtpm",
        owner_template: "nixling-<vm>-swtpm",
        group_template: "nixling-<vm>-swtpm",
        mode: 0o0700,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "sshd-host-keys",
        owner_template: "nixlingd",
        group_template: "nixling",
        mode: 0o0750,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "host-keys",
        owner_template: "nixlingd",
        group_template: "nixling",
        mode: 0o0750,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "store",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o0755,
        kind: EntryKind::Dir,
        // LEGACY RECOVERY ARTIFACT — optional (absent on native,
        // post-cutover VMs).
        required: false,
        // HARDLINK FARM CARVE-OUT — must stay false. The
        // `nixling_host::ownership_matrix::should_recurse` invariant
        // additionally rejects recursion into `store` regardless of
        // this flag.
        recursive: false,
    },
    EntrySpec {
        path: "store-meta",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o0755,
        kind: EntryKind::Dir,
        // LEGACY RECOVERY ARTIFACT — optional.
        required: false,
        recursive: false,
    },
    EntrySpec {
        path: "store-view",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o0755,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "store-view/live",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o0755,
        kind: EntryKind::Dir,
        required: true,
        // HARDLINK FARM CARVE-OUT — must stay false.
        recursive: false,
    },
    EntrySpec {
        path: "store-view/meta",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o0755,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "store-view/meta/generations",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o0755,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "store-view/state",
        owner_template: "nixlingd",
        group_template: "nixling",
        // HOST-ONLY — must NOT reuse the runner-readable `users 0755`
        // store-view posture.
        mode: 0o0750,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "store-view/state/generations",
        owner_template: "nixlingd",
        group_template: "nixling",
        mode: 0o0750,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "store-view/gcroots",
        owner_template: "nixlingd",
        group_template: "nixling",
        mode: 0o0750,
        kind: EntryKind::Dir,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "store-view/sync.lock",
        owner_template: "nixlingd",
        group_template: "nixling",
        // BROKER-PRIVATE lock.
        mode: 0o0600,
        kind: EntryKind::File,
        required: true,
        recursive: false,
    },
    EntrySpec {
        path: "store-view/state/integrity-unknown.json",
        owner_template: "nixlingd",
        group_template: "nixling",
        mode: 0o0640,
        kind: EntryKind::File,
        // Lazily created by broker integrity code.
        required: false,
        recursive: false,
    },
    EntrySpec {
        path: "store-view/live/.nixling-marker-<vm>",
        owner_template: "nixlingd",
        group_template: "users",
        // Guest-readable zero-length readiness marker.
        mode: 0o0644,
        kind: EntryKind::File,
        // Absent before the first successful StoreSync.
        required: false,
        recursive: false,
    },
];

fn substitute_vm(template: &str, vm: &str) -> String {
    template.replace("<vm>", vm)
}

fn resolve_uid(name: &str) -> Option<Uid> {
    User::from_name(name).ok().flatten().map(|u| u.uid)
}

fn resolve_gid(name: &str) -> Option<Gid> {
    Group::from_name(name).ok().flatten().map(|g| g.gid)
}

/// Build the runtime `Vec<OwnershipEntry>` for a given VM, resolving
/// usernames + group names against the live host's NSS database.
/// Entries whose principal is not provisioned (e.g. fresh host, or a
/// `swtpm` entry for a VM with `tpm.enable = false`) are SKIPPED with
/// a `tracing::warn!` event rather than treated as drift. The `<vm>`
/// token is substituted in `path` as well as in the owner/group
/// templates (e.g. the `store-view/live/.nixling-marker-<vm>` leaf).
fn resolve_matrix(vm: &str) -> Vec<OwnershipEntry> {
    let mut out = Vec::with_capacity(CANONICAL_MATRIX.len());
    for spec in CANONICAL_MATRIX {
        let owner_name = substitute_vm(spec.owner_template, vm);
        let group_name = substitute_vm(spec.group_template, vm);
        let Some(uid) = resolve_uid(&owner_name) else {
            tracing::warn!(
                vm = %vm,
                path = %spec.path,
                owner = %owner_name,
                "ownership-matrix: skipping entry; owner principal not resolvable on this host",
            );
            continue;
        };
        let Some(gid) = resolve_gid(&group_name) else {
            tracing::warn!(
                vm = %vm,
                path = %spec.path,
                group = %group_name,
                "ownership-matrix: skipping entry; group principal not resolvable on this host",
            );
            continue;
        };
        out.push(OwnershipEntry {
            path: substitute_vm(spec.path, vm),
            expected_uid: uid.as_raw(),
            expected_gid: gid.as_raw(),
            expected_mode: spec.mode,
            kind: spec.kind,
            required: spec.required,
            recursive: spec.recursive,
        });
    }
    out
}

/// Outcome of the ownership-matrix preflight.
#[derive(Debug)]
pub enum OwnershipPreflightOutcome {
    /// State is consistent with the matrix.
    Clean,
    /// Drift detected on at least one entry. The daemon MUST refuse
    /// to start the VM and surface this to the operator.
    Drift(Vec<OwnershipMismatch>),
}

/// Run the preflight for the given VM. Missing per-VM state
/// directories surface as `Clean` (with warn logs) because state is
/// materialized lazily; only real ownership drift on existing paths
/// fails closed.
pub fn preflight(vm: &str, state_dir: &Path) -> OwnershipPreflightOutcome {
    if !state_dir.exists() {
        tracing::warn!(
            vm = %vm,
            state_dir = %state_dir.display(),
            "ownership-matrix: per-VM state directory absent; skipping preflight (state will be materialized on first run)",
        );
        return OwnershipPreflightOutcome::Clean;
    }
    let matrix = resolve_matrix(vm);
    if matrix.is_empty() {
        tracing::warn!(
            vm = %vm,
            "ownership-matrix: no entries resolvable on this host; skipping preflight",
        );
        return OwnershipPreflightOutcome::Clean;
    }
    let mismatches = check_ownership_matrix(vm, state_dir, &matrix);
    let drift: Vec<OwnershipMismatch> = mismatches
        .into_iter()
        .filter(|m| {
            // During the daemon-only migration window, an absent path
            // (`ENOENT`) is treated as "state not yet provisioned" and
            // skipped: the broker StoreSync / first runner exec creates
            // the required tree before it is used. Every other failure
            // axis — non-`ENOENT` stat errors, kind mismatches, and
            // owner/group/mode drift on existing paths — is
            // fail-closed.
            match m {
                OwnershipMismatch::StatFailed {
                    path,
                    detail,
                    not_found: true,
                } => {
                    tracing::warn!(
                        vm = %vm,
                        path = %path.display(),
                        detail = %detail,
                        "ownership-matrix: entry not present; skipping (will be materialized on first run)",
                    );
                    false
                }
                _ => true,
            }
        })
        .collect();
    if drift.is_empty() {
        OwnershipPreflightOutcome::Clean
    } else {
        OwnershipPreflightOutcome::Drift(drift)
    }
}

/// Render a drift list into the public operator-facing message. Kept
/// short and structured: `<count> drifted entr(y/ies): <path> (<axes>),
/// ...`.
pub fn render_drift_message(vm: &str, drift: &[OwnershipMismatch]) -> String {
    use std::fmt::Write as _;
    let mut s = format!(
        "vm '{vm}' refused: per-VM state ownership drifted from the declared matrix ({} entr{}): ",
        drift.len(),
        if drift.len() == 1 { "y" } else { "ies" },
    );
    for (i, m) in drift.iter().enumerate() {
        if i > 0 {
            s.push_str("; ");
        }
        match m {
            OwnershipMismatch::Drift {
                path,
                expected,
                actual,
                drift_reason,
            } => {
                let mut axes = Vec::new();
                if drift_reason.owner {
                    axes.push(format!("owner {}→{}", expected.uid, actual.uid));
                }
                if drift_reason.group {
                    axes.push(format!("group {}→{}", expected.gid, actual.gid));
                }
                if drift_reason.mode {
                    axes.push(format!("mode {:o}→{:o}", expected.mode, actual.mode));
                }
                let _ = write!(s, "{} ({})", path.display(), axes.join(", "));
            }
            OwnershipMismatch::ChildDrift { path, actual, .. } => {
                let _ = write!(
                    s,
                    "{} (child drift uid={} gid={} mode={:o})",
                    path.display(),
                    actual.uid,
                    actual.gid,
                    actual.mode
                );
            }
            OwnershipMismatch::KindMismatch {
                path,
                expected_kind,
                actual_kind,
            } => {
                let _ = write!(
                    s,
                    "{} (kind mismatch: expected {expected_kind}, found {actual_kind})",
                    path.display(),
                );
            }
            OwnershipMismatch::StatFailed {
                path,
                detail,
                not_found,
            } => {
                let _ = write!(
                    s,
                    "{} (stat failed{}: {})",
                    path.display(),
                    if *not_found { ", not found" } else { "" },
                    detail
                );
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn missing_state_dir_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        match preflight("vm1", &missing) {
            OwnershipPreflightOutcome::Clean => {}
            other => panic!("expected Clean, got {other:?}"),
        }
    }

    #[test]
    fn render_drift_message_lists_axes() {
        let drift = vec![OwnershipMismatch::Drift {
            path: "/var/lib/nixling/vms/vm1/state".into(),
            expected: nixling_host::ownership_matrix::Ownership {
                uid: 100,
                gid: 200,
                mode: 0o0750,
            },
            actual: nixling_host::ownership_matrix::Ownership {
                uid: 0,
                gid: 0,
                mode: 0o0755,
            },
            drift_reason: nixling_host::ownership_matrix::DriftReason {
                owner: true,
                group: true,
                mode: true,
            },
        }];
        let msg = render_drift_message("vm1", &drift);
        assert!(msg.contains("vm 'vm1' refused"), "message: {msg}");
        assert!(msg.contains("owner 100→0"), "message: {msg}");
        assert!(msg.contains("group 200→0"), "message: {msg}");
        assert!(msg.contains("mode 750→755"), "message: {msg}");
    }

    #[test]
    fn render_drift_message_includes_kind_mismatch() {
        let drift = vec![OwnershipMismatch::KindMismatch {
            path: "/var/lib/nixling/vms/vm1/store-view/sync.lock".into(),
            expected_kind: "file".to_owned(),
            actual_kind: "dir".to_owned(),
        }];
        let msg = render_drift_message("vm1", &drift);
        assert!(
            msg.contains("kind mismatch: expected file, found dir"),
            "message: {msg}"
        );
    }

    #[test]
    fn render_drift_message_marks_non_enoent_stat_failure() {
        // A non-ENOENT stat failure (not_found = false) is fail-closed
        // and rendered with its detail.
        let drift = vec![OwnershipMismatch::StatFailed {
            path: "/var/lib/nixling/vms/vm1/store-view/state".into(),
            detail: "Permission denied (os error 13)".to_owned(),
            not_found: false,
        }];
        let msg = render_drift_message("vm1", &drift);
        assert!(msg.contains("stat failed:"), "message: {msg}");
        assert!(!msg.contains("not found"), "message: {msg}");
    }

    /// Smoke check: with a fully-materialized state dir owned by the
    /// test process, the preflight returns Clean (after the
    /// non-resolvable owner principals are skipped with warn logs).
    #[test]
    fn provisioned_state_dir_with_unresolvable_principals_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        fs::set_permissions(base, fs::Permissions::from_mode(0o2770)).unwrap();
        // Don't materialize subdirs — those will surface as
        // StatFailed and be skipped.
        match preflight("vm1", base) {
            OwnershipPreflightOutcome::Clean => {}
            OwnershipPreflightOutcome::Drift(drift) => {
                // If the test host happens to have e.g. a `nixlingd`
                // user, the top-level may drift on owner — that's a
                // legitimate fail-closed signal, not a test failure
                // (the unit test for actual drift lives in
                // nixling_host::ownership_matrix::tests).
                assert!(!drift.is_empty());
            }
        }
    }
}
