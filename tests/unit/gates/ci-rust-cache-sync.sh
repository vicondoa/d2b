#!/usr/bin/env bash
# tests/unit/gates/ci-rust-cache-sync.sh — fail-closed gate: the CI
# rust-cache directory list must cover every CARGO_TARGET_DIR used by
# tests/test-rust.sh. Run by `make test-drift`.
#
# If test-rust.sh adds a new target dir (e.g. a new broker feature
# pass), this gate catches the missing CI cache entry so warm builds
# don't silently degrade to cold.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

wf="$ROOT/.github/workflows/pr-l1-static-fast.yml"
test_script="$ROOT/tests/test-rust.sh"

rc=0

# The pinned gate places every workspace and standalone feature pass below one
# toolchain-scoped root. Cache that root rather than enumerating a channel or
# each scope, so a toolchain bump does not require a workflow path rewrite.
declared_dirs=("packages/.d2b-gate-targets")
if ! grep -q 'd2b_cargo_gate_target_dir' "$test_script"; then
  log "FAIL: test-rust.sh does not use the toolchain-scoped target helper"
  rc=1
fi

# --- Extract cached dirs from CI workflow (simple grep) ---
# The workflow's Swatinem/rust-cache step declares paths in `workspaces:`
# (format: "path -> target") and `cache-directories:` (plain paths).
cached_in_ci=$(
  grep -E '^\s+(packages|packages/)' "$wf" \
    | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//' \
    | sort -u
)

# --- Check that every declared dir is cached ---
for dir in "${declared_dirs[@]}"; do
  if ! echo "$cached_in_ci" | grep -qxF "$dir"; then
    log "FAIL: target dir '$dir' used by test-rust.sh is NOT in CI rust-cache config"
    rc=1
  fi
done

if [ "$rc" = 0 ]; then
  ok "ci-rust-cache-sync: all test-rust.sh target dirs are cached in CI"
else
  fail "ci-rust-cache-sync: one or more target dirs missing from .github/workflows/pr-l1-static-fast.yml rust-cache config"
  log "  Fix: add the missing paths to the Swatinem/rust-cache step's workspaces/cache-directories"
fi

exit "$rc"
