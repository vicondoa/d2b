#!/usr/bin/env bash
# tests/test-lint.sh - `make test-lint`: fast static lint, no Nix eval, no cargo.
#
#   * preflight disk-space guard (fail closed before the Nix-heavy siblings)
#   * nix-instantiate --parse on every .nix file
#   * shellcheck --severity=warning on the d2b shell scripts
#
# CI runs this as its own job; locally it is one prerequisite of `make test-unit`.
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

log "test-lint OK (duration: $((SECONDS - suite_started))s)"
