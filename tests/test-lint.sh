#!/usr/bin/env bash
# tests/test-lint.sh - `make test-lint`: fail-fast lint before long Layer-1 jobs.
#
#   * preflight disk-space guard (fail closed before the Nix-heavy siblings)
#   * compiler-derived API input fingerprint drift
#   * Rust formatting across every gated workspace
#   * changed-scope clippy for the main and guest-shell-runner workspaces
#   * nix-instantiate --parse on every .nix file
#   * shellcheck --severity=warning on the d2b shell scripts
#
# CI leaves clippy to the required full Rust shard because its fresh runner has
# no shared target with this job. Local `make check` runs this phase serially
# before dispatching the long parallel jobs.
# Driver script name matches the make target (tests/test-<target>.sh).

set -euo pipefail
suite_started=$SECONDS

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
D2B_LOG=${D2B_LOG:-/dev/null}
export ROOT D2B_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

# --- preflight ------------------------------------------------------------
if [ -f "$ROOT/tests/tools/preflight-disk-space.sh" ]; then
  log "--> preflight-disk-space"
  bash "$ROOT/tests/tools/preflight-disk-space.sh"
else
  fail "required preflight gate is missing: tests/tools/preflight-disk-space.sh"
  exit 1
fi

# --- change scope ---------------------------------------------------------
resolve_lint_base() {
  local base_ref
  if [ -n "${D2B_LINT_BASE:-}" ]; then
    git rev-parse --verify "${D2B_LINT_BASE}^{commit}" >/dev/null 2>&1 || {
      fail "D2B_LINT_BASE does not name a commit"
      return 1
    }
    git rev-parse "${D2B_LINT_BASE}^{commit}"
    return
  fi

  base_ref=${GITHUB_BASE_REF:-v3}
  if git rev-parse --verify --quiet "origin/$base_ref" >/dev/null; then
    git merge-base HEAD "origin/$base_ref"
  elif git rev-parse --verify --quiet HEAD^ >/dev/null; then
    echo "WARN: origin/$base_ref not found; using HEAD^ for fast lint scope" >&2
    git rev-parse HEAD^
  else
    git rev-parse HEAD
  fi
}

lint_base=$(resolve_lint_base)
export D2B_LINT_BASE="$lint_base"

# --- compiler-derived API input fingerprint -------------------------------
# The authoritative census still runs in test-rust-api-surface. This generated
# fingerprint proves that make api-surface-pin ran for the exact workspace
# source state before the expensive census starts.
log "--> compiler-derived API pin precheck"
bash "$ROOT/tests/tools/api-surface-input-fingerprint.sh" --check || {
  fail "compiler-derived API pin is stale"
  exit 1
}
ok "compiler-derived API pin precheck"

# --- Rust format + changed-scope clippy ----------------------------------
log "--> Rust format + changed-scope clippy"
env -u D2B_EXECUTION_MANIFEST \
  D2B_LINT_BASE="$lint_base" \
  D2B_RUST_CARGO_JOBS="${D2B_LINT_RUST_JOBS:-2}" \
  bash "$ROOT/tests/test-rust.sh" fast-lint
ok "Rust format + changed-scope clippy"

# --- nix-instantiate --parse ---------------------------------------------
log "--> nix-instantiate --parse on all .nix files"
parsed=0
while IFS= read -r f; do
  [ -n "$f" ] || continue
  if ! nix-instantiate --parse "$f" >/dev/null 2>&1; then
    echo "PARSE FAIL: $f" >&2
    fail "nix-instantiate --parse ($f)"
    exit 1
  fi
  parsed=$((parsed + 1))
done < <(find nixos-modules tests -name '*.nix' -type f; printf '%s\n' flake.nix)
ok "nix-instantiate --parse ($parsed files)"

# --- shellcheck -----------------------------------------------------------
log "--> shellcheck --severity=warning on all d2b shell scripts"
if ! command -v shellcheck >/dev/null 2>&1; then
  if command -v nix >/dev/null 2>&1; then
    sc_path=$(nix shell --quiet --inputs-from "$ROOT" nixpkgs#shellcheck \
      --command bash -lc 'printf %s "$PATH"')
    PATH="$sc_path:$PATH"
    export PATH
  else
    fail "shellcheck not found and nix unavailable"
    exit 1
  fi
fi
mapfile -t sh_files < <(
  {
    find tests scripts harness/ubuntu -maxdepth 1 -name '*.sh' -type f 2>/dev/null
    # The Copilot agent surface keeps its scripts one level deeper, under
    # scripts/copilot/ and inside each skill directory, so the maxdepth-1
    # sweep above does not reach them.
    find scripts/copilot .github/skills -name '*.sh' -type f 2>/dev/null
  } | sort -u
)
if [ "${#sh_files[@]}" -eq 0 ]; then
  fail "shellcheck: no .sh files found"
  exit 1
fi
shellcheck --severity=warning -x "${sh_files[@]}"
ok "shellcheck (${#sh_files[@]} scripts)"

# --- copilot agent bindings ----------------------------------------------
# Reads committed files only. A panel lane dispatched without an explicit
# reasoning effort silently runs at the model default while its record would
# attest the policy level, so a mispinned table is a false attestation rather
# than an error. This is the cheapest place to catch it.
log "--> copilot agent binding tables"
if [ -f "$ROOT/scripts/copilot/check-bindings.mjs" ]; then
  if command -v node >/dev/null 2>&1; then
    node "$ROOT/scripts/copilot/check-bindings.mjs"
    ok "copilot agent binding tables"
  else
    fail "node not found; scripts/copilot/check-bindings.mjs cannot run"
    exit 1
  fi
else
  fail "required gate is missing: scripts/copilot/check-bindings.mjs"
  exit 1
fi

# --- panel lifecycle behavior ---------------------------------------------
# Selection, one comprehensive discovery, ledger responses, scoped
# verification, and legacy continuation are covered by one focused Node
# harness. It is intentionally separate from the delivery crate tests.
log "--> panel lifecycle behavior"
if [ -f "$ROOT/scripts/copilot/test-panel-lifecycle.mjs" ]; then
  if command -v node >/dev/null 2>&1; then
    node "$ROOT/scripts/copilot/test-panel-lifecycle.mjs" >/dev/null
    ok "panel lifecycle behavior"
  else
    fail "node not found; scripts/copilot/test-panel-lifecycle.mjs cannot run"
    exit 1
  fi
else
  fail "required gate is missing: scripts/copilot/test-panel-lifecycle.mjs"
  exit 1
fi

# --- panel record assembly ------------------------------------------------
# make-records.mjs produces the artifacts that seal a wave. Its fail-closed
# behaviour is the only thing standing between a lane that silently ran at
# the wrong effort and a record attesting the policy level, so it gets real
# coverage rather than being exercised only when a panel round happens to run.
log "--> panel record assembly"
if [ -f "$ROOT/scripts/copilot/test-make-records.mjs" ]; then
  if command -v node >/dev/null 2>&1; then
    node "$ROOT/scripts/copilot/test-make-records.mjs" >/dev/null
    ok "panel record assembly"
  else
    fail "node not found; scripts/copilot/test-make-records.mjs cannot run"
    exit 1
  fi
else
  fail "required gate is missing: scripts/copilot/test-make-records.mjs"
  exit 1
fi

# --- panel review request -------------------------------------------------
# Reviewers have no shell and receive one generated request rather than a
# hand-written summary. Exercise that request so incremental ranges, full
# context, prior verdicts, evidence, and no-rerun instructions cannot silently
# drop out of the integrator handoff.
log "--> panel review request"
if [ -f "$ROOT/scripts/copilot/test-stage-diffs.mjs" ]; then
  if command -v node >/dev/null 2>&1; then
    node "$ROOT/scripts/copilot/test-stage-diffs.mjs" >/dev/null
    ok "panel review request"
  else
    fail "node not found; scripts/copilot/test-stage-diffs.mjs cannot run"
    exit 1
  fi
else
  fail "required gate is missing: scripts/copilot/test-stage-diffs.mjs"
  exit 1
fi

# --- binding gate self-coverage -------------------------------------------
# The seat-roster comparison inside check-bindings.mjs is enforced by parsing
# source with a regex, and a regex guard can stop matching without anything
# else changing. A guard that no longer matches fails open in silence, so it
# gets a negative test rather than being trusted because it once worked.
log "--> copilot binding gate self-coverage"
if [ -f "$ROOT/scripts/copilot/test-check-bindings.mjs" ]; then
  if command -v node >/dev/null 2>&1; then
    node "$ROOT/scripts/copilot/test-check-bindings.mjs" >/dev/null
    ok "copilot binding gate self-coverage"
  else
    fail "node not found; scripts/copilot/test-check-bindings.mjs cannot run"
    exit 1
  fi
else
  fail "required gate is missing: scripts/copilot/test-check-bindings.mjs"
  exit 1
fi

log "test-lint OK (duration: $((SECONDS - suite_started))s)"
