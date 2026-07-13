#!/usr/bin/env bash
# tests/unit/gates/ci-rust-cache-sync.sh — fail-closed gate: pinned gate
# compiler artifacts must stay isolated from CI cache restoration. Run by
# `make test-drift`.
#
# Restoring a partially populated Cargo target can preserve fingerprints while
# omitting build-script executables, producing false "No such file" failures.
# rust-cache still caches the registry; the pinned gate rebuilds compiler
# metadata in its toolchain-scoped target on each CI run.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

wf="$ROOT/.github/workflows/pr-l1-static-fast.yml"
test_script="$ROOT/tests/test-rust.sh"

rc=0

if ! grep -q 'd2b_cargo_gate_target_dir' "$test_script"; then
  log "FAIL: test-rust.sh does not use the toolchain-scoped target helper"
  rc=1
fi

if grep -q 'packages/.d2b-gate-targets' "$wf"; then
  log "FAIL: toolchain-scoped Cargo gate targets must not be restored from CI cache"
  rc=1
fi

if [ "$rc" = 0 ]; then
  ok "ci-rust-cache-sync: pinned Cargo gate targets are isolated from CI cache"
else
  fail "ci-rust-cache-sync: pinned Cargo gate target isolation drifted"
fi

exit "$rc"
