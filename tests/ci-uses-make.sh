#!/usr/bin/env bash
# tests/ci-uses-make.sh — W0 convergence guard for workflow entrypoints.
#
# New workflows must invoke a top-level make target. Existing workflows are
# temporarily allowlisted until the CI convergence wave repoints them.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
WORKFLOW_DIR="$ROOT/.github/workflows"

# TODO(test-rearch): remove these entries as workflows converge on make targets.
allowlisted_workflows=(
  .github/workflows/eval-with-entra-id.yml
  .github/workflows/pr-cargo-workspace.yml
  .github/workflows/pr-eval-shell-tests.yml
  .github/workflows/pr-l1c-privilege-oracle.yml
)

is_allowlisted() {
  local rel="$1" entry
  for entry in "${allowlisted_workflows[@]}"; do
    [ "$entry" = "$rel" ] && return 0
  done
  return 1
}

calls_make_target() {
  local workflow="$1"
  grep -Eq '(^|[[:space:]])make[[:space:]]+(check|check-ci|check-all|check-fast|check-tier0|test-rust|test-drift|test-fixtures|test-contract|test-nix-unit|test-flake|test-policy|test-mutation|test-integration|test-hardware|perf|check-inventory|ledger-regen)([[:space:]]|$)' "$workflow"
}

[ -d "$WORKFLOW_DIR" ] || { echo "ci-uses-make: missing workflow dir: $WORKFLOW_DIR" >&2; exit 1; }

shopt -s nullglob
workflows=("$WORKFLOW_DIR"/*.yml)
shopt -u nullglob

if [ "${#workflows[@]}" -eq 0 ]; then
  echo "ci-uses-make: no .github/workflows/*.yml files found" >&2
  exit 1
fi

fail=0
for workflow in "${workflows[@]}"; do
  rel=${workflow#"$ROOT/"}
  if calls_make_target "$workflow"; then
    printf 'PASS: %s calls a make target\n' "$rel"
  elif is_allowlisted "$rel"; then
    printf 'WARN: %s is allowlisted until CI make-target convergence\n' "$rel"
  else
    printf 'FAIL: %s neither calls a make target nor appears in the W0 allowlist\n' "$rel" >&2
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi
