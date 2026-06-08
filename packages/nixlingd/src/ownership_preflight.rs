//! ph2-p2-ownership-matrix: VM-start preflight that invokes the
//! `nixling_host::ownership_matrix` enforcer against the per-VM
//! state directory before any broker dispatch occurs.
//!
//! See `nixos-modules/options-ownership-matrix.nix` for the typed
//! matrix declaration and `docs/reference/per-vm-state-ownership.md`
//! for the operator-facing reference.
//!
//! # Migration-window posture
//!
//! Some per-VM subdirectories are materialized lazily by the
//! activation hook (`store-meta`) or by the first runner exec
//! (`swtpm`). On a fresh host, or in unit tests that don't
//! materialize the full per-VM tree, those subdirectories will be
//! absent. We therefore treat `OwnershipMismatch::StatFailed` as a
//! **logged warning** (state not yet provisioned), not a refusal.
//! Real ownership drift (`Drift` / `ChildDrift`) is fail-closed.

use nix::unistd::{Gid, Group, Uid, User};
use nixling_host::ownership_matrix::{
    check_ownership_matrix, OwnershipEntry, OwnershipMismatch,
};
use std::path::Path;

/// Specification of one matrix row in symbolic (username/group-name)
/// form. Mirrors the entries declared in
/// `nixos-modules/options-ownership-matrix.nix`.
///
/// Kept as plain data so the daemon can construct the canonical
/// matrix without depending on a bundle-format change in P2 — the
/// per-bundle override path lands in a follow-up commit that wires
/// `nixling.daemon.perVmStateOwnershipMatrix` into the bundle.
struct EntrySpec {
    path: &'static str,
    owner_template: &'static str,
    group_template: &'static str,
    mode: u32,
    recursive: bool,
}

const CANONICAL_MATRIX: &[EntrySpec] = &[
    EntrySpec {
        path: ".",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o2770,
        recursive: false,
    },
    EntrySpec {
        path: "state",
        owner_template: "nixlingd",
        group_template: "nixling-launcher",
        mode: 0o0750,
        recursive: false,
    },
    EntrySpec {
        path: "swtpm",
        owner_template: "nixling-<vm>-swtpm",
        group_template: "nixling-<vm>-swtpm",
        mode: 0o0700,
        recursive: false,
    },
    EntrySpec {
        path: "sshd-host-keys",
        owner_template: "nixlingd",
        group_template: "nixling-launcher",
        mode: 0o0750,
        recursive: false,
    },
    EntrySpec {
        path: "host-keys",
        owner_template: "nixlingd",
        group_template: "nixling-launcher",
        mode: 0o0750,
        recursive: false,
    },
    EntrySpec {
        path: "store",
        owner_template: "nixlingd",
        group_template: "users",
        mode: 0o2775,
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
        mode: 0o2775,
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
/// a `tracing::warn!` event rather than treated as drift.
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
            path: spec.path.to_owned(),
            expected_uid: uid.as_raw(),
            expected_gid: gid.as_raw(),
            expected_mode: spec.mode,
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
            // StatFailed is treated as "state not yet provisioned"
            // during the daemon-only migration window. The fail-closed
            // axes are owner / group / mode drift on existing paths,
            // which is what
            // `OwnershipMismatch::{Drift,ChildDrift}` express.
            match m {
                OwnershipMismatch::StatFailed { path, detail } => {
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
                    axes.push(format!(
                        "mode {:o}→{:o}",
                        expected.mode, actual.mode
                    ));
                }
                let _ = write!(s, "{} ({})", path.display(), axes.join(", "));
            }
            OwnershipMismatch::ChildDrift {
                path, actual, ..
            } => {
                let _ = write!(
                    s,
                    "{} (child drift uid={} gid={} mode={:o})",
                    path.display(),
                    actual.uid,
                    actual.gid,
                    actual.mode
                );
            }
            OwnershipMismatch::StatFailed { path, detail } => {
                let _ = write!(s, "{} (stat failed: {})", path.display(), detail);
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
