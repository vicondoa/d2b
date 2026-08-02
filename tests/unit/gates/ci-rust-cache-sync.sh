#!/usr/bin/env bash
# tests/unit/gates/ci-rust-cache-sync.sh - fail-closed gate: the CI
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

# --- Build the set of compiler artifact dirs the Rust gate actually uses ---
# These paths MUST be cached for warm CI builds. The .scratch subtree contains
# only toolchain-keyed persistent trees owned by tests that invoke nested Cargo
# or rustdoc; omitting it restores the outer workspace but leaves its 12-minute
# critical path cold.
declared_dirs=(
  "packages -> target"
  "packages/d2b-priv-broker -> target"
  "packages/d2b-guest-shell-runner -> target"
  "tests/tools/no-bash-ast-walker/target"
  ".scratch/rust-test-cache"
)

if ! grep -qF 'fixture_target_dir="$ROOT/.scratch/rust-test-cache/fixture-contracts"' "$test_script"; then
  log "FAIL: fixture/CLI target does not use its isolated cached Cargo target"
  rc=1
fi
api_script="$ROOT/tests/tools/api-surface-json.sh"
for marker in \
  'public_target="$target_root/public-census"' \
  'private_target="$target_root/private-census"' \
  'checker_target="$target_root/checker"'; do
  if ! grep -qF "$marker" "$api_script"; then
    log "FAIL: API surface target marker '$marker' is missing"
    rc=1
  fi
done
if ! grep -qF 'CARGO_BUILD_JOBS="$public_jobs"' "$api_script" \
  || ! grep -qF 'CARGO_BUILD_JOBS="$private_jobs"' "$api_script"; then
  log "FAIL: parallel API rustdoc passes do not carry split Cargo quotas"
  rc=1
fi
if ! grep -qF 'cargo run --quiet --release --locked' "$api_script"; then
  log "FAIL: API snapshot checker is not using the measured release profile"
  rc=1
fi
# Broker parallel feature-pass target dirs: the script uses
# ${broker_target_dir%/}-<suffix> where broker_target_dir resolves to
# packages/d2b-priv-broker/target.
while IFS= read -r suffix; do
  declared_dirs+=("packages/d2b-priv-broker/target-${suffix}")
done < <(
  grep -oP '(?<=broker_target_dir%/\}-)[a-z0-9]+' "$test_script" | sort -u
)

# --- Extract cached dirs from CI workflow (simple grep) ---
# The workflow's Swatinem/rust-cache step declares paths in `workspaces:`
# (format: "path -> target") and `cache-directories:` (plain paths).
cached_in_ci=$(
  grep -E '^\s+(packages|packages/|tests/tools/no-bash-ast-walker/target|\.scratch/)' "$wf" \
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
