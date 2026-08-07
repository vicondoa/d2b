#!/usr/bin/env bash
# tests/unit/gates/flake-check-matrix-sync.sh - fail-closed gate: the CI
# flake-check inventory and hosted-runner shard wiring must stay in sync with
# the flake. Run by `make test-drift`.
#
# Two invariants guard against the "CI matrix silently drifts" failure mode:
#
#   1. NAME PIN - the live `flake.checks.x86_64-linux.*` set must equal the
#      committed pin (tests/golden/flake-check-matrix/x86_64-linux.txt). A
#      new/removed check fails closed until `make flake-matrix-pin` is run, so a
#      reviewer confirms the check is covered deliberately. The pin is the full
#      static check set; the hosted-runner matrix may intentionally filter
#      checks that require a local/manual or alternate validation path.
#
#   2. WIRING - the workflow must still GENERATE the hosted matrix from
#      `make test-flake-list` and aggregate every hosted shard into the required
#      `test-flake-x86` context. This catches anyone hardcoding/forking the
#      matrix source or dropping the aggregator.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

rc=0
wf="$ROOT/.github/workflows/pr-l1-static-fast.yml"
release_wf="$ROOT/.github/workflows/release-host-binaries.yml"

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
    fail "wiring: $label - pattern not found in $(basename "$wf"): $pattern"
    rc=1
  fi
}

assert_release() {
  local label="$1" pattern="$2"
  if grep -Eq "$pattern" "$release_wf"; then
    ok "release: $label"
  else
    fail "release: $label - pattern not found in $(basename "$release_wf"): $pattern"
    rc=1
  fi
}

if [ ! -f "$wf" ]; then
  fail "missing workflow: $wf"
  exit 1
fi
if [ ! -f "$release_wf" ]; then
  fail "missing release workflow: $release_wf"
  exit 1
fi

# discover job sources the names from the live flake via make test-flake-partition
assert_wf "discover enumerates via make test-flake-partition" 'make -s test-flake-partition'
# both shard lanes consume the discovered JSON (not a hardcoded list)
assert_wf "eval matrix sourced from discover output" 'fromJSON\(needs\.flake-eval-discover\.outputs\.evalchecks\)'
assert_wf "realized matrix sourced from discover output" 'fromJSON\(needs\.flake-eval-discover\.outputs\.realizedchecks\)'
# Nix-unit CI retains the pre-change per-check matrix because the full local
# eval-jobs runner does not fit the hosted runner memory envelope.
assert_wf "nix-unit discovery exports the partition" 'checks:\s*\$\{\{\s*steps\.list\.outputs\.nixunitchecks'
assert_wf "nix-unit matrix sourced from discovery" 'fromJSON\(needs\.nix-unit-discover\.outputs\.checks\)'
assert_wf "nix-unit shard invokes make test-nix-unit" 'D2B_NIX_UNIT_CHECK'
if grep -Eq 'D2B_NIX_UNIT_JOBS' "$wf"; then
  fail "wiring: retired D2B_NIX_UNIT_JOBS remains in CI"
  rc=1
else
  ok "wiring: retired D2B_NIX_UNIT_JOBS is absent"
fi
# each shard runs the make-routed single-check evaluation
assert_wf "shard runs D2B_FLAKE_CHECK make test-flake" 'D2B_FLAKE_CHECK'
# the required-context aggregator gates on both shard matrices
assert_wf "aggregator needs the shard matrix" 'needs:\s*\[flake-eval-discover,\s*flake-eval-x86,\s*flake-eval-x86-realized'
assert_wf "aggregator fails on a red realized lane" '\[ "\$realized" = success \]'
# non-checks x86 outputs are still evaluated (packages, etc.)
assert_wf "x86 non-checks outputs are evaluated" 'D2B_FLAKE_OUTPUTS'
# The release workflow must use the unified product workspace and never the
# retired broker manifest. Keep these checks here because this gate already
# guards the flake/workflow boundary and is fail-closed in test-drift.
assert_release "release builds use the root Cargo manifest" \
  'cargo build --release --locked --manifest-path packages/Cargo\.toml'
assert_release "release copies the broker from the root target" \
  'packages/target/release/d2b-priv-broker'
if grep -qF 'packages/d2b-priv-broker/Cargo.toml' "$release_wf"; then
  fail "release: retired broker manifest path remains"
  rc=1
else
  ok "release: retired broker manifest path is absent"
fi

# Native aarch64 realizes the six package/artifact checks and then runs the
# native supply-chain gate. The checkout and verification are bound to the
# same PR-head ref, so one renderer-covered stable head owns both results.

aarch64_block=$(awk '
  /^  test-flake-aarch64:/ { in_block = 1 }
  in_block { print }
  in_block && /^  test-drift:/ { exit }
' "$wf")
if grep -q 'smoke-eval-aarch64\.nix' <<<"$aarch64_block"; then
  fail "wiring: aarch64 smoke evaluation remains instead of native realization"
  rc=1
else
  ok "wiring: aarch64 smoke evaluation is absent"
fi
if grep -qF 'runs-on: ubuntu-24.04-arm' <<<"$aarch64_block"; then
  ok "wiring: aarch64 job uses the native arm runner"
else
  fail "wiring: aarch64 job must use ubuntu-24.04-arm"
  rc=1
fi
if grep -qF 'timeout-minutes: 60' <<<"$aarch64_block"; then
  ok "wiring: aarch64 job has a 60-minute bound"
else
  fail "wiring: aarch64 job must have a 60-minute bound"
  rc=1
fi
if grep -qF 'make test-rust-supply-chain' <<<"$aarch64_block"; then
  ok "wiring: aarch64 job runs make test-rust-supply-chain"
else
  fail "wiring: aarch64 job must run make test-rust-supply-chain"
  rc=1
fi
if grep -qF 'nix build --no-link' <<<"$aarch64_block"; then
  ok "wiring: aarch64 job realizes checks with nix build"
else
  fail "wiring: aarch64 job must realize checks with nix build --no-link"
  rc=1
fi
for check in \
  broker-production-dependency-policy \
  guest-shell-runner-static-dependency-policy \
  broker-production-package-policy \
  guest-real-libshpool-package-policy \
  broker-host-artifact-contract \
  guest-static-elf
do
  if grep -qF ".#checks.aarch64-linux.$check" <<<"$aarch64_block"; then
    ok "wiring: aarch64 realizes $check"
  else
    fail "wiring: aarch64 six-check realization is missing $check"
    rc=1
  fi
done
for marker in \
  'github.event.pull_request.head.sha' \
  'git rev-parse HEAD' \
  'git status --porcelain'
do
  if grep -qF "$marker" <<<"$aarch64_block"; then
    ok "wiring: aarch64 stable-head binding includes $marker"
  else
    fail "wiring: aarch64 stable-head binding is missing $marker"
    rc=1
  fi
done
if grep -Eq -- '(^|[[:space:]])--system(=|[[:space:]])|(^|[[:space:]])--builders?(=|[[:space:]])|ssh://|remote-builder' <<<"$aarch64_block"; then
  fail "wiring: aarch64 job supplies a foreign system or remote builder"
  rc=1
else
  ok "wiring: aarch64 job refuses foreign systems and remote builders"
fi

if [ "$rc" -eq 0 ]; then
  log "flake-check-matrix-sync OK"
fi
exit "$rc"
