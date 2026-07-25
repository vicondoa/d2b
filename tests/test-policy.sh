#!/usr/bin/env bash
# tests/test-policy.sh - `make test-policy`: repository policy / meta gates that
# guard the test architecture itself and other cross-cutting invariants.
#
#   * adr-index-coverage      - every docs/adr/*.md is indexed
#   * ci-coverage             - every tests/*.sh is wired into CI / an aggregator
#   * deliverable-gate-inventory - required gate scripts exist
#   * layer1-self-inventory   - Layer-1 driver scripts are accounted for
#   * no-new-deferral         - ADR 0022 I3 invariant (no new v1.3 deferrals)
#   * pr-checklist-gate       - PR template checklist is well-formed
#
# CI runs this as its own job; locally it is one prerequisite of `make test-unit`.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
D2B_LOG=${D2B_LOG:-/dev/null}
export ROOT D2B_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

# The fixture-independent contract-test policy binaries below need a cargo
# toolchain. Locally and in the `rust` CI job cargo is already present; in a
# bare nix environment, re-enter once through the repo-pinned nixpkgs toolchain
# so the gate still runs instead of silently skipping. Fail closed if neither
# cargo nor nix can provide it.
if ! command -v cargo >/dev/null 2>&1 && [ -z "${D2B_POLICY_GATE_IN_NIX_SHELL:-}" ]; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "neither cargo nor nix is on PATH; the policy cargo gates cannot run"
    exit 1
  fi
  toolchain_file="$ROOT/packages/rust-toolchain.toml"
  pinned_channel=$(
    sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]\+\)".*/\1/p' "$toolchain_file" | head -1
  )
  if [ -z "$pinned_channel" ]; then
    fail "could not read pinned Rust channel from $toolchain_file"
    exit 1
  fi
  policy_gate_scratch=$(d2b_mktemp .d2b-policy-gate.XXXXXX)
  add_cleanup "rm -rf -- \"$policy_gate_scratch\""
  log "  cargo not on PATH; re-entering via nix shell to acquire pinned Rust $pinned_channel toolchain"
  export D2B_POLICY_GATE_IN_NIX_SHELL=1
  export RUSTUP_HOME="$policy_gate_scratch/rustup"
  export CARGO_HOME="$policy_gate_scratch/cargo"
  nix shell --quiet --inputs-from "$ROOT" nixpkgs#rustup nixpkgs#stdenv.cc \
    --command bash -c "
      set -euo pipefail
      rustup toolchain install '$pinned_channel' --profile minimal >/dev/null
      rustup default '$pinned_channel' >/dev/null
      export PATH=\"\$CARGO_HOME/bin:\$PATH\"
      exec bash '$0' \"\$@\"
    " -- "$@"
  exit $?
fi

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

# Run one fixture-independent contract-test policy binary and fail closed unless
# it actually executed at least one test. This is the orchestration assertion:
# a binary that is skipped, filtered to nothing, or reports zero tests would
# make the policy gate a silent no-op, which is exactly the fail-open hole this
# target closes. D2B_SKIP_FIXTURE_BUILD is deliberately NOT honored here - these
# binaries read committed repository artifacts, not D2B_FIXTURES.
run_policy_cargo_binary() {
  local label="$1" testname="$2"
  local out status result_line passed failed
  log "--> $label"
  set +e
  out=$(
    cd "$ROOT/packages" \
      && CARGO_TERM_COLOR=never cargo test -p d2b-contract-tests --test "$testname" 2>&1
  )
  status=$?
  set -e
  printf '%s\n' "$out" | sed 's/^/    /' >&2
  if [ "$status" -ne 0 ]; then
    fail "$label (cargo exit $status)"
    rc=1
    return
  fi
  result_line=$(printf '%s\n' "$out" | grep -E '^test result:' | tail -1)
  if [ -z "$result_line" ]; then
    fail "$label produced no test-result line; the binary did not run"
    rc=1
    return
  fi
  passed=$(printf '%s\n' "$result_line" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' | head -1)
  failed=$(printf '%s\n' "$result_line" | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+' | head -1)
  if [ -z "$passed" ] || [ "$passed" -lt 1 ]; then
    fail "$label ran 0 tests (skipped or filtered); the policy gate would be a no-op"
    rc=1
    return
  fi
  if [ -n "$failed" ] && [ "$failed" -ne 0 ]; then
    fail "$label reported $failed failing test(s)"
    rc=1
    return
  fi
  ok "$label ($passed passed)"
}

run_policy_gate "adr-index-coverage"        tests/unit/meta/adr-index-coverage.sh
run_policy_gate "w0-dep-direction"          tests/unit/meta/w0-dep-direction.sh
run_policy_gate "deliverable-gate-inventory" tests/unit/meta/deliverable-gate-inventory.sh
run_policy_gate "layer1-self-inventory"     tests/unit/meta/layer1-self-inventory.sh
run_policy_gate "no-new-deferral"           tests/unit/meta/no-new-deferral.sh
run_policy_gate "pr-checklist-gate"         tests/unit/meta/pr-checklist-gate.sh .github/PULL_REQUEST_TEMPLATE.md

# ci-coverage must run LAST: it attests that every other test is wired into a
# workflow or aggregator, so it has to observe the final reference set.
run_policy_gate "ci-coverage"               tests/unit/meta/ci-coverage.sh

# Fixture-independent contract-test policy binaries. These prove the dash ban,
# the ADR 0046 manifest-drift bijection, the changelog gate, and the ADR 0046
# spec-literal drift lints (datetime precision, ResourceType grammar, retry
# scalar shape) actually work. They are excluded from `make test-rust`'s
# `cargo test --workspace` run and skipped there under D2B_SKIP_FIXTURE_BUILD,
# so this mandatory target is their only guaranteed execution point.
run_policy_cargo_binary "policy-dash-gate"        policy_dash_gate
run_policy_cargo_binary "policy-adr046-work-items" policy_adr046_work_items
run_policy_cargo_binary "policy-changelog-gate"   policy_changelog_gate
run_policy_cargo_binary "policy-adr046-spec-literals" policy_adr046_spec_literals
run_policy_cargo_binary "policy-adr046-envelopes"     policy_adr046_envelopes

[ "$rc" -eq 0 ] || exit 1
log "test-policy OK"
