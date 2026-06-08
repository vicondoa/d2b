#!/usr/bin/env bash
# Pidfd handoff contract.
#
# Plan ref: ~/.copilot/session-state/<id>/plan.md
#   §" pidfd handoff contract"
#
# Asserts:
#   - SCM_RIGHTS transports a pidfd from broker → daemon with CLOEXEC
#     preserved (integration test
#     `pidfd_handoff_scm_rights::scm_rights_transports_pidfd_with_cloexec_preserved`);
#   - reconciliation refuses pid+start-time drift
#     (`reconciliation_refuses_start_time_drift`);
#   - `nixlingd::supervisor::pidfd::PidfdTable` registers an AsyncFd in
#     the tokio epoll/poll loop, refuses duplicate registration, and
#     forbids raw-pid kill/wait;
#   - `nixlingd::supervisor::pidfd::set_child_subreaper_with_self_test`
#     succeeds and is idempotent.
#

set -euo pipefail

HERE=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

CARGO=${CARGO:-cargo}
if ! command -v "$CARGO" >/dev/null 2>&1 && ! command -v rustup >/dev/null 2>&1; then
  echo "pidfd-handoff: cargo not on PATH; expected the static gate's rust shell" >&2
  exit 1
fi

run_cargo() {
  if command -v rustup >/dev/null 2>&1 && [ -n "${RUSTUP_TOOLCHAIN:-}" ]; then
    RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" rustup run "$RUSTUP_TOOLCHAIN" cargo "$@"
  else
    RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" "$CARGO" "$@"
  fi
}

LOG_DIR=${TMPDIR:-/tmp}/nixling-pidfd-handoff.$$
mkdir -p "$LOG_DIR"
cleanup() { rm -rf -- "$LOG_DIR"; }
trap cleanup EXIT INT TERM

declare -a BROKER_INTEGRATION=(
  scm_rights_transports_pidfd_with_cloexec_preserved
  reconciliation_refuses_start_time_drift
)

declare -a BROKER_UNIT=(
  ops::pidfd::tests::cloexec_assertion_rejects_non_cloexec
  ops::pidfd::tests::fake_spawner_yields_cloexec_pidfd
  ops::pidfd::tests::reconcile_succeeds_on_matching_start_time
  ops::pidfd::tests::reconcile_refuses_on_start_time_drift
  ops::pidfd::tests::parses_proc_stat_field_22
)

declare -a DAEMON_CANARIES=(
  supervisor::pidfd_table::tests::registers_and_deregisters
  supervisor::pidfd_table::tests::refuses_duplicate_registration
  supervisor::pidfd_table::tests::child_subreaper_self_test_takes_effect
  supervisor::pidfd_table::tests::register_signal_snapshot_roundtrip
  supervisor::pidfd_table::tests::wait_terminated_times_out_for_running_child
)

BROKER_INT_LOG="$LOG_DIR/broker-integration.log"
BROKER_UNIT_LOG="$LOG_DIR/broker-unit.log"
DAEMON_LOG="$LOG_DIR/daemon.log"

printf '\n[pidfd-handoff] broker SCM_RIGHTS integration test\n'
set +e
(
  cd "$ROOT/packages/nixling-priv-broker"
  run_cargo test --all-features --test pidfd_handoff_scm_rights
) >"$BROKER_INT_LOG" 2>&1
status=$?
set -e
if [ $status -ne 0 ]; then
  printf 'pidfd-handoff: broker integration block failed (status %d)\n' "$status" >&2
  tail -80 "$BROKER_INT_LOG" >&2
  exit 1
fi
for canary in "${BROKER_INTEGRATION[@]}"; do
  if ! grep -qE "^test ${canary} \\.\\.\\. ok" "$BROKER_INT_LOG"; then
    printf 'pidfd-handoff: missing pass marker for integration test %s\n' "$canary" >&2
    tail -120 "$BROKER_INT_LOG" >&2
    exit 1
  fi
done
printf '  ok: SCM_RIGHTS pidfd transport + CLOEXEC preservation\n'

printf '\n[pidfd-handoff] broker unit canaries (ops::pidfd)\n'
set +e
(
  cd "$ROOT/packages/nixling-priv-broker"
  run_cargo test --all-features --lib ops::pidfd
) >"$BROKER_UNIT_LOG" 2>&1
status=$?
set -e
if [ $status -ne 0 ]; then
  printf 'pidfd-handoff: broker unit block failed (status %d)\n' "$status" >&2
  tail -80 "$BROKER_UNIT_LOG" >&2
  exit 1
fi
for canary in "${BROKER_UNIT[@]}"; do
  if ! grep -qE "^test ${canary} \\.\\.\\. ok" "$BROKER_UNIT_LOG"; then
    printf 'pidfd-handoff: missing pass marker for %s\n' "$canary" >&2
    tail -120 "$BROKER_UNIT_LOG" >&2
    exit 1
  fi
done
printf '  ok: %d broker unit canaries passed\n' "${#BROKER_UNIT[@]}"

printf '\n[pidfd-handoff] daemon supervisor canaries (supervisor::pidfd_table)\n'
set +e
(
  cd "$ROOT/packages"
  run_cargo test -p nixlingd --lib supervisor::pidfd_table
) >"$DAEMON_LOG" 2>&1
status=$?
set -e
if [ $status -ne 0 ]; then
  printf 'pidfd-handoff: daemon block failed (status %d)\n' "$status" >&2
  tail -80 "$DAEMON_LOG" >&2
  exit 1
fi
for canary in "${DAEMON_CANARIES[@]}"; do
  if ! grep -qE "^test ${canary} \\.\\.\\. ok" "$DAEMON_LOG"; then
    printf 'pidfd-handoff: missing pass marker for %s\n' "$canary" >&2
    tail -120 "$DAEMON_LOG" >&2
    exit 1
  fi
done
printf '  ok: %d daemon supervisor canaries passed\n' "${#DAEMON_CANARIES[@]}"

printf '\npidfd-handoff: all pidfd canaries passed\n'
exit 0
