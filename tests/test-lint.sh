#!/usr/bin/env bash
# tests/test-lint.sh — `make test-lint`: fast static lint, no Nix eval, no cargo.
#
#   * preflight disk-space guard (fail closed before the Nix-heavy siblings)
#   * nix-instantiate --parse on every .nix file
#   * shellcheck --severity=warning on the nixling shell scripts
#
# CI runs this as its own job; locally it is one prerequisite of `make test-unit`.
# Driver script name matches the make target (tests/test-<target>.sh).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
NL_LOG=${NL_LOG:-/dev/null}
export ROOT NL_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

# --- preflight ------------------------------------------------------------
if [ -x "$ROOT/tests/tools/preflight-disk-space.sh" ]; then
  log "--> preflight-disk-space"
  bash "$ROOT/tests/tools/preflight-disk-space.sh"
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
log "--> shellcheck --severity=warning on all nixling shell scripts"
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
  find tests scripts harness/ubuntu -maxdepth 1 -name '*.sh' -type f 2>/dev/null | sort
)
if [ "${#sh_files[@]}" -eq 0 ]; then
  fail "shellcheck: no .sh files found"
  exit 1
fi
shellcheck --severity=warning -x "${sh_files[@]}"
ok "shellcheck (${#sh_files[@]} scripts)"

log "test-lint OK"
