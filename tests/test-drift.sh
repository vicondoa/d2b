#!/usr/bin/env bash
# tests/test-drift.sh — `make test-drift`: generated-artifact + rendered-vs-doc
# drift gates. Fail closed when a committed generated file is stale.
#
#   * tests/unit/gates/drift-check.sh  — consolidated xtask gen-* drift
#                                        (error-codes, daemon-api, schemas, …)
#   * tests/unit/gates/vms-json-parity.sh — rendered vms.json vs manifest parity
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
for gate in \
  tests/unit/gates/drift-check.sh \
  tests/unit/gates/vms-json-parity.sh \
  tests/unit/gates/flake-check-matrix-sync.sh; do
  if [ -x "$ROOT/$gate" ]; then
    log "--> $gate"
    if bash "$ROOT/$gate"; then
      ok "$gate"
    else
      fail "$gate"
      rc=1
    fi
  else
    log "  SKIP: $gate (not present)"
  fi
done

[ "$rc" -eq 0 ] || exit 1
log "test-drift OK"
