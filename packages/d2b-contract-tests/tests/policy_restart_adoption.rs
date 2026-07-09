//! Restart/adoption design-contract gates for W13/W16.
//!
//! These source-lint and structural gates enforce the key architectural
//! invariant: **workload identity in the read model is config-driven, not
//! state-driven**. Daemon restart preserves workload identity because the
//! daemon rebuilds the `WorkloadTargetIndex` from `realm-controllers.json`
//! on every public request — no workload identity is persisted in the runner
//! snapshot records.
//!
//! Coverage:
//!  * `RunnerSnapshotRecord` in `supervisor/state.rs` does NOT carry a
//!    `workload_identity` field — process adoption tracks `(pid,
//!    start_time_ticks)`, not realm identity.
//!  * `WorkloadTargetIndex` is re-built per request in `lib.rs`
//!    (`build_from_controllers` call site) rather than stored in `ServerState`.
//!  * `identity_for_vm` is used (not a hardcoded None) to populate list/status
//!    `workloadIdentity`, so realm-registered workloads always show their
//!    identity after restart.
//!  * The `daemon-restart-vm-survival.nix` runNixOSTest exists as the live
//!    process-identity gate; `workload-identity-restart.nix` covers the
//!    read-model workload-identity gate.

use d2b_contract_tests::{read_repo_file, repo_path_exists};

fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = regex::Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

// ── RunnerSnapshotRecord: no workload_identity field ────────────────────────

/// `RunnerSnapshotRecord` in `supervisor/state.rs` must NOT have a
/// `workload_identity` field.  Process adoption is keyed on
/// `(pid, start_time_ticks)` — injecting workload identity into the snapshot
/// would create a second, divergent source of truth that could drift from the
/// config-driven `WorkloadTargetIndex`.
#[test]
fn snapshot_record_does_not_carry_workload_identity() {
    let src = read_repo_file("packages/d2bd/src/supervisor/state.rs");

    // Verify the struct definition is present (so the test isn't vacuous).
    assert!(
        any_line_matches(&src, r"pub struct RunnerSnapshotRecord"),
        "policy_restart_adoption: RunnerSnapshotRecord definition not found in state.rs"
    );

    // Scan only the struct body (lines between `RunnerSnapshotRecord {` and
    // the closing `}`) for a `workload_identity` field.  We find the struct
    // block by collecting lines after the struct header until we hit an
    // unindented `}`.
    let struct_body: String = {
        let mut in_struct = false;
        let mut depth: usize = 0;
        let mut body = String::new();
        for line in src.lines() {
            if !in_struct {
                if line.contains("pub struct RunnerSnapshotRecord") {
                    in_struct = true;
                    depth = 0;
                }
                continue;
            }
            // Track brace depth to find the closing `}`.
            depth += line.chars().filter(|&c| c == '{').count();
            depth = depth.saturating_sub(line.chars().filter(|&c| c == '}').count());
            if depth == 0 && line.trim() == "}" {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        body
    };

    assert!(
        !any_line_matches(&struct_body, r"workload_identity"),
        "policy_restart_adoption: RunnerSnapshotRecord must not carry a \
         `workload_identity` field — process adoption is keyed on \
         (pid, start_time_ticks), not realm identity; workload identity \
         must be sourced from the config-driven WorkloadTargetIndex on \
         every public request"
    );
}

// ── WorkloadTargetIndex: rebuilt per-request, not stored in ServerState ──────

/// `packages/d2bd/src/lib.rs` must call `WorkloadTargetIndex::build_from_controllers`
/// (not store it in `ServerState`) so the index is always fresh after a restart.
#[test]
fn workload_target_index_is_rebuilt_per_request_in_lib_rs() {
    let src = read_repo_file("packages/d2bd/src/lib.rs");

    assert!(
        any_line_matches(&src, r"WorkloadTargetIndex::build_from_controllers"),
        "policy_restart_adoption: WorkloadTargetIndex::build_from_controllers call \
         site missing from packages/d2bd/src/lib.rs — the index must be rebuilt on \
         every public request so workload identity is always fresh post-restart"
    );
}

/// `packages/d2bd/src/lib.rs` must call `identity_for_vm` to populate the
/// read-model `workloadIdentity` field.  Without this call, list/status would
/// silently omit workload identity after restart even though the config is
/// correct.
#[test]
fn identity_for_vm_is_used_to_populate_list_status() {
    let src = read_repo_file("packages/d2bd/src/lib.rs");

    assert!(
        any_line_matches(&src, r"identity_for_vm"),
        "policy_restart_adoption: `identity_for_vm` call missing from \
         packages/d2bd/src/lib.rs — list/status workloadIdentity population \
         must use the WorkloadTargetIndex lookup so the field is present \
         after daemon restart"
    );
}

// ── Host-integration gate files exist ───────────────────────────────────────

/// The live process-identity gate (`daemon-restart-vm-survival.nix`) must
/// exist — it verifies PID/pidfd adoption across restarts.
#[test]
fn daemon_restart_vm_survival_host_integration_test_exists() {
    assert!(
        repo_path_exists("tests/host-integration/daemon-restart-vm-survival.nix"),
        "policy_restart_adoption: tests/host-integration/daemon-restart-vm-survival.nix \
         is missing — this runNixOSTest is the live gate for process-identity \
         adoption across daemon restarts"
    );
}

/// The workload-identity read-model gate (`workload-identity-restart.nix`)
/// must exist — it verifies that `d2b list --json` emits `workloadIdentity`
/// for realm-registered workloads after daemon restart.
#[test]
fn workload_identity_restart_host_integration_test_exists() {
    assert!(
        repo_path_exists("tests/host-integration/workload-identity-restart.nix"),
        "policy_restart_adoption: tests/host-integration/workload-identity-restart.nix \
         is missing — this runNixOSTest is the required gate (per W13/W16 plan) \
         proving workload identity in the read model survives daemon restart"
    );
}
