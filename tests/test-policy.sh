#!/usr/bin/env bash
# tests/test-policy.sh — `make test-policy`: repository policy / meta gates that
# guard the test architecture itself and other cross-cutting invariants.
#
#   * adr-index-coverage      — every docs/adr/*.md is indexed
#   * ci-coverage             — every tests/*.sh is wired into CI / an aggregator
#   * ci-uses-make            — every workflow invokes a make target
#   * deliverable-gate-inventory — required gate scripts exist
#   * layer1-self-inventory   — Layer-1 driver scripts are accounted for
#   * no-new-deferral         — ADR 0022 I3 invariant (no new v1.3 deferrals)
#   * pr-checklist-gate       — PR template checklist is well-formed
#
# CI runs this as its own job; locally it is one prerequisite of `make test-unit`.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
NL_LOG=${NL_LOG:-/dev/null}
export ROOT NL_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

rc=0
run_policy_gate() {
  local label="$1" script="$2"
  shift 2
  if [ -f "$ROOT/$script" ]; then
    log "--> $label"
    if bash "$ROOT/$script" "$@"; then
      ok "$label"
    else
      fail "$label"
      rc=1
    fi
  else
    log "  SKIP: $label ($script not present)"
  fi
}

run_policy_gate "adr-index-coverage"        tests/unit/meta/adr-index-coverage.sh
run_policy_gate "ci-uses-make"              tests/unit/meta/ci-uses-make.sh
run_policy_gate "deliverable-gate-inventory" tests/unit/meta/deliverable-gate-inventory.sh
run_policy_gate "layer1-self-inventory"     tests/unit/meta/layer1-self-inventory.sh
run_policy_gate "no-new-deferral"           tests/unit/meta/no-new-deferral.sh
run_policy_gate "pr-checklist-gate"         tests/unit/meta/pr-checklist-gate.sh .github/PULL_REQUEST_TEMPLATE.md

# ci-coverage must run LAST: it attests that every other test is wired into a
# workflow or aggregator, so it has to observe the final reference set.
run_policy_gate "ci-coverage"               tests/unit/meta/ci-coverage.sh

[ "$rc" -eq 0 ] || exit 1
log "test-policy OK"
