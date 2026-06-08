#!/usr/bin/env bash
# Cgroup v2 delegation canary matrix.
#
# This gate drives the algorithm through the in-memory
# `nixling_host::cgroup::fake::FakeCgroupBackend` and the broker's
# `ops::cgroup::test_harness::RecordingAuditSink`. Each canary
# corresponds to a named `cargo test` in `nixling-host` (raw algorithm)
# or `nixling-priv-broker` (broker variant + audit record).

set -euo pipefail

HERE=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

CARGO=${CARGO:-cargo}
if ! command -v "$CARGO" >/dev/null 2>&1 && ! command -v rustup >/dev/null 2>&1; then
  echo "cgroup-delegation-oracle: cargo not on PATH; expected the static gate's rust shell" >&2
  exit 1
fi

run_cargo() {
  if command -v rustup >/dev/null 2>&1 && [ -n "${RUSTUP_TOOLCHAIN:-}" ]; then
    RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" rustup run "$RUSTUP_TOOLCHAIN" cargo "$@"
  else
    RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" "$CARGO" "$@"
  fi
}

# Required canaries (named cargo tests). Each must pass for this gate.
declare -a HOST_CANARIES=(
  cgroup::tests::refuses_uid_zero
  cgroup::tests::refuses_when_unified_hierarchy_missing
  cgroup::tests::refuses_when_controllers_missing
  cgroup::tests::refuses_kill_on_ancestor
  cgroup::tests::happy_path_delegation
  cgroup::tests::detects_internal_processes
  cgroup::tests::refuses_threaded_cgroups
  cgroup::tests::rejects_partition_root_writes
  cgroup::tests::vm_subtree_creates_chowned_per_vm_interior
  cgroup::tests::vm_role_leaf_creates_chowned_per_role_leaf_under_vm_interior
  cgroup::tests::cpuset_inheritance_fills_empty_files
)

declare -a BROKER_CANARIES=(
  ops::cgroup::tests::delegate_happy_path
  ops::cgroup::tests::delegate_refused_uid_zero
  ops::cgroup::tests::open_unknown_subject_audited
  ops::cgroup::tests::open_known_vm_leaf_after_delegation
  ops::cgroup::tests::cgroup_kill_ancestor_refused
)

LOG_DIR=${TMPDIR:-/tmp}/nixling-cgroup-oracle.$$
mkdir -p "$LOG_DIR"
cleanup() { rm -rf -- "$LOG_DIR"; }
trap cleanup EXIT INT TERM

HOST_LOG="$LOG_DIR/host.log"
BROKER_LOG="$LOG_DIR/broker.log"

printf '\n[cgroup-delegation-oracle] host canaries (nixling-host fake backend)\n'
set +e
(
  cd "$ROOT/packages"
  run_cargo test -p nixling-host --all-features --lib cgroup::
) >"$HOST_LOG" 2>&1
host_status=$?
set -e
if [ $host_status -ne 0 ]; then
  printf 'cgroup-delegation-oracle: host canary block failed (status %d)\n' "$host_status" >&2
  tail -80 "$HOST_LOG" >&2
  exit 1
fi

for canary in "${HOST_CANARIES[@]}"; do
  if ! grep -qE "^test ${canary} \\.\\.\\. ok" "$HOST_LOG"; then
    printf 'cgroup-delegation-oracle: missing pass marker for %s\n' "$canary" >&2
    tail -120 "$HOST_LOG" >&2
    exit 1
  fi
done
printf '  ok: %d host canaries passed\n' "${#HOST_CANARIES[@]}"

printf '\n[cgroup-delegation-oracle] broker canaries (ops::cgroup + RecordingAuditSink)\n'
set +e
(
  cd "$ROOT/packages/nixling-priv-broker"
  run_cargo test --all-features --lib ops::cgroup
) >"$BROKER_LOG" 2>&1
broker_status=$?
set -e
if [ $broker_status -ne 0 ]; then
  printf 'cgroup-delegation-oracle: broker canary block failed (status %d)\n' "$broker_status" >&2
  tail -80 "$BROKER_LOG" >&2
  exit 1
fi

for canary in "${BROKER_CANARIES[@]}"; do
  if ! grep -qE "^test ${canary} \\.\\.\\. ok" "$BROKER_LOG"; then
    printf 'cgroup-delegation-oracle: missing pass marker for %s\n' "$canary" >&2
    tail -120 "$BROKER_LOG" >&2
    exit 1
  fi
done
printf '  ok: %d broker canaries passed\n' "${#BROKER_CANARIES[@]}"

printf '\ncgroup-delegation-oracle: all cgroup canaries passed\n'
exit 0
