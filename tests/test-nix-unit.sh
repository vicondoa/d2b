#!/usr/bin/env bash
# tests/test-nix-unit.sh — `make test-nix-unit`: build the nix-unit corpus checks
# (`flake.checks.<system>.nix-unit*`) for the native system.
#
# This is a FOCUSED convenience target for iterating on the declarative
# value/throw corpus under tests/unit/nix/. It is NOT part of `make test-unit`,
# because the root `nix flake check` run by `make test-flake` already evaluates
# the nix-unit check — running both would double the work.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
NL_LOG=${NL_LOG:-/dev/null}
export ROOT NL_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

system=$(nix eval --raw --impure --expr builtins.currentSystem)
mapfile -t checks < <(
  nix eval --raw ".#checks.$system" --apply '
    cs:
      builtins.concatStringsSep "\n"
        (builtins.filter
          (name: name == "nix-unit" || builtins.substring 0 9 name == "nix-unit-")
          (builtins.sort builtins.lessThan (builtins.attrNames cs)))
  '
)

if [ "${#checks[@]}" -eq 0 ]; then
  fail "nix-unit corpus ($system): no nix-unit* checks found"
  exit 1
fi

for check in "${checks[@]}"; do
  log "--> nix build .#checks.$system.$check"
  if nix build --no-link --print-out-paths ".#checks.$system.$check"; then
    ok "nix-unit check $check ($system)"
  else
    fail "nix-unit check $check ($system)"
    exit 1
  fi
done

log "test-nix-unit OK"
