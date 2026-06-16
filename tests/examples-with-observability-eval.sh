#!/usr/bin/env bash
# tests/examples-with-observability-eval.sh — partial migration gate.
#
# PARTIAL migration. This shell gate is retained only for the realized
# `nix flake check` of `examples/with-observability`, which has no pure-eval
# nix-unit successor. The retired source assertions now live in
# `packages/nixling-contract-tests/tests/policy_examples_observability.rs`;
# the resolved-config value assertions now live in
# `tests/nix-unit/cases/examples-with-observability.nix`.
#
# Skips cleanly if `nix` is unavailable. Fails closed on any flake-check error.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
EXAMPLE_DIR="$ROOT/examples/with-observability"

export NL_LOG=${NL_LOG:-$ROOT/.examples-with-observability-eval.log}
export TMPDIR=${TMPDIR:-$ROOT/.copilot-work}
mkdir -p "$TMPDIR"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/examples-with-observability-eval.sh"

PASS=0
FAIL=0

ok()   { log "  PASS: $*"; PASS=$((PASS + 1)); }
bad()  { log "  FAIL: $*"; FAIL=$((FAIL + 1)); }

if ! command -v nix >/dev/null 2>&1; then
  log "  SKIP: nix not on PATH — skipping example flake check"
  log "==> summary: PASS=$PASS FAIL=$FAIL SKIP=1"
  exit 0
fi

scratch=$(nl_mktemp .with-observability-flake-check.XXXXXX)
flake_check_log="$scratch/flake-check.log"
if (cd "$EXAMPLE_DIR" && nix flake check --no-build --all-systems --no-write-lock-file) \
    >"$flake_check_log" 2>&1; then
  ok "nix flake check (examples/with-observability)"
else
  bad "nix flake check (examples/with-observability)"
  tail -40 "$flake_check_log" | sed 's/^/    /' >&2 || true
fi

log "==> summary: PASS=$PASS FAIL=$FAIL"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
