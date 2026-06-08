#!/usr/bin/env bash
# Per-VM DAG executor topology + reconciliation.
#
# Drives the supervisor::dag and supervisor::state +
# daemon_version test surfaces so any regression in topo sort,
# fail-fast skip propagation, /proc/<pid>/stat parsing, or the
# daemon pending-restart classifier surfaces on every PR-loop run.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_W4_H10_DAG_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "dag-topo: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_W4_H10_DAG_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache nixpkgs#jq \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir workspace)}
export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

log "==> tests/dag-topo.sh"

cd "$ROOT/packages"

dag_tests=(
  supervisor::dag::tests::topo_sort_linear_dag
  supervisor::dag::tests::topo_sort_diamond_emits_both_branches
  supervisor::dag::tests::topo_sort_detects_cycle
  supervisor::dag::tests::topo_sort_rejects_self_loop_as_cycle
  supervisor::dag::tests::topo_sort_rejects_unknown_edge_target
  supervisor::dag::tests::topo_sort_rejects_duplicate_node_ids
  supervisor::dag::tests::executor_runs_all_nodes_in_topo_order_on_success
  supervisor::dag::tests::executor_fail_fast_skips_remaining_nodes
  supervisor::dag::tests::executor_propagates_topo_error
  supervisor::dag::tests::budget_threaded_to_runner
  supervisor::dag::tests::report_round_trip_serializable
)

state_tests=(
  supervisor::state::tests::parses_simple_proc_stat
  supervisor::state::tests::parses_comm_with_spaces_and_parens
  supervisor::state::tests::parser_rejects_missing_comm_close
  supervisor::state::tests::parser_rejects_short_tail
  supervisor::state::tests::parser_rejects_non_integer_starttime
  supervisor::state::tests::parser_strips_trailing_newline
  supervisor::state::tests::adopts_matching_record
  supervisor::state::tests::quarantines_drifted_pid
  supervisor::state::tests::marks_missing_when_proc_pid_gone
  supervisor::state::tests::marks_unparseable_when_reader_errors
  supervisor::state::tests::report_ordered_by_vm_then_role
  supervisor::state::tests::empty_snapshots_produce_empty_report
  supervisor::state::tests::in_memory_store_round_trip
  supervisor::state::tests::in_memory_remove_missing_is_ok
  supervisor::state::tests::filesystem_store_round_trip
  supervisor::state::tests::filesystem_store_upsert_replaces_existing
  supervisor::state::tests::filesystem_store_remove_missing_is_ok
  supervisor::state::tests::filesystem_store_list_skips_non_runtime_files
  supervisor::state::tests::filesystem_store_list_handles_missing_root
  supervisor::state::tests::snapshot_record_round_trip_json
  supervisor::state::tests::snapshot_record_rejects_unknown_fields
)

daemon_version_tests=(
  daemon_version::tests::up_to_date_when_paths_match
  daemon_version::tests::pending_restart_when_paths_differ
  daemon_version::tests::daemon_not_running_when_version_file_missing
  daemon_version::tests::version_file_unreadable_surfaces_detail
  daemon_version::tests::missing_install_path_treats_as_up_to_date
  daemon_version::tests::banner_up_to_date
  daemon_version::tests::banner_pending_includes_both_paths
  daemon_version::tests::banner_not_running
  daemon_version::tests::banner_unreadable_includes_detail
  daemon_version::tests::version_file_round_trip_serializable
  daemon_version::tests::version_file_rejects_unknown_fields
  daemon_version::tests::restart_status_round_trip_serializable
)

log "  canary: supervisor::dag (${#dag_tests[@]} cases)"
for t in "${dag_tests[@]}"; do
  cargo test -p nixlingd --lib "$t" 2>&1 | tail -3
done

log "  canary: supervisor::state (${#state_tests[@]} cases)"
for t in "${state_tests[@]}"; do
  cargo test -p nixlingd --lib "$t" 2>&1 | tail -3
done

log "  canary: daemon_version (${#daemon_version_tests[@]} cases)"
for t in "${daemon_version_tests[@]}"; do
  cargo test -p nixlingd --lib "$t" 2>&1 | tail -3
done

ok "tests/dag-topo.sh: every supervisor DAG canary passed"
