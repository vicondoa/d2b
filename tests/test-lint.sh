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
  find tests scripts harness/ubuntu -maxdepth 1 -name '*.sh' -type f 2>/dev/null | sort -u
)
if [ "${#sh_files[@]}" -eq 0 ]; then
  fail "shellcheck: no .sh files found"
  exit 1
fi
shellcheck --severity=warning -x "${sh_files[@]}"
ok "shellcheck (${#sh_files[@]} scripts)"

log "test-lint OK (duration: $((SECONDS - suite_started))s)"
