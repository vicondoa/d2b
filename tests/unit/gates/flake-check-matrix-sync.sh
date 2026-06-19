#!/usr/bin/env bash
# tests/unit/gates/flake-check-matrix-sync.sh — fail-closed gate: the CI
# flake-check shard matrix must stay in sync with the flake. Run by
# `make test-drift`.
#
# Two invariants guard against the "CI matrix silently drifts" failure mode:
#
#   1. NAME PIN — the live `flake.checks.x86_64-linux.*` set must equal the
#      committed pin (tests/golden/flake-check-matrix/x86_64-linux.txt). A
#      new/removed check fails closed until `make flake-matrix-pin` is run, so a
#      reviewer confirms the sharded coverage changed deliberately.
#
#   2. WIRING — the workflow must still GENERATE the matrix from the live flake
#      (via `make test-flake-list`) and aggregate every shard into the required
#      `test-flake-x86` context. This catches anyone hardcoding/forking the
#      matrix source or dropping the aggregator, which would let coverage drift
#      even while the name pin still matched.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

rc=0
wf="$ROOT/.github/workflows/pr-l1-static-fast.yml"

# 1. Name pin: live flake checks == committed pin.
if bash "$ROOT/tests/tools/gen-flake-check-matrix-pin.sh" --check; then
  ok "flake-check-matrix name pin in sync"
else
  fail "flake-check-matrix name pin drifted (run: make flake-matrix-pin)"
  rc=1
fi

# 2. Wiring: the matrix is generated from the live flake and fully aggregated.
assert_wf() {
  local label="$1" pattern="$2"
  if grep -Eq "$pattern" "$wf"; then
    ok "wiring: $label"
  else
    fail "wiring: $label — pattern not found in $(basename "$wf"): $pattern"
    rc=1
  fi
}

if [ ! -f "$wf" ]; then
  fail "missing workflow: $wf"
  exit 1
fi

# discover job sources the names from the live flake via make test-flake-list
assert_wf "discover enumerates via make test-flake-list" 'make -s test-flake-list'
# matrix consumes the discovered JSON (not a hardcoded list)
assert_wf "matrix sourced from discover output" 'fromJSON\(needs\.flake-eval-discover\.outputs\.checks\)'
# each shard runs the make-routed single-check evaluation
assert_wf "shard runs NL_FLAKE_CHECK make test-flake" 'NL_FLAKE_CHECK'
# the required-context aggregator gates on the full shard matrix result
assert_wf "aggregator needs the shard matrix" 'needs:\s*\[flake-eval-discover,\s*flake-eval-x86'
# non-checks x86 outputs are still evaluated (packages, etc.)
assert_wf "x86 non-checks outputs are evaluated" 'NL_FLAKE_OUTPUTS'
# aarch64 stays a lightweight smoke eval, not a full monolithic flake check.
assert_wf "aarch64 job uses smoke eval" 'smoke-eval-aarch64\.nix'

aarch64_block=$(awk '
  /^  test-flake-aarch64:/ { in_block = 1 }
  in_block { print }
  in_block && /^  test-drift:/ { exit }
' "$wf")
if grep -q 'make test-flake' <<<"$aarch64_block"; then
  fail "wiring: aarch64 job must not run make test-flake"
  rc=1
else
  ok "wiring: aarch64 job no longer runs make test-flake"
fi

if [ "$rc" -eq 0 ]; then
  log "flake-check-matrix-sync OK"
fi
exit "$rc"
